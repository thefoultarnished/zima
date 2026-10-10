//! Markdown → a flat list of blocks for the preview.
//!
//! Block structure (headings, lists, tables, …) is handled here; inline formatting (bold, italic, code,
//! links, underline) is re-emitted as Markdown for Slint's `StyledText` to render.

use pulldown_cmark::{BlockQuoteKind, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    Heading,
    Paragraph,
    Item,
    Task,
    Quote,
    Code,
    Rule,
    Row,
    /// `> [!tip]` box; `marker` is its title on the first paragraph.
    Callout,
    /// A footnote definition; `marker` is its label.
    Footnote,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Heading => "heading",
            Kind::Paragraph => "paragraph",
            Kind::Item => "item",
            Kind::Task => "task",
            Kind::Quote => "quote",
            Kind::Code => "code",
            Kind::Rule => "rule",
            Kind::Row => "row",
            Kind::Callout => "callout",
            Kind::Footnote => "footnote",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub kind: Kind,
    /// Inline Markdown (plain text for code blocks).
    pub text: String,
    /// Heading level (1–6); 1 for a table header row.
    pub level: u8,
    /// List marker: "•", "◦", "3." …
    pub marker: String,
    /// Nesting depth of lists.
    pub indent: u8,
    pub checked: bool,
    pub cells: Vec<String>,
    /// 0-based source line, used to toggle task checkboxes.
    pub line: usize,
    /// Space above the block, in logical pixels.
    pub space: u8,
}

impl Block {
    pub fn new(kind: Kind, text: String, line: usize) -> Self {
        Self { kind, text, level: 0, marker: String::new(), indent: 0, checked: false, cells: Vec::new(), line, space: 0 }
    }
}

struct ListState {
    next: Option<u64>,
}

/// Stands in for a `==` in a block's text until [`highlights`] knows the theme's colour.
/// A private-use character, so it never clashes with what people type.
const MARK: &str = "\u{E000}";

/// A named text colour for `==red:text==`, as "#rrggbb" for light and dark themes.
pub struct TextColour {
    pub name: &'static str,
    pub light: &'static str,
    pub dark: &'static str,
}

/// The colours the formatting toolbar offers, readable on light and dark themes.
pub const TEXT_COLOURS: [TextColour; 7] = [
    TextColour { name: "red", light: "#dc2626", dark: "#f87171" },
    TextColour { name: "orange", light: "#ea580c", dark: "#fb923c" },
    TextColour { name: "green", light: "#16a34a", dark: "#4ade80" },
    TextColour { name: "blue", light: "#2563eb", dark: "#60a5fa" },
    TextColour { name: "purple", light: "#9333ea", dark: "#c084fc" },
    TextColour { name: "pink", light: "#db2777", dark: "#f472b6" },
    TextColour { name: "gray", light: "#6b7280", dark: "#9ca3af" },
];

/// The colour named at the start of a highlight (`red:` in `==red:text==`) and the text after it.
/// Only known names count, and only when some text follows.
fn named_colour(inner: &str) -> Option<(&'static TextColour, &str)> {
    let (name, rest) = inner.split_once(':')?;
    let colour = TEXT_COLOURS.iter().find(|c| c.name.eq_ignore_ascii_case(name))?;
    (!rest.is_empty()).then_some((colour, rest))
}

/// Turn the `==highlight==` marks in a block's inline Markdown into coloured text (StyledText can't
/// draw a background). Like Obsidian, a mark opens before a non-space and closes after one;
/// a mark left without a partner is shown as a plain `==`. `==red:text==` picks a named colour
/// instead of `color`.
pub fn highlights(inline: &str, color: &str, dark: bool) -> String {
    let parts: Vec<&str> = inline.split(MARK).collect();
    let mut out = String::with_capacity(inline.len());
    out.push_str(parts[0]);
    let mut i = 1;
    while i < parts.len() {
        let opens = parts[i].starts_with(|c: char| !c.is_whitespace());
        // The partner: the next mark that comes right after a non-space.
        let close = (i + 1..parts.len()).find(|&j| parts[j - 1].ends_with(|c: char| !c.is_whitespace()));
        match close {
            Some(j) if opens => {
                let inner = parts[i..j].join("==");
                let (color, inner) = match named_colour(&inner) {
                    Some((named, rest)) => (if dark { named.dark } else { named.light }, rest),
                    None => (color, inner.as_str()),
                };
                out.push_str(&format!("<font color=\"{color}\">"));
                out.push_str(inner);
                out.push_str("</font>");
                out.push_str(parts[j]);
                i = j + 1;
            }
            _ => {
                out.push_str("==");
                out.push_str(parts[i]);
                i += 1;
            }
        }
    }
    out
}

/// The same text with every `==` mark left as typed and `@words` uncoloured, for when the
/// colours can't be shown.
pub fn plain_marks(inline: &str) -> String {
    inline.replace(MARK, "==").replace([KEYWORD_START, KEYWORD_END], "")
}

/// Around an `@word` (`@due`, `@remind`…) until [`keywords`] knows the colour. Private-use
/// characters, like [`MARK`].
const KEYWORD_START: char = '\u{E001}';
const KEYWORD_END: char = '\u{E002}';

/// Mark every `@word` in plain text: an `@` at the start of a word (so not in `me@mail.com`)
/// followed by a letter, then letters, digits, `-` or `_`.
fn mark_keywords(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.char_indices().peekable();
    let mut previous: Option<char> = None;
    while let Some((_, c)) = chars.next() {
        let starts_word = previous.is_none_or(|p| !p.is_alphanumeric() && p != '@' && p != '_');
        if c == '@' && starts_word && chars.peek().is_some_and(|&(_, n)| n.is_alphabetic()) {
            out.push(KEYWORD_START);
            out.push('@');
            while let Some(&(_, n)) = chars.peek() {
                if !(n.is_alphanumeric() || n == '-' || n == '_') {
                    break;
                }
                out.push(n);
                previous = Some(n);
                chars.next();
            }
            out.push(KEYWORD_END);
            continue;
        }
        out.push(c);
        previous = Some(c);
    }
    out
}

/// Colour the marked `@words` in a block's inline Markdown.
pub fn keywords(inline: &str, color: &str) -> String {
    inline.replace(KEYWORD_START, &format!("<font color=\"{color}\">")).replace(KEYWORD_END, "</font>")
}

/// `[[Note title]]` → a link with a `note:` URL (spaces as %20). Newlines are kept, so line numbers don't move.
fn wiki_links(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut rest = source;
    while let Some(start) = rest.find("[[") {
        let after = &rest[start + 2..];
        match after.find("]]") {
            Some(end) if !after[..end].contains('\n') && !after[..end].trim().is_empty() => {
                let title = after[..end].trim();
                out.push_str(&rest[..start]);
                out.push_str(&format!("[{title}](note:{})", title.replace(' ', "%20")));
                rest = &after[end + 2..];
            }
            _ => {
                out.push_str(&rest[..start + 2]);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

pub fn parse(source: &str) -> Vec<Block> {
    let linked = wiki_links(source);
    let source = linked.as_str();
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_GFM
        | Options::ENABLE_FOOTNOTES;
    let line_of = |offset: usize| source[..offset].matches('\n').count();

    let mut blocks: Vec<Block> = Vec::new();
    let mut inline = String::new();
    let mut code = String::new();
    let mut in_code = false;
    let mut quote_depth = 0usize;
    let mut lists: Vec<ListState> = Vec::new();
    // An item whose text hasn't been emitted yet: (line, marker, task state).
    let mut item: Option<(usize, String, Option<bool>)> = None;
    let mut heading: Option<u8> = None;
    let mut row: Vec<String> = Vec::new();
    // URLs of the links being read (they nest only in theory).
    let mut links: Vec<String> = Vec::new();
    let mut block_line = 0usize;
    // Inside a `> [!kind]` callout: its title, and whether the next paragraph is its first.
    let mut callout: Option<&'static str> = None;
    let mut callout_first = false;
    // Inside a footnote definition: its label.
    let mut footnote: Option<String> = None;
    // Language of the code block being read.
    let mut code_lang = String::new();

    // Emit the pending inline text as the right kind of block.
    let flush = |blocks: &mut Vec<Block>,
                     inline: &mut String,
                     item: &mut Option<(usize, String, Option<bool>)>,
                     lists: &[ListState],
                     quote_depth: usize,
                     line: usize| {
        let text = std::mem::take(inline).trim().to_string();
        if let Some((item_line, marker, task)) = item.take() {
            let mut block = Block::new(if task.is_some() { Kind::Task } else { Kind::Item }, text, item_line);
            block.marker = marker;
            block.checked = task.unwrap_or(false);
            block.indent = lists.len().saturating_sub(1) as u8;
            blocks.push(block);
        } else if !text.is_empty() {
            let kind = if quote_depth > 0 { Kind::Quote } else { Kind::Paragraph };
            blocks.push(Block::new(kind, text, line));
        }
    };

    for (event, range) in Parser::new_ext(source, options).into_offset_iter() {
        match event {
            Event::Start(tag) => match tag {
                Tag::Paragraph => block_line = line_of(range.start),
                Tag::Heading { level, .. } => {
                    block_line = line_of(range.start);
                    heading = Some(heading_level(level));
                }
                Tag::BlockQuote(kind) => {
                    quote_depth += 1;
                    if let Some(kind) = kind {
                        callout = Some(match kind {
                            BlockQuoteKind::Note => "Note",
                            BlockQuoteKind::Tip => "Tip",
                            BlockQuoteKind::Important => "Important",
                            BlockQuoteKind::Warning => "Warning",
                            BlockQuoteKind::Caution => "Caution",
                        });
                        callout_first = true;
                    }
                }
                Tag::FootnoteDefinition(label) => footnote = Some(label.to_string()),
                Tag::CodeBlock(kind) => {
                    code_lang = match &kind {
                        CodeBlockKind::Fenced(info) => info.split_whitespace().next().unwrap_or("").to_string(),
                        CodeBlockKind::Indented => String::new(),
                    };
                    block_line = line_of(range.start);
                    in_code = true;
                    code.clear();
                }
                Tag::List(start) => {
                    // A nested list ends the parent item's own text.
                    if item.is_some() {
                        flush(&mut blocks, &mut inline, &mut item, &lists, quote_depth, block_line);
                    }
                    lists.push(ListState { next: start });
                }
                Tag::Item => {
                    let depth = lists.len();
                    let marker = match lists.last_mut().and_then(|l| l.next.as_mut()) {
                        Some(n) => {
                            let marker = format!("{n}.");
                            *n += 1;
                            marker
                        }
                        None => ["•", "◦", "▪"][(depth.saturating_sub(1)) % 3].to_string(),
                    };
                    item = Some((line_of(range.start), marker, None));
                }
                Tag::TableHead | Tag::TableRow => {
                    block_line = line_of(range.start);
                    row.clear();
                }
                Tag::TableCell => inline.clear(),
                Tag::Emphasis => inline.push('*'),
                Tag::Strong => inline.push_str("**"),
                Tag::Strikethrough => inline.push_str("~~"),
                Tag::Link { dest_url, .. } => {
                    links.push(dest_url.to_string());
                    inline.push('[');
                }
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Paragraph => {
                    let before = blocks.len();
                    flush(&mut blocks, &mut inline, &mut item, &lists, quote_depth, block_line);
                    // Turn the new block into a callout or footnote paragraph if we're inside one.
                    if blocks.len() > before {
                        let block = blocks.last_mut().unwrap();
                        if let Some(label) = &footnote {
                            block.kind = Kind::Footnote;
                            block.marker = label.clone();
                        } else if let (Some(title), Kind::Quote) = (callout, block.kind) {
                            block.kind = Kind::Callout;
                            block.marker = if callout_first { title.to_string() } else { String::new() };
                            block.level = match title {
                                "Note" => 1,
                                "Tip" => 2,
                                "Important" => 3,
                                "Warning" => 4,
                                _ => 5,
                            };
                            callout_first = false;
                        }
                    }
                }
                TagEnd::FootnoteDefinition => footnote = None,
                TagEnd::Heading(_) => {
                    let text = std::mem::take(&mut inline).trim().to_string();
                    let mut block = Block::new(Kind::Heading, format!("**{text}**"), block_line);
                    block.level = heading.take().unwrap_or(1);
                    blocks.push(block);
                }
                TagEnd::BlockQuote(_) => {
                    quote_depth = quote_depth.saturating_sub(1);
                    if quote_depth == 0 {
                        callout = None;
                    }
                }
                TagEnd::CodeBlock => {
                    in_code = false;
                    let mut block = Block::new(Kind::Code, code.trim_end_matches('\n').to_string(), block_line);
                    block.marker = std::mem::take(&mut code_lang);
                    blocks.push(block);
                }
                TagEnd::List(_) => {
                    lists.pop();
                }
                TagEnd::Item => {
                    if item.is_some() {
                        flush(&mut blocks, &mut inline, &mut item, &lists, quote_depth, block_line);
                    }
                }
                TagEnd::TableCell => row.push(std::mem::take(&mut inline).trim().to_string()),
                TagEnd::TableHead | TagEnd::TableRow => {
                    let mut block = Block::new(Kind::Row, String::new(), block_line);
                    block.level = matches!(tag, TagEnd::TableHead) as u8;
                    block.cells = std::mem::take(&mut row);
                    blocks.push(block);
                }
                TagEnd::Emphasis => inline.push('*'),
                TagEnd::Strong => inline.push_str("**"),
                TagEnd::Strikethrough => inline.push_str("~~"),
                TagEnd::Link => {
                    let url = links.pop().unwrap_or_default().replace(' ', "%20");
                    inline.push_str(&format!("]({url})"));
                }
                _ => {}
            },
            Event::Text(text) if in_code => code.push_str(&text),
            Event::Text(text) => inline.push_str(&escape(&mark_keywords(&text)).replace("==", MARK)),
            Event::Code(text) => inline.push_str(&format!("`{}`", text.replace('`', "'"))),
            Event::FootnoteReference(label) => inline.push_str(&format!("\\[{label}\\]")),
            Event::InlineHtml(html) | Event::Html(html) => {
                // Underline and colored text are supported by StyledText; drop other tags.
                let tag = html.trim().to_ascii_lowercase();
                if tag.starts_with("<u>") || tag.starts_with("</u>") || tag.starts_with("<font") || tag.starts_with("</font") {
                    inline.push_str(html.trim());
                }
            }
            Event::SoftBreak => inline.push(' '),
            Event::HardBreak => inline.push(' '),
            Event::TaskListMarker(checked) => {
                if let Some((_, _, task)) = item.as_mut() {
                    *task = Some(checked);
                }
            }
            Event::Rule => blocks.push(Block::new(Kind::Rule, String::new(), line_of(range.start))),
            _ => {}
        }
    }

    let mut previous: Option<Kind> = None;
    for block in &mut blocks {
        block.space = space_above(previous, block.kind);
        previous = Some(block.kind);
    }
    blocks
}

/// Vertical rhythm: the space above a block (logical px) depends on what it follows. Headings get
/// more room above than below, so each one sits with the text it introduces.
fn space_above(previous: Option<Kind>, kind: Kind) -> u8 {
    match (previous, kind) {
        (None, _) => 0,
        (_, Kind::Heading) => 24,
        (Some(Kind::Heading), _) => 6,
        (Some(Kind::Item | Kind::Task), Kind::Item | Kind::Task) => 3,
        (Some(Kind::Row), Kind::Row) => 0,
        (Some(Kind::Quote), Kind::Quote) => 4,
        _ => 10,
    }
}

fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

/// Titles of the `[[notes]]` linked in a block's inline Markdown (`[text](note:Some%20title)`), in order,
/// without repeats (ignoring case).
pub fn note_links(inline: &str) -> Vec<String> {
    const MARK: &str = "](note:";
    let mut found: Vec<String> = Vec::new();
    let mut from = 0;
    while let Some(at) = inline[from..].find(MARK) {
        let start = from + at;
        let url = start + MARK.len();
        from = url;
        // An escaped bracket is literal text, not a link.
        if inline[..start].ends_with('\\') {
            continue;
        }
        // The link ends at the matching ")"; titles may hold balanced brackets.
        let mut depth = 0usize;
        let mut end = None;
        for (i, c) in inline[url..].char_indices() {
            match c {
                '(' => depth += 1,
                ')' if depth == 0 => {
                    end = Some(url + i);
                    break;
                }
                ')' => depth -= 1,
                _ => {}
            }
        }
        let Some(end) = end else { break };
        let title = inline[url..end].replace("%20", " ").trim().to_string();
        from = end;
        if !title.is_empty() && !found.iter().any(|t| t.eq_ignore_ascii_case(&title)) {
            found.push(title);
        }
    }
    found
}

/// The first link in a block's Markdown, as the URL to open: `note:Title` for `[[Title]]` (spaces as
/// %20), or the web address. Links inside code are not links, so they are skipped.
pub fn first_link(source: &str) -> Option<String> {
    let linked = wiki_links(source);
    Parser::new(&linked).find_map(|event| match event {
        Event::Start(Tag::Link { dest_url, .. }) if !dest_url.is_empty() => Some(dest_url.to_string()),
        _ => None,
    })
}

/// A line without its Markdown symbols: "# ", "> ", "- [ ] " and the emphasis marks `* _ ~ \``.
pub fn strip_markers(line: &str) -> String {
    let line = line.trim_start_matches(['#', '>', '-', '+', ' ']);
    let line = line.strip_prefix("[ ] ").or_else(|| line.strip_prefix("[x] ")).unwrap_or(line);
    line.chars().filter(|c| !matches!(c, '*' | '_' | '~' | '`')).collect()
}

/// `[[X]]` to `X`, `[t](u)` to `t` and `![a](u)` to `a`.
fn strip_links(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(open) = rest.find('[') {
        let image = rest[..open].ends_with('!');
        let before = &rest[..open - usize::from(image)];
        let after = &rest[open + 1..];
        if let Some(inner) = after.strip_prefix('[').and_then(|a| a.find("]]").map(|e| (&a[..e], &a[e + 2..]))) {
            out.push_str(before);
            out.push_str(inner.0.trim());
            rest = inner.1;
        } else if let Some(close) = after.find("](").filter(|&c| !after[..c].contains('[')) {
            match after[close + 2..].find(')') {
                Some(end) => {
                    out.push_str(before);
                    out.push_str(&after[..close]);
                    rest = &after[close + 2 + end + 1..];
                }
                None => {
                    out.push_str(&rest[..open + 1]);
                    rest = after;
                }
            }
        } else {
            out.push_str(&rest[..open + 1]);
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

/// The first few lines of a note as plain text, for a hover card: no blank lines, code, rules or table
/// separators; Markdown marks removed. A first line equal to `skip` (the title) is dropped.
pub fn plain_snippet(body: &str, skip: &str, max_lines: usize, max_chars: usize) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut fence: Option<&str> = None;
    for raw in body.lines() {
        let trimmed = raw.trim();
        let marker = ["```", "~~~"].into_iter().find(|m| trimmed.starts_with(m));
        match (fence, marker) {
            (Some(open), Some(m)) if open == m => fence = None,
            (Some(_), _) => {}
            (None, Some(m)) => fence = Some(m),
            (None, None) => {
                let rule = trimmed.len() >= 3 && trimmed.chars().all(|c| c == '-');
                let table_separator = trimmed.contains('-') && trimmed.chars().all(|c| matches!(c, '|' | '-' | ':' | ' '));
                if trimmed.is_empty() || rule || table_separator {
                    continue;
                }
                // "#tag" is text; only "# Heading" has a symbol to remove.
                let plain = match trimmed.strip_prefix('#') {
                    Some(tag) if tag.chars().next().is_some_and(|c| c != '#' && c != ' ') => format!("#{}", strip_markers(tag)),
                    _ => strip_markers(trimmed),
                };
                let text = strip_links(&plain).trim().to_string();
                if !text.is_empty() {
                    lines.push(text);
                }
            }
        }
    }
    if lines.first().is_some_and(|first| first.eq_ignore_ascii_case(skip.trim())) {
        lines.remove(0);
    }
    lines.truncate(max_lines);
    let text = lines.join("\n");
    if text.chars().count() <= max_chars {
        return text;
    }
    let mut cut: String = text.chars().take(max_chars).collect();
    cut.truncate(cut.trim_end().len());
    cut.push('\u{2026}');
    cut
}

/// Escape characters that would otherwise be read as Markdown when re-parsed.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(c, '\\' | '`' | '*' | '_' | '~' | '[' | ']' | '<' | '>' | '#' | '|') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<(Kind, String)> {
        parse(src).into_iter().map(|b| (b.kind, b.text)).collect()
    }

    #[test]
    fn headings_and_paragraphs() {
        assert_eq!(
            kinds("# Title\n\nSome *soft* **bold** text\n\n## Sub"),
            vec![
                (Kind::Heading, "**Title**".into()),
                (Kind::Paragraph, "Some *soft* **bold** text".into()),
                (Kind::Heading, "**Sub**".into()),
            ]
        );
        assert_eq!(parse("## Sub")[0].level, 2);
    }

    #[test]
    fn lists_and_tasks() {
        let blocks = parse("- one\n- two\n  - nested\n\n1. a\n2. b\n\n- [ ] todo\n- [x] done");
        let summary: Vec<_> = blocks.iter().map(|b| (b.kind, b.marker.as_str(), b.indent, b.checked, b.text.as_str())).collect();
        assert_eq!(
            summary,
            vec![
                (Kind::Item, "•", 0, false, "one"),
                (Kind::Item, "•", 0, false, "two"),
                (Kind::Item, "◦", 1, false, "nested"),
                (Kind::Item, "1.", 0, false, "a"),
                (Kind::Item, "2.", 0, false, "b"),
                (Kind::Task, "•", 0, false, "todo"),
                (Kind::Task, "•", 0, true, "done"),
            ]
        );
        assert_eq!(blocks[5].line, 7);
    }

    #[test]
    fn code_quote_rule_table() {
        let blocks = parse("> quoted\n\n```\nlet x = 1;\n```\n\n---\n\n| A | B |\n|---|---|\n| 1 | 2 |");
        assert_eq!(blocks[0].kind, Kind::Quote);
        assert_eq!((blocks[1].kind, blocks[1].text.as_str()), (Kind::Code, "let x = 1;"));
        assert_eq!(blocks[2].kind, Kind::Rule);
        assert_eq!((blocks[3].kind, blocks[3].level, blocks[3].cells.clone()), (Kind::Row, 1, vec!["A".to_string(), "B".to_string()]));
        assert_eq!(blocks[4].cells, vec!["1".to_string(), "2".to_string()]);
    }

    #[test]
    fn wiki_links_become_note_links() {
        assert_eq!(kinds("see [[Book ideas]] now")[0].1, "see [Book ideas](note:Book%20ideas) now");
        assert_eq!(parse("- [ ] a\n\n[[x]]\n\n- [ ] b")[2].line, 4);
    }

    #[test]
    fn callouts_footnotes_and_code_language() {
        let blocks = parse("> [!tip]\n> Drink water.\n\nSee this[^1].\n\n[^1]: The source.\n\n```rust\nfn x() {}\n```");
        let summary: Vec<_> = blocks.iter().map(|b| (b.kind, b.marker.as_str(), b.text.as_str())).collect();
        assert_eq!(
            summary,
            vec![
                (Kind::Callout, "Tip", "Drink water."),
                (Kind::Paragraph, "", "See this\\[1\\]."),
                (Kind::Footnote, "1", "The source."),
                (Kind::Code, "rust", "fn x() {}"),
            ]
        );
    }

    #[test]
    fn inline_code_links_and_escapes() {
        assert_eq!(kinds("use `x` and [site](https://a.b)")[0].1, "use `x` and [site](https://a.b)");
        assert_eq!(kinds("2 \\* 3")[0].1, "2 \\* 3");
        assert_eq!(kinds("a <u>b</u>")[0].1, "a <u>b</u>");
    }

    /// The block's text with its `==` marks turned into colour, as Preview shows it.
    fn marked(src: &str) -> String {
        highlights(&parse(src)[0].text, "#f00", false)
    }

    #[test]
    fn highlight_marks_become_colour() {
        assert_eq!(marked("a ==big== day"), "a <font color=\"#f00\">big</font> day");
        assert_eq!(marked("==one== and ==two=="), "<font color=\"#f00\">one</font> and <font color=\"#f00\">two</font>");
        assert_eq!(marked("==a **b** c=="), "<font color=\"#f00\">a **b** c</font>");
        assert_eq!(marked("- [ ] ==soon=="), "<font color=\"#f00\">soon</font>");
        assert_eq!(marked("# ==Title=="), "**<font color=\"#f00\">Title</font>**");
    }

    #[test]
    fn highlight_needs_text_against_its_marks() {
        assert_eq!(marked("if a == b then"), "if a == b then");
        assert_eq!(marked("a == b == c"), "a == b == c");
        assert_eq!(marked("x ==y"), "x ==y");
        assert_eq!(marked("plain"), "plain");
        assert_eq!(marked("a = b"), "a = b");
    }

    #[test]
    fn keywords_are_coloured() {
        let shown = |src: &str| keywords(&parse(src)[0].text, "#00f");
        assert_eq!(shown("pay rent @due fri"), "pay rent <font color=\"#00f\">@due</font> fri");
        assert_eq!(shown("@remind me (@timer 25)"), "<font color=\"#00f\">@remind</font> me (<font color=\"#00f\">@timer</font> 25)");
        assert_eq!(shown("- [ ] rent @due-soon, ok"), "rent <font color=\"#00f\">@due-soon</font>, ok");
    }

    #[test]
    fn keywords_need_a_word_start_and_a_letter() {
        let shown = |src: &str| keywords(&parse(src)[0].text, "#00f");
        assert_eq!(shown("mail me@site.com"), "mail me@site.com");
        assert_eq!(shown("at @5pm or @ noon"), "at @5pm or @ noon");
        assert_eq!(shown("`@due` in code"), "`@due` in code");
        assert_eq!(parse("```\n@due fri\n```")[0].text, "@due fri");
    }

    #[test]
    fn keywords_edges() {
        assert_eq!(mark_keywords(""), "");
        assert_eq!(mark_keywords("@"), "@");
        assert_eq!(plain_marks(&parse("try @due ==now==")[0].text), "try @due ==now==");
        // Together with a highlight, both colours show.
        let both = keywords(&highlights(&parse("==@due fri==")[0].text, "#f00", false), "#00f");
        assert_eq!(both, "<font color=\"#f00\"><font color=\"#00f\">@due</font> fri</font>");
    }

    #[test]
    fn highlight_left_alone_in_code() {
        assert_eq!(marked("`a==b==c`"), "`a==b==c`");
        assert_eq!(parse("```\n==x==\n```")[0].text, "==x==");
    }

    #[test]
    fn highlight_edge_cases() {
        assert_eq!(highlights("", "#f00", false), "");
        assert_eq!(marked("===="), "====");
        assert_eq!(marked("a === b"), "a === b");
        assert_eq!(plain_marks(&parse("==x==")[0].text), "==x==");
    }

    #[test]
    fn named_highlight_colours() {
        assert_eq!(marked("a ==red:big== day"), "a <font color=\"#dc2626\">big</font> day");
        assert_eq!(marked("==Blue:sky=="), "<font color=\"#2563eb\">sky</font>");
        assert_eq!(marked("==green:a **b**== and ==c=="), "<font color=\"#16a34a\">a **b**</font> and <font color=\"#f00\">c</font>");
        assert_eq!(highlights(&parse("==red:hot==")[0].text, "#f00", true), "<font color=\"#f87171\">hot</font>");
    }

    #[test]
    fn unknown_or_empty_colour_names_stay_as_text() {
        assert_eq!(marked("==note: read this=="), "<font color=\"#f00\">note: read this</font>");
        assert_eq!(marked("==red:=="), "<font color=\"#f00\">red:</font>");
        assert_eq!(marked("==time 10:30=="), "<font color=\"#f00\">time 10:30</font>");
        assert_eq!(plain_marks(&parse("==red:x==")[0].text), "==red:x==");
    }

    #[test]
    fn headings_have_more_room_above_than_below() {
        let space: Vec<u8> = parse("intro\n\n# Plan\n\nfirst\n\nsecond\n\n- a\n- b").iter().map(|b| b.space).collect();
        assert_eq!(space, vec![0, 24, 6, 10, 10, 3]);
        // A heading at the very top has no space above it, and two headings in a row keep theirs.
        let space: Vec<u8> = parse("# A\n## B").iter().map(|b| b.space).collect();
        assert_eq!(space, vec![0, 24]);
    }

    #[test]
    fn first_link_wiki_link_opens_the_note() {
        assert_eq!(first_link("Some [[Project Plan]] text"), Some("note:Project%20Plan".into()));
    }

    #[test]
    fn first_link_markdown_link_gives_its_address() {
        assert_eq!(first_link("see [docs](https://x.org/a)"), Some("https://x.org/a".into()));
    }

    #[test]
    fn first_link_autolink() {
        assert_eq!(first_link("<https://x.org>"), Some("https://x.org".into()));
    }

    #[test]
    fn first_link_takes_the_first_of_several() {
        assert_eq!(first_link("[[A]] and [b](https://b.org)"), Some("note:A".into()));
    }

    #[test]
    fn first_link_none_in_plain_text() {
        assert_eq!(first_link("plain text, no links"), None);
        assert_eq!(first_link(""), None);
    }

    #[test]
    fn first_link_ignores_links_in_code() {
        assert_eq!(first_link("`[[Note]]` and `[x](https://x.org)`"), None);
    }

    #[test]
    fn first_link_ignores_empty_targets() {
        assert_eq!(first_link("[empty]() and [[]]"), None);
    }

    #[test]
    fn note_links_found_in_order() {
        assert_eq!(note_links("see [Zed](note:Zed) and [Alpha beta](note:Alpha%20beta)"), vec!["Zed", "Alpha beta"]);
    }

    #[test]
    fn note_links_deduped() {
        assert_eq!(note_links("[a](note:Shop) [b](note:shop) [c](note:Other) [d](note:Shop)"), vec!["Shop", "Other"]);
    }

    #[test]
    fn note_links_ignore_web_and_escaped() {
        assert!(note_links("[site](https://a.b) and \\[x\\](note:Nope)").is_empty());
        assert_eq!(note_links("[x](note:Fine) \\[y\\](note:Nope)"), vec!["Fine"]);
        assert_eq!(note_links("[x](note:Foo%20(bar))"), vec!["Foo (bar)"]);
    }

    #[test]
    fn note_links_empty() {
        assert!(note_links("").is_empty());
        assert!(note_links("plain text").is_empty());
        assert!(note_links("[x](note:)").is_empty());
        assert!(note_links("[x](note:unclosed").is_empty());
    }

    #[test]
    fn note_links_after_parse() {
        assert_eq!(note_links(&parse("see [[Groceries]]")[0].text), vec!["Groceries"]);
        assert_eq!(note_links(&parse("a [[Book ideas]] b")[0].text), vec!["Book ideas"]);
    }

    #[test]
    fn snippet_strips_markdown() {
        assert_eq!(plain_snippet("- [ ] buy **milk**\n> see [[Shop]] and [site](https://a.b)", "", 3, 200), "buy milk\nsee Shop and site");
        assert_eq!(plain_snippet("![a cat](c.png) here", "", 3, 200), "a cat here");
    }

    #[test]
    fn snippet_skips_code_blank_and_rules() {
        let body = "\n```rust\nlet x = 1;\n\n```\n---\n| a | b |\n|---|:-:|\n\nreal text\n~~~\nmore code\n~~~\nlast";
        assert_eq!(plain_snippet(body, "", 5, 200), "| a | b |\nreal text\nlast");
    }

    #[test]
    fn snippet_skips_title_line() {
        assert_eq!(plain_snippet("# Groceries\nmilk", "groceries", 3, 200), "milk");
        // Only the first line, and only when it is the title.
        assert_eq!(plain_snippet("milk\nGroceries", "Groceries", 3, 200), "milk\nGroceries");
    }

    #[test]
    fn snippet_keeps_tags() {
        assert_eq!(plain_snippet("#work idea\n## Heading", "", 3, 200), "#work idea\nHeading");
    }

    #[test]
    fn snippet_limits_lines_and_chars() {
        assert_eq!(plain_snippet("a\nb\nc\nd", "", 2, 200), "a\nb");
        assert_eq!(plain_snippet("abcdefghij", "", 3, 5), "abcde\u{2026}");
        assert_eq!(plain_snippet("abcde", "", 3, 5), "abcde");
        let long = "\u{e9}".repeat(500);
        assert_eq!(plain_snippet(&long, "", 3, 220).chars().count(), 221);
        let emoji = "\u{1F389}".repeat(500);
        assert!(plain_snippet(&emoji, "", 3, 220).ends_with('\u{2026}'));
    }

    #[test]
    fn snippet_empty_body() {
        assert_eq!(plain_snippet("", "x", 3, 200), "");
        assert_eq!(plain_snippet("\n  \n---\n", "x", 3, 200), "");
    }

    #[test]
    fn strip_markers_removes_symbols() {
        assert_eq!(strip_markers("## **Hi** _there_"), "Hi there");
        assert_eq!(strip_markers("- [x] done"), "done");
    }
}
