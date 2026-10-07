//! Note emoji: the short list in the picker and the check for what counts as an emoji.

/// (emoji, name) pairs offered in the picker.
pub const COMMON: &[(&str, &str)] = &[
    ("\u{1F4CC}", "Pin"),
    ("\u{2B50}", "Star"),
    ("\u{2764}\u{FE0F}", "Heart"),
    ("\u{1F525}", "Fire"),
    ("\u{2705}", "Done"),
    ("\u{1F4A1}", "Idea"),
    ("\u{1F4DA}", "Reading"),
    ("\u{1F4DD}", "Notes"),
    ("\u{1F4C5}", "Plans"),
    ("\u{1F3AF}", "Goal"),
    ("\u{1F680}", "Launch"),
    ("\u{1F389}", "Party"),
    ("\u{1F4BC}", "Work"),
    ("\u{1F3E0}", "Home"),
    ("\u{1F6D2}", "Shopping"),
    ("\u{2708}\u{FE0F}", "Travel"),
    ("\u{1F4B0}", "Money"),
    ("\u{1F3B5}", "Music"),
    ("\u{1F373}", "Cooking"),
    ("\u{1F3C3}", "Fitness"),
    ("\u{1F331}", "Growth"),
    ("\u{1F512}", "Private"),
    ("\u{26A0}\u{FE0F}", "Warning"),
    ("\u{2753}", "Question"),
];

/// Longest accepted emoji, in characters. Joined emoji (a family, a flag with a skin tone) use several.
const MAX_CHARS: usize = 8;

/// The trimmed emoji if `input` looks like one: no letters, digits, spaces or control characters.
pub fn clean(input: &str) -> Option<String> {
    let text = input.trim();
    let ok = !text.is_empty()
        && text.chars().count() <= MAX_CHARS
        && !text.is_ascii()
        && !text.chars().any(|c| c.is_alphanumeric() || c.is_whitespace() || c.is_control());
    ok.then(|| text.to_string())
}

/// One row in the emoji picker.
pub struct Choice {
    pub emoji: String,
    pub label: String,
    pub score: i32,
}

/// Picker rows for what the user typed: the typed emoji first, or the named ones that match.
pub fn choices(query: &str) -> Vec<Choice> {
    if let Some(emoji) = clean(query) {
        let label = COMMON.iter().find(|(e, _)| *e == emoji).map_or("Use this emoji", |(_, name)| name);
        return vec![Choice { emoji, label: label.into(), score: 1000 }];
    }
    COMMON
        .iter()
        .filter_map(|(emoji, name)| {
            crate::palette::fuzzy_score(query, name).map(|score| Choice { emoji: (*emoji).into(), label: (*name).into(), score })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_accepts_single_emoji() {
        assert_eq!(clean("\u{1F389}").as_deref(), Some("\u{1F389}"));
        assert_eq!(clean(" \u{1F4CC} ").as_deref(), Some("\u{1F4CC}"));
        assert!(clean("\u{1F469}\u{200D}\u{1F4BB}").is_some());
        assert!(clean("\u{1F1F3}\u{1F1F1}").is_some());
        assert!(clean("\u{1F44D}\u{1F3FD}").is_some());
    }

    #[test]
    fn clean_rejects_text_and_blank() {
        for bad in ["", "   ", "abc", "\u{1F389} party", "\u{65E5}\u{672C}", "7", "\u{1F389}\u{1F389}\u{1F389}\u{1F389}\u{1F389}\u{1F389}\u{1F389}\u{1F389}\u{1F389}\u{1F389}\u{1F389}"] {
            assert_eq!(clean(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn common_entries_are_clean_and_unique() {
        for (i, (emoji, name)) in COMMON.iter().enumerate() {
            assert_eq!(clean(emoji).as_deref(), Some(*emoji), "{name}");
            assert!(COMMON.iter().skip(i + 1).all(|(e, n)| e != emoji && n != name), "{name}");
        }
    }

    #[test]
    fn choices_empty_lists_all() {
        let all = choices("");
        assert_eq!(all.len(), COMMON.len());
        assert!(all.iter().all(|c| c.score == 0));
        assert_eq!(all[0].emoji, COMMON[0].0);
    }

    #[test]
    fn choices_filters_by_name() {
        let found = choices("launch");
        assert_eq!(found[0].emoji, "\u{1F680}");
        assert!(found.len() < COMMON.len());
    }

    #[test]
    fn choices_typed_emoji_first() {
        let found = choices("\u{1F984}");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].label, "Use this emoji");
        assert_eq!(found[0].score, 1000);
    }

    #[test]
    fn choices_typed_common_not_duplicated() {
        let found = choices("\u{2B50}");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].label, "Star");
    }

    #[test]
    fn choices_no_match() {
        assert!(choices("zzzz").is_empty());
    }
}
