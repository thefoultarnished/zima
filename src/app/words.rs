//! Spelling panel (Windows spell checker) and word lookup.

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use super::App;
use crate::{LookupRow, SpellRow};

/// Misspelled words shown at once (each needs a suggestion lookup).
const MAX_WORDS: usize = 30;

impl App {
    pub fn open_spelling(&mut self) {
        #[cfg(windows)]
        if self.spell.is_none() {
            self.spell = crate::spell::Checker::new();
        }
        if !self.spell_available() {
            self.toast("Spell checking isn't available on this PC", true);
            return;
        }
        self.refresh_spelling();
        if let Some(ui) = self.ui.upgrade() {
            ui.set_words_mode("spell".into());
            ui.set_words_open(true);
        }
    }

    fn spell_available(&self) -> bool {
        #[cfg(windows)]
        return self.spell.is_some();
        #[cfg(not(windows))]
        false
    }

    fn refresh_spelling(&self) {
        let (Some(ui), Some(note)) = (self.ui.upgrade(), self.current()) else { return };
        #[cfg(windows)]
        let rows: Vec<SpellRow> = {
            let Some(checker) = &self.spell else { return };
            // Group by word, in order of first appearance.
            let mut words: Vec<(String, i32)> = Vec::new();
            for m in checker.check(&note.body) {
                match words.iter_mut().find(|(w, _)| *w == m.word) {
                    Some((_, count)) => *count += 1,
                    None => words.push((m.word, 1)),
                }
            }
            words
                .into_iter()
                .take(MAX_WORDS)
                .map(|(word, count)| {
                    let suggestions: Vec<SharedString> = checker.suggest(&word, 3).into_iter().map(Into::into).collect();
                    SpellRow { word: word.into(), count, suggestions: ModelRc::new(VecModel::from(suggestions)) }
                })
                .collect()
        };
        #[cfg(not(windows))]
        let rows: Vec<SpellRow> = Vec::new();
        ui.set_spell_rows(ModelRc::new(VecModel::from(rows)));
    }

    /// Replace every whole-word occurrence of `word` with `replacement`.
    pub fn spell_replace(&mut self, word: String, replacement: String) {
        let Some(body) = self.current().map(|n| n.body.clone()) else { return };
        let mut out = String::with_capacity(body.len());
        let mut rest = body.as_str();
        let mut previous: Option<char> = None;
        while let Some(i) = rest.find(&word) {
            let before = rest[..i].chars().next_back().or(previous);
            let after = rest[i + word.len()..].chars().next();
            let whole = !before.is_some_and(|c| c.is_alphanumeric()) && !after.is_some_and(|c| c.is_alphanumeric());
            out.push_str(&rest[..i]);
            out.push_str(if whole { &replacement } else { &word });
            previous = word.chars().next_back();
            rest = &rest[i + word.len()..];
        }
        out.push_str(rest);
        if out != body {
            self.replace_body(out, None);
        }
        self.refresh_spelling();
    }

    pub fn spell_ignore(&mut self, word: String, add_to_dictionary: bool) {
        #[cfg(windows)]
        if let Some(checker) = &self.spell {
            if add_to_dictionary {
                checker.add(&word);
            } else {
                checker.ignore(&word);
            }
        }
        let _ = (&word, add_to_dictionary);
        self.refresh_spelling();
    }

    /// Look up the selected word (or the word at the cursor) online.
    pub fn look_up_word(&mut self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let Some(body) = self.current().map(|n| n.body.clone()) else { return };
        let (anchor, cursor) = self.selection();
        let (start, end) = (anchor.min(cursor), anchor.max(cursor));
        let word = if start < end && end <= body.len() {
            body[start..end].trim().to_string()
        } else {
            // Expand around the cursor.
            let is_word = |c: char| c.is_alphanumeric() || c == '\'' || c == '-';
            let left = body[..cursor.min(body.len())].char_indices().rev().take_while(|(_, c)| is_word(*c)).last().map_or(cursor, |(i, _)| i);
            let right = body[cursor.min(body.len())..].char_indices().find(|(_, c)| !is_word(*c)).map_or(body.len(), |(i, _)| cursor + i);
            body[left.min(right)..right].to_string()
        };
        ui.set_lookup_word(word.as_str().into());
        ui.set_lookup_status("Looking up\u{2026}".into());
        ui.set_lookup_rows(ModelRc::default());
        ui.set_words_mode("lookup".into());
        ui.set_words_open(true);

        // Network request off the UI thread; results come back through the event loop.
        let weak = ui.as_weak();
        std::thread::spawn(move || {
            let result = crate::spell::look_up(&word);
            let _ = slint::invoke_from_event_loop(move || {
                let Some(ui) = weak.upgrade() else { return };
                match result {
                    Ok(meanings) => {
                        let rows: Vec<LookupRow> = meanings
                            .into_iter()
                            .map(|(part, definitions, synonyms)| LookupRow {
                                part: part.into(),
                                definitions: ModelRc::new(VecModel::from(definitions.into_iter().map(SharedString::from).collect::<Vec<_>>())),
                                synonyms: synonyms.join(", ").into(),
                            })
                            .collect();
                        ui.set_lookup_status("".into());
                        ui.set_lookup_rows(ModelRc::new(VecModel::from(rows)));
                    }
                    Err(message) => ui.set_lookup_status(message.into()),
                }
            });
        });
    }
}
