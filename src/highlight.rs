//! Syntax highlighting for code blocks in the preview.
//!
//! Each line becomes styled-text Markdown with `<font color>` tags, which Slint's StyledText renders.

use std::sync::OnceLock;

use syntect::easy::HighlightLines;
use syntect::highlighting::{Theme, ThemeSet};
use syntect::parsing::SyntaxSet;

fn syntaxes() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

fn theme(dark: bool) -> &'static Theme {
    static THEMES: OnceLock<ThemeSet> = OnceLock::new();
    let themes = THEMES.get_or_init(ThemeSet::load_defaults);
    &themes.themes[if dark { "base16-ocean.dark" } else { "InspiredGitHub" }]
}

/// Highlight `code` in `language` (a fence tag like "rust" or "py"). Unknown languages come back plain.
pub fn highlight(code: &str, language: &str, dark: bool) -> Vec<String> {
    let set = syntaxes();
    let syntax = (!language.trim().is_empty())
        .then(|| set.find_syntax_by_token(language.trim()))
        .flatten();
    let Some(syntax) = syntax else {
        return code.lines().map(plain).collect();
    };
    let mut highlighter = HighlightLines::new(syntax, theme(dark));
    let mut lines = Vec::new();
    for line in code.split('\n') {
        let with_newline = format!("{line}\n");
        let Ok(ranges) = highlighter.highlight_line(&with_newline, set) else {
            lines.push(plain(line));
            continue;
        };
        let mut out = String::new();
        for (style, text) in ranges {
            let text = text.trim_end_matches('\n');
            if text.is_empty() {
                continue;
            }
            let c = style.foreground;
            out.push_str(&format!("<font color=\"#{:02x}{:02x}{:02x}\">{}</font>", c.r, c.g, c.b, protect(text)));
        }
        lines.push(if out.is_empty() { "\u{00a0}".to_string() } else { out });
    }
    // Drop the trailing empty line from a final newline.
    while lines.len() > 1 && lines.last().is_some_and(|l| l == "\u{00a0}") && code.ends_with('\n') {
        lines.pop();
    }
    lines
}

/// Escape Markdown and keep spaces (indentation) from collapsing.
fn protect(text: &str) -> String {
    crate::markdown::escape(text).replace(' ', "\u{00a0}")
}

fn plain(line: &str) -> String {
    if line.is_empty() { "\u{00a0}".to_string() } else { protect(line) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlights_known_languages() {
        let lines = highlight("fn main() {\n    let x = 1;\n}", "rust", true);
        assert_eq!(lines.len(), 3);
        assert!(lines[0].contains("<font color="), "{}", lines[0]);
        // Indentation survives as non-breaking spaces.
        assert!(lines[1].contains('\u{00a0}'));
    }

    #[test]
    fn unknown_language_is_plain() {
        assert_eq!(highlight("a *b*", "nope", false), vec!["a\u{00a0}\\*b\\*".to_string()]);
    }
}
