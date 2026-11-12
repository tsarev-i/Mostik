//! Content-free diagnostics: response structure (keys and types) and identifier patterns.
//! Used to check protocol parsing without storing any conversation.

use serde_json::{Map, Value, json};

/// JSON structure: objects become keys with types, arrays become a length and the merged item structure.
pub fn shape(v: &Value) -> Value {
    shape_depth(v, 0)
}

fn shape_depth(v: &Value, depth: usize) -> Value {
    if depth > 8 {
        return json!("…");
    }
    match v {
        Value::Null => json!("null"),
        Value::Bool(_) => json!("bool"),
        Value::Number(_) => json!("number"),
        Value::String(_) => json!("string"),
        Value::Array(items) => {
            let mut merged = Value::Null;
            for item in items.iter().take(50) {
                merged = merge(merged, shape_depth(item, depth + 1));
            }
            json!({ "len": items.len(), "item": merged })
        }
        Value::Object(map) => {
            let mut out = Map::new();
            for (k, val) in map {
                out.insert(k.clone(), shape_depth(val, depth + 1));
            }
            Value::Object(out)
        }
    }
}

fn merge(a: Value, b: Value) -> Value {
    match (a, b) {
        (Value::Null, b) => b,
        (Value::Object(mut a), Value::Object(b)) => {
            for (k, v) in b {
                let merged = merge(a.remove(&k).unwrap_or(Value::Null), v);
                a.insert(k, merged);
            }
            Value::Object(a)
        }
        (a, _) => a,
    }
}

/// Identifier pattern: alphanumeric runs are replaced by their class,
/// for example `0/0/<hex36>` or `<hex36>_<hex36>`.
pub fn id_pattern(id: &str) -> String {
    let mut out = String::new();
    let mut run = String::new();
    let flush = |run: &mut String, out: &mut String| {
        if run.is_empty() {
            return;
        }
        if run.len() <= 2 && run.chars().all(|c| c.is_ascii_digit()) {
            out.push_str(run);
        } else if run.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
            out.push_str(&format!("<hex{}>", run.len()));
        } else {
            out.push_str(&format!("<str{}>", run.len()));
        }
        run.clear();
    };
    for ch in id.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' {
            run.push(ch);
        } else {
            flush(&mut run, &mut out);
            out.push(ch);
        }
    }
    flush(&mut run, &mut out);
    out
}

/// Whether `MOSTIK_PUSH_LOG=<file>` asks for a traffic log (protocol research only: the file
/// holds message texts and names).
pub fn traffic_logging() -> bool {
    std::env::var_os("MOSTIK_PUSH_LOG").is_some()
}

/// Appends one record to the traffic log, with the time in ms; does nothing without the variable.
pub fn traffic_log(mut record: Value) {
    use std::io::Write as _;
    let Some(path) = std::env::var_os("MOSTIK_PUSH_LOG") else {
        return;
    };
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64);
    record["t"] = json!(t);
    // Several threads log at once: one lock and one write per line keep the lines whole.
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let line = format!("{record}\n");
    let _guard = LOCK.lock();
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = f.write_all(line.as_bytes());
    }
}

/// Bytes as a hex string, for bodies that are not JSON.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shape_hides_values_and_merges_items() {
        let v = json!({"a": "secret", "list": [{"x": 1}, {"y": true}]});
        assert_eq!(
            shape(&v),
            json!({"a": "string", "list": {"len": 2, "item": {"x": "number", "y": "bool"}}})
        );
    }

    #[test]
    fn id_patterns() {
        assert_eq!(id_pattern("0/0/00000000-0000-4000-8000-000000000001"), "0/0/<hex36>");
        assert_eq!(id_pattern("abc_def"), "<hex3>_<hex3>");
        assert_eq!(id_pattern("chat_x9"), "<str4>_<str2>");
    }
}
