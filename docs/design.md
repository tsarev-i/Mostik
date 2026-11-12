# Mostik design

Layout and behaviour follow Slack. The metrics were taken from a Slack 4.53 screenshot (Flatpak, KDE Wayland,
display scale 1.2; values converted to CSS pixels) in a test workspace without anyone else's data.
Reference screenshots are kept locally in `docs/design/reference/` and never go into the repository.

Slack's name, logo and icons are not used. The icons are our own in the Lucide style (ISC).
The app icon is `docs/design/app-icon.png` (source: `app-icon.svg`, a suspension bridge whose cable draws the letter
"М"). Tauri icons are generated with `cargo tauri icon docs/design/app-icon.png -o app/src-tauri/icons`
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

## Colours (measured from Slack, dark mode)

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

The rail and sidebar rows above are Slack's Aubergine; Mostik no longer has that theme.

The colour mode (light, dark) paints the main column, dialogs and menus; a theme paints the frame, rail and sidebar
(`:root[data-theme]` blocks). Preferences → Appearance shows Mostik's own themes first, Dusk (the default) and Moss,
then the classic ones: Twilight (in place of Aubergine), Midnight, Forest, Ocean, Sunset. Dusk is a whole palette:
in dark mode it also sets the message column, links, inline code and accents; light mode keeps its own column.
Text on the chrome takes its colour from `--chrome-ink` (an RGB triple for `rgba()`) and `--chrome-strong` rather
than a fixed white. A theme's swatch in Preferences carries `data-theme` and paints the theme's own sidebar and
selected-row tokens, so it cannot drift from the theme.

## Typography

Lato (OFL), with Noto Sans as the fallback. Message text 15/22px; author name 15px, weight 900;
time 12px in the secondary colour; chat title 18px, 900; workspace name 18px, 900;
rail captions 11px, 700.

Lato is not bundled yet: the font can be placed in `app/ui/assets` (the OFL allows it);
until then Noto Sans is used.

## Behaviour

- Consecutive messages of one author within 5 minutes are grouped without an avatar; the time shows on hover.
- Hovering a message shows the action bar: reaction, reply in thread, forward, "important", more.
- The message menu starts with "Reply" and, when text of the message is selected, "Quote selection"
  and "Copy selection"; a right click on selected text of a message opens this menu instead of the
  system one. A reply puts a bar over its chat's or thread's field (whom it answers, the quote or the
  start of the text, × and Escape cancel it); the sent reply shows the original above its text, and a
  click on it scrolls to the original.
- The day separator is a centred pill that sticks to the top of the feed.
- A collapsed sidebar section still shows chats with unread messages; the open chat hides with the rest.
- Sidebar section headers have no caret, as in Slack: on hover it shows in place of the section icon
  (the star of "Starred", the hash of "Channels", the speech bubbles of "Direct messages", a folder's
  emoji) or left of an icon-less title, and turns down over an expanded section. A section with an
  icon reads like a chat row; icon-less ones ("Meeting chats", folders without an emoji) are smaller
  muted headers. Chats sit a little to the right of their header, as in Slack.
- A section header shows "+" and "⋮" on hover: "+" creates a channel in "Channels" and in one's own
  sections (the channel goes into that section), a message in "Direct messages"; "⋮" (or a right
  click) opens the section menu as in Slack: what the section makes, managing it, filter and sort
  (common to all sections in Mostik), deleting it.
- "Find a conversation" above the sections, as in Slack: while it has text, the sidebar shows only
  the conversations whose name has it (people's names for direct chats), collapsed sections and
  the "Active only" filter aside. Enter opens the first one, Escape clears the field.
- A mention of yourself is highlighted with a background and a bar on the left; "important" gets a green label
  (the counterpart of "Saved" in Slack).
- Enter sends, Shift+Enter inserts a line break.
- The message field is a plain-text `contenteditable` block rather than a textarea, so custom emoji show
  as images while its text keeps them as `:name:`, and markup shows as it will look (bold as bold) with
  its markers hidden, as in Slack; the formatting buttons toggle it. Editing a message, as in Slack,
  hides the author line and shows the field with the formatting bar at its bottom and "Cancel" /
  "Save changes" under it. Custom emoji are added with "Add emoji" at the bottom
  of the emoji picker, as in Slack, or in Preferences → Custom emoji, which also lists them and renames or
  deletes your own; in the picker an own one is deleted with a right click.

## Differences from Slack dictated by Telemost

- Telemost has no custom emoji, so Mostik clients share them through the service chat `mostik-service` of
  an organization: a public group with the alias `mostiksvc-<organization>`, marked `mostik:shared:v2` in its
  description, which Mostik hides, mutes and keeps read; it joins the chat by its invite. One emoji is one
  image message there with the caption `:name:`; only its author can rename or delete it, and anything else
  in the chat is ignored. Texts carry them as `:name:`; a custom reaction is a Telemost reaction of Mostik's own type (`0x4D000000` + the message number), since Telemost stores any
  32-bit type. The official client shows the text and an empty reaction.
- Telemost keeps one reaction per person on a message: a new one replaces the previous one.
- Markup: bold, italic, strikethrough, underline, code, links and code blocks. Lists and `>` quotes are
  not markup in Telemost; a quote is a reply carrying a fragment of the original
  (`ForwardedMessageRefs` with `ForwardedMessageStyles[].Quote`), as the official client sends it.
- A call opens in its own window, like a huddle: the Telemost meeting page, titled "Call · #channel", with
  the current call shown at the bottom of the sidebar (where, and "Leave") and the header call button
  turned green. The window shows only the meeting page (`/private-join/<id>`, the iframe of the messenger's
  `/j/<id>`), without the messenger and the Yandex 360 bar. Leaving the meeting closes the window, and a
  leftover "In meeting" status is cleared. WebKitGTK has no WebRTC, so on Linux the window is an
  installed Chromium-based browser in app mode driven over a DevTools pipe (`call.rs`, `cdp.rs`); on Windows
  it is a WebView2 window.
- A call is one line in its chat, as a huddle is in Slack: who started it, how many are in it now
  (`StateSync` `meeting_updated`) and "Join"; once over, how long it lasted (`DurationSeconds` of a direct
  call, the time since the start for a group one). A group call started outside a chat gets a temporary
  chat of its own (`0/58/…`, "Групповой звонок 01.02 в 10:00") whose creation starts its first call; these
  chats are meeting chats, or with Preferences → Home → "Group calls in a section of their own" a
  "Group calls" section of Mostik's own (not a Telemost folder, so nothing is written to Telemost).
