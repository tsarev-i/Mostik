# Mostik design

Layout and behaviour follow Slack. The metrics were taken from a Slack 4.53 screenshot (Flatpak, KDE Wayland,
display scale 1.2; values converted to CSS pixels) in a test workspace without anyone else's data.
Reference screenshots are kept locally in `docs/design/reference/` and never go into the repository.

Slack's name, logo and icons are not used. The icons are our own in the Lucide style (ISC).
The app icon is `docs/design/app-icon.png` (source: `app-icon.svg`, a lowercase Cyrillic "м" drawn as an arched
bridge over water). Tauri icons are generated with `cargo tauri icon docs/design/app-icon.png -o app/src-tauri/icons`
(remove the `android` and `ios` directories afterwards).

## Layout

| Element | Size |
|---|---|
| Top bar (navigation, search, help) | 40px |
| Rail (workspace, Home, DMs, Activity; "+" and the avatar at the bottom) | 70px |
| Sidebar | 270px |
| Chat header: title row + tabs row | 49px + 37px |
| Sidebar row | 28px, 8px inset from the edges, 6px radius |
| Thread panel | 400px |
| Avatar in the feed | 36px, 8px radius |
| Workspace tile in the rail | 36px, 8px radius |
| Composer | 20px padding; toolbar, field, bottom bar |

The sidebar and the main column sit on a shared card with an 8px radius on top of the
gradient window background.

## Colours (dark theme, the default)

| Token | Value | Measured at |
|---|---|---|
| Rail, gradient | `#3F1745` → `#290C2A` | top and bottom of the rail |
| Sidebar, gradient | `#28192B` → `#1E1420` | top and bottom of the sidebar |
| Selected sidebar row | `#7C3C85` | the open chat |
| Main background | `#202326` | the feed |
| Chat header | `#25272B` | the title row |
| Composer field | `#272A2D`, border `#37393D` | the composer |
| Send button | `#007A5A` | active send |
| Top bar search | white 20% over the gradient | the search field |
| Workspace tile | `#A9A9AC` | the rail |

The other tokens (badge `#E01E5A`, online `#2BAC76`, mention highlight and so on) are
the `:root` variables in `app/ui/assets/app.css`.

## Typography

Lato (OFL), with Noto Sans as the fallback. Message text 15/22px; author name 15px, weight 900;
time 12px in the secondary colour; chat title 18px, 900; workspace name 18px, 900;
rail captions 11px, 700.

Lato is not bundled yet: the font can be placed in `app/ui/assets` (the OFL allows it);
until then Noto Sans is used.

## Behaviour

- Consecutive messages of one author within 5 minutes are grouped without an avatar; the time shows on hover.
- Hovering a message shows the action bar: reaction, reply in thread, forward, "important", more.
- The day separator is a centred pill that sticks to the top of the feed.
- A collapsed sidebar section still shows the open chat and chats with unread messages.
- A mention of yourself is highlighted with a background and a bar on the left; "important" gets a green label
  (the counterpart of "Saved" in Slack).
- Enter sends, Shift+Enter inserts a line break.

## Differences from Slack dictated by Telemost

- Reactions come only from the Telemost set, no custom emoji.
- Markup: bold, italic, strikethrough, underline, code, links and code blocks. Lists and quotes are not supported.
- A call opens in the external browser: WebKitGTK has no WebRTC.
