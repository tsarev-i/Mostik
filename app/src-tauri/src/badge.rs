//! The unread mark on the app icons, as in Slack: a blue dot on the tray icon for unread messages,
//! a red one for unread direct messages and mentions. On Windows the red mark also goes on the
//! taskbar button, on macOS a badge with their number on the Dock icon.

use mostik_core::UnreadBadge;
use tauri::AppHandle;
use tauri::image::Image;

/// Slack's colours of the two marks.
const BLUE: [u8; 3] = [0x1d, 0x9b, 0xd1];
const RED: [u8; 3] = [0xe0, 0x1e, 0x5a];

/// Shows `badge` on the tray icon, and on the taskbar button or the Dock icon; `count` is the
/// number of unread direct messages and mentions behind a red mark.
pub fn show(app: &AppHandle, badge: UnreadBadge, count: u32) -> tauri::Result<()> {
    if let (Some(tray), Some(icon)) = (app.tray_by_id("main"), app.default_window_icon()) {
        let image = match badge {
            UnreadBadge::None => icon.clone(),
            UnreadBadge::Unread => with_dot(icon, BLUE),
            UnreadBadge::Mention => with_dot(icon, RED),
        };
        tray.set_icon(Some(image))?;
    }
    #[cfg(windows)]
    if let Some(window) = tauri::Manager::get_webview_window(app, "main") {
        window.set_overlay_icon((badge == UnreadBadge::Mention).then(|| dot(16, RED)))?;
    }
    #[cfg(target_os = "macos")]
    if let Some(window) = tauri::Manager::get_webview_window(app, "main") {
        window.set_badge_label(match badge {
            UnreadBadge::None => None,
            UnreadBadge::Unread => Some("•".to_owned()),
            UnreadBadge::Mention => Some(count.to_string()),
        })?;
    }
    #[cfg(not(target_os = "macos"))]
    let _ = count;
    Ok(())
}

/// `icon` with a dot of `color` in its bottom right corner, set apart from the icon by a
/// transparent ring so that it reads on any panel.
fn with_dot(icon: &Image<'_>, color: [u8; 3]) -> Image<'static> {
    let (w, h) = (icon.width(), icon.height());
    let mut rgba = icon.rgba().to_vec();
    let size = w.min(h) as f32;
    let (radius, ring) = (size * 0.22, size * 0.07);
    let centre = (w as f32 - radius - ring / 2.0, h as f32 - radius - ring / 2.0);
    paint(&mut rgba, w, centre, radius, ring, color);
    Image::new_owned(rgba, w, h)
}

/// A round mark of `color`, `size` pixels across, on a transparent square.
#[cfg(any(windows, test))]
fn dot(size: u32, color: [u8; 3]) -> Image<'static> {
    let mut rgba = vec![0; (size * size * 4) as usize];
    let half = size as f32 / 2.0;
    paint(&mut rgba, size, (half, half), half - 0.5, 0.0, color);
    Image::new_owned(rgba, size, size)
}

/// Paints a disc of `color` and `radius` at `centre` over the RGBA pixels of a `width` wide
/// picture, clearing a ring of `ring` around it; the edges are anti-aliased.
fn paint(rgba: &mut [u8], width: u32, centre: (f32, f32), radius: f32, ring: f32, color: [u8; 3]) {
    for (i, px) in rgba.chunks_exact_mut(4).enumerate() {
        let (x, y) = ((i as u32 % width) as f32 + 0.5, (i as u32 / width) as f32 + 0.5);
        let d = ((x - centre.0).powi(2) + (y - centre.1).powi(2)).sqrt();
        let cleared = (radius + ring + 0.5 - d).clamp(0.0, 1.0);
        if cleared == 0.0 {
            continue;
        }
        // The icon fades out in the ring, the dot goes over what is left of it.
        let under = f32::from(px[3]) / 255.0 * (1.0 - cleared);
        let over = (radius + 0.5 - d).clamp(0.0, 1.0);
        let alpha = over + under * (1.0 - over);
        if alpha > 0.0 {
            for (channel, &paint) in px[..3].iter_mut().zip(&color) {
                let mixed = f32::from(paint) * over + f32::from(*channel) * under * (1.0 - over);
                *channel = (mixed / alpha).round() as u8;
            }
        }
        px[3] = (alpha * 255.0).round() as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(image: &Image<'_>, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * image.width() + x) * 4) as usize;
        image.rgba()[i..i + 4].try_into().unwrap()
    }

    #[test]
    fn dot_goes_in_the_bottom_right_corner() {
        let icon = Image::new_owned([10, 20, 30, 255].repeat(64 * 64), 64, 64);
        let marked = with_dot(&icon, RED);
        assert_eq!((marked.width(), marked.height()), (64, 64));
        // The dot: its colour, opaque.
        assert_eq!(pixel(&marked, 50, 50), [0xe0, 0x1e, 0x5a, 255]);
        // The ring around it: the icon is cut out.
        assert_eq!(pixel(&marked, 47, 31)[3], 0);
        // The rest of the icon stays as it was.
        assert_eq!(pixel(&marked, 5, 5), [10, 20, 30, 255]);
        assert_eq!(pixel(&marked, 60, 5), [10, 20, 30, 255]);
    }

    #[test]
    fn overlay_dot_is_a_circle() {
        let overlay = dot(16, RED);
        assert_eq!(pixel(&overlay, 8, 8), [0xe0, 0x1e, 0x5a, 255]);
        assert_eq!(pixel(&overlay, 0, 0)[3], 0);
        assert_eq!(pixel(&overlay, 15, 15)[3], 0);
    }
}
