//! Telemost reaction types (the enum from web client 3.2.7) and their emoji.

const TABLE: &[(u64, &str, &str)] = &[
    (100101, "CHECK", "✅"),
    (100102, "LIKE", "👍"),
    (100103, "DISLIKE", "👎"),
    (100104, "FIRE", "🔥"),
    (100105, "CRYING", "😢"),
    (100106, "SURPRISED", "😮"),
    (100107, "GOOSE", "🪿"),
    (100108, "SMILE", "😄"),
    (100109, "HEART", "❤️"),
    (100110, "EYES", "👀"),
    (100111, "PLUS", "➕"),
    (100112, "FEARFUL", "😨"),
    (100113, "FOUR_PLUS", "4️⃣➕"),
    (100114, "OK", "👌"),
    (100115, "EXPLODING_HEAD", "🤯"),
    (100116, "BROKEN_HEART", "💔"),
    (100117, "SALUTE", "🫡"),
    (100118, "FACE_VOMITING", "🤮"),
    (100119, "POOP", "💩"),
    (100120, "HAND", "✋"),
    (100121, "MOYAI", "🗿"),
    (100122, "FACEPALM", "🤦"),
    (100123, "TADA", "🎉"),
    (100124, "MINUS", "➖"),
    (100125, "HEART_EYES", "😍"),
    (100126, "THINKING", "🤔"),
    (100127, "CLOWN", "🤡"),
    (100128, "EXPRESSIONLESS", "😑"),
    (100129, "CLAP", "👏"),
    (100130, "SUNGLASSES", "😎"),
    (100131, "HUNDRED_POINTS", "💯"),
    (100132, "FLOWERS_YELLOW", "🌼"),
    (100133, "FLOWERS_RED", "🌹"),
    (100134, "HANDSHAKE", "🤝"),
    (100135, "PRAY", "🙏"),
    (100136, "SLIGHT_SMILE", "🙂"),
    (100137, "JOY", "😂"),
    (100138, "SMIRK", "😏"),
    (100139, "PENSIVE", "😔"),
    (100140, "ZIPPER_MOUTH_FACE", "🤐"),
    (100141, "WOOZY_FACE", "🥴"),
    (100142, "SHAKING_FACE", "🫨"),
    (100143, "ANGRY", "😠"),
    (100144, "IMP", "👿"),
    (100145, "BLUE_HEART", "💙"),
    (100146, "PINK_HEART", "🩷"),
    (100147, "BLACK_HEART", "🖤"),
    (100148, "VIOLET_HEART", "💜"),
    (100149, "GREEN_HEART", "💚"),
    (100150, "WHITE_HEART", "🤍"),
    (100151, "ORANGE_HEART", "🧡"),
    // Branded reactions with no emoji counterpart: the closest match by meaning.
    (100152, "PIXEL_HEART", "💗"),
    (100153, "NEW_MOON_WITH_FACE", "🌚"),
    (100154, "HAMBURGER", "🍔"),
    (100155, "LOGO_360", "🔵"),
    (100156, "ROWAN", "🍒"),
    (100157, "CALENDAR", "📅"),
];

/// Default reaction set in work chats (`reactions_by_namespace.default_team` in config.json).
pub const DEFAULT_SET: [u64; 10] = [
    100109, 100108, 100104, 100102, 100103, 100107, 100106, 100105, 100110, 100101,
];

/// All Telemost reactions as (type, name, emoji), for the full reaction picker.
pub fn all() -> impl Iterator<Item = (u64, &'static str, &'static str)> {
    TABLE.iter().copied()
}

/// Emoji for a reaction type; `❔` for an unknown one.
pub fn emoji(kind: u64) -> &'static str {
    TABLE.iter().find(|(id, _, _)| *id == kind).map_or("❔", |(_, _, e)| e)
}

/// Protocol name of a reaction type.
pub fn name(kind: u64) -> Option<&'static str> {
    TABLE.iter().find(|(id, _, _)| *id == kind).map(|(_, n, _)| *n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_and_unknown() {
        assert_eq!(emoji(100102), "👍");
        assert_eq!(name(100123), Some("TADA"));
        assert_eq!(emoji(1), "❔");
        let mut ids: Vec<u64> = TABLE.iter().map(|t| t.0).collect();
        ids.dedup();
        assert_eq!(ids.len(), TABLE.len(), "IDs are unique");
    }
}
