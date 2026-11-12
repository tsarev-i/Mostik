//! What the HTTP client and the Xiva socket share on the way to the network: one TLS setup
//! that checks certificates the way the system does, and the system proxy.
//!
//! reqwest gets the TLS setup ready-made and finds the proxy itself; the socket is opened here,
//! through the same proxy (an HTTP proxy, with `CONNECT`) and with the same TLS setup. So behind
//! a corporate proxy that re-signs TLS with its own root, the live updates work wherever the
//! requests do.

use std::sync::{Arc, OnceLock};

use hyper_util::client::proxy::matcher::{Intercept, Matcher};
use rustls::ClientConfig;
use rustls::pki_types::ServerName;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;
use tokio_tungstenite::tungstenite::http::Uri;

#[derive(Debug, thiserror::Error)]
pub enum NetError {
    #[error("TLS: {0}")]
    Tls(#[from] rustls::Error),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("proxy {proxy}: {reason}")]
    Proxy { proxy: String, reason: String },
    #[error("bad host name: {0}")]
    BadHost(String),
}

/// The TLS setup of every connection: rustls with aws-lc-rs (the provider reqwest has) and the
/// platform verifier, which trusts what the system trusts, a corporate root included. Made once.
fn base_config() -> Result<Arc<ClientConfig>, NetError> {
    static CONFIG: OnceLock<Result<Arc<ClientConfig>, rustls::Error>> = OnceLock::new();
    let config = CONFIG.get_or_init(|| {
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let verifier = rustls_platform_verifier::Verifier::new(provider.clone())?;
        let config = ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()?
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(verifier))
            .with_no_client_auth();
        Ok(Arc::new(config))
    });
    Ok(config.clone()?)
}

/// The TLS setup for reqwest: the common one offering HTTP/2 and HTTP/1.1, which reqwest
/// leaves to a ready-made setup. (The socket offers no protocol: WebSocket needs HTTP/1.1.)
pub fn http_tls() -> Result<ClientConfig, NetError> {
    let mut config = (*base_config()?).clone();
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok(config)
}

/// The proxy for `https://<host>`, found the way reqwest finds it: `HTTPS_PROXY`, `ALL_PROXY`
/// and `NO_PROXY`, else the system settings (the Windows registry, macOS System Configuration).
/// Read anew on every connection, like reqwest does for every new client.
fn system_proxy(host: &str) -> Option<Intercept> {
    let uri: Uri = format!("https://{host}/").parse().ok()?;
    Matcher::from_system().intercept(&uri)
}

/// A TLS connection to `host:port`, through the system proxy when there is one.
pub async fn connect_tls(host: &str, port: u16) -> Result<TlsStream<TcpStream>, NetError> {
    let tcp = match system_proxy(host) {
        Some(proxy) => tunnel(&proxy, host, port).await?,
        None => TcpStream::connect((host, port)).await?,
    };
    let name = ServerName::try_from(host.to_owned()).map_err(|_| NetError::BadHost(host.to_owned()))?;
    Ok(TlsConnector::from(base_config()?).connect(name, tcp).await?)
}

/// The longest proxy answer to `CONNECT` accepted.
const MAX_PROXY_HEAD: usize = 8 * 1024;

/// A connection to `host:port` through an HTTP proxy: `CONNECT`, then the bytes pass as they are.
/// SOCKS and HTTPS proxies are refused with an error that says so.
async fn tunnel(proxy: &Intercept, host: &str, port: u16) -> Result<TcpStream, NetError> {
    let uri = proxy.uri();
    let fail = |reason: String| NetError::Proxy {
        proxy: uri.to_string(),
        reason,
    };
    if uri.scheme_str() != Some("http") {
        return Err(fail(format!(
            "{} proxies are not supported, only HTTP ones",
            uri.scheme_str().unwrap_or("this kind of")
        )));
    }
    let proxy_host = uri
        .host()
        .unwrap_or_default()
        .trim_start_matches('[')
        .trim_end_matches(']');
    let mut stream = TcpStream::connect((proxy_host, uri.port_u16().unwrap_or(80)))
        .await
        .map_err(|e| fail(e.to_string()))?;
    let mut request = format!("CONNECT {host}:{port} HTTP/1.1\r\nHost: {host}:{port}\r\n");
    if let Some(auth) = proxy.basic_auth().and_then(|a| a.to_str().ok()) {
        request.push_str(&format!("Proxy-Authorization: {auth}\r\n"));
    }
    request.push_str("\r\n");
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(|e| fail(e.to_string()))?;
    // Byte by byte: the TLS handshake that follows must find the stream right after the head.
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() >= MAX_PROXY_HEAD {
            return Err(fail("the answer to CONNECT is too long".into()));
        }
        let byte = stream.read_u8().await.map_err(|e| fail(e.to_string()))?;
        head.push(byte);
    }
    match connect_status(&head) {
        Some(200..=299) => Ok(stream),
        Some(407) => Err(fail("the proxy asks for a password (HTTP 407)".into())),
        Some(status) => Err(fail(format!("CONNECT refused with HTTP {status}"))),
        None => Err(fail("the answer to CONNECT is not HTTP".into())),
    }
}

/// The status code of a proxy's answer to `CONNECT`: `HTTP/1.1 200 Connection established`.
fn connect_status(head: &[u8]) -> Option<u16> {
    let line = head.split(|&b| b == b'\n').next()?;
    let mut words = std::str::from_utf8(line).ok()?.split_whitespace();
    words.next().filter(|v| v.starts_with("HTTP/"))?;
    words.next()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    fn proxy(url: &str) -> Intercept {
        Matcher::builder()
            .https(url)
            .build()
            .intercept(&Uri::from_static("https://ws.example.ru/"))
            .unwrap()
    }

    #[test]
    fn reads_the_connect_status() {
        assert_eq!(
            connect_status(b"HTTP/1.1 200 Connection established\r\n\r\n"),
            Some(200)
        );
        assert_eq!(
            connect_status(b"HTTP/1.0 407 Proxy Authentication Required\r\n\r\n"),
            Some(407)
        );
        assert_eq!(connect_status(b"SSH-2.0-OpenSSH\r\n\r\n"), None);
    }

    #[test]
    fn the_tls_setup_offers_http2_to_reqwest_only() {
        assert_eq!(
            http_tls().unwrap().alpn_protocols,
            [b"h2".to_vec(), b"http/1.1".to_vec()]
        );
        assert!(base_config().unwrap().alpn_protocols.is_empty());
    }

    #[tokio::test]
    async fn tunnels_through_an_http_proxy() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut head = Vec::new();
            while !head.ends_with(b"\r\n\r\n") {
                head.push(s.read_u8().await.unwrap());
            }
            s.write_all(b"HTTP/1.1 200 Connection established\r\n\r\nhello")
                .await
                .unwrap();
            String::from_utf8(head).unwrap()
        });
        let mut stream = tunnel(&proxy(&format!("http://anna:secret@{addr}")), "ws.example.ru", 443)
            .await
            .unwrap();
        let head = server.await.unwrap();
        assert!(head.starts_with("CONNECT ws.example.ru:443 HTTP/1.1\r\nHost: ws.example.ru:443\r\n"));
        // base64("anna:secret")
        assert!(head.contains("Proxy-Authorization: Basic YW5uYTpzZWNyZXQ=\r\n"));
        let mut rest = [0; 5];
        stream.read_exact(&mut rest).await.unwrap();
        assert_eq!(&rest, b"hello");
    }

    #[tokio::test]
    async fn a_refusing_proxy_or_a_socks_one_is_a_clear_error() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut buf = [0; 512];
            let _ = s.read(&mut buf).await;
            let _ = s.write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\n\r\n").await;
        });
        let err = tunnel(&proxy(&format!("http://{addr}")), "ws.example.ru", 443)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("password"), "{err}");
        let err = tunnel(&proxy("socks5://127.0.0.1:1"), "ws.example.ru", 443)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("socks5 proxies are not supported"), "{err}");
    }
}
