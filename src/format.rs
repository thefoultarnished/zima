//! Markdown formatting commands (bold, lists, …) applied to the editor text and selection.
//!
//! Offsets are UTF-8 byte offsets, the same as Slint's `TextInput` uses.

/// The result of a formatting command: new text and the new selection.
#[derive(Debug, PartialEq)]
pub struct Edit {
    pub text: String,
    pub anchor: usize,
    pub cursor: usize,
}

pub fn apply(kind: &str, text: &str, anchor: usize, cursor: usize) -> Option<Edit> {
    if anchor > text.len() || cursor > text.len() || !text.is_char_boundary(anchor) || !text.is_char_boundary(cursor) {
        return None;
    }
    let (start, end) = (anchor.min(cursor), anchor.max(cursor));
    Some(match kind {
        "bold" => wrap(text, start, end, "**", "**"),
        "italic" => wrap(text, start, end, "*", "*"),
        "underline" => wrap(text, start, end, "<u>", "</u>"),
        "strike" => wrap(text, start, end, "~~", "~~"),
        "code" => wrap(text, start, end, "`", "`"),
        "heading" => prefix_lines(text, start, end, Prefix::Heading),
        "bullet" => prefix_lines(text, start, end, Prefix::Fixed("- ")),
        "task" => prefix_lines(text, start, end, Prefix::Fixed("- [ ] ")),
        "quote" => prefix_lines(text, start, end, Prefix::Fixed("> ")),
        "number" => prefix_lines(text, start, end, Prefix::Numbered),
        "move-up" => return move_lines(text, anchor, cursor, true),
        "move-down" => return move_lines(text, anchor, cursor, false),
        "duplicate-line" => return duplicate_lines(text, anchor, cursor),
        "highlight" => colour(text, start, end, ""),
        _ => match kind.strip_prefix("colour:") {
            Some(name) if crate::markdown::TEXT_COLOURS.iter().any(|c| c.name == name) => colour(text, start, end, name),
            _ => return None,
        },
    })
}

/// The opening mark for a colour: `==` for the plain highlight, `==red:` for a named one.
fn colour_mark(name: &str) -> String {
    if name.is_empty() { "==".into() } else { format!("=={name}:") }
}

/// Colour the selection with `==name:…==` (`==…==` when `name` is empty). The same colour again
/// takes it off; another colour replaces it.
fn colour(text: &str, start: usize, end: usize, name: &str) -> Edit {
    let selected = &text[start..end];
    let new_open = colour_mark(name);
    // Named colours first: a plain `==` would also match the start of `==red:`.
    let names = crate::markdown::TEXT_COLOURS.iter().map(|c| c.name).chain([""]);
    for old in names {
        let open = colour_mark(old);
        // Marks just outside the selection: ==red:|word|==
        if text[..start].ends_with(&open) && text[end..].starts_with("==") {
            let before = &text[..start - open.len()];
            if old == name {
                let out = format!("{before}{selected}{}", &text[end + 2..]);
                return Edit { text: out, anchor: before.len(), cursor: before.len() + selected.len() };
            }
            let out = format!("{before}{new_open}{selected}{}", &text[end..]);
            let anchor = before.len() + new_open.len();
            return Edit { text: out, anchor, cursor: anchor + selected.len() };
        }
        // Marks inside the selection: |==red:word==|
        if selected.len() >= open.len() + 2 && selected.starts_with(&open) && selected.ends_with("==") {
            let inner = &selected[open.len()..selected.len() - 2];
            if old == name {
                let out = format!("{}{inner}{}", &text[..start], &text[end..]);
                return Edit { text: out, anchor: start, cursor: start + inner.len() };
            }
            let out = format!("{}{new_open}{inner}=={}", &text[..start], &text[end..]);
            return Edit { text: out, anchor: start, cursor: start + new_open.len() + inner.len() + 2 };
        }
    }
    wrap(text, start, end, &new_open, "==")
}

/// Surround the selection with markers, or remove them if they're already there.
fn wrap(text: &str, start: usize, end: usize, open: &str, close: &str) -> Edit {
    let selected = &text[start..end];

    // Markers just outside the selection: **|word|**
    if text[..start].ends_with(open) && text[end..].starts_with(close) {
        let mut out = String::with_capacity(text.len());
        out.push_str(&text[..start - open.len()]);
        out.push_str(selected);
        out.push_str(&text[end + close.len()..]);
        return Edit { text: out, anchor: start - open.len(), cursor: end - open.len() };
    }
    // Markers inside the selection: |**word**|
    if selected.len() >= open.len() + close.len() && selected.starts_with(open) && selected.ends_with(close) {
        let inner = &selected[open.len()..selected.len() - close.len()];
        let out = format!("{}{inner}{}", &text[..start], &text[end..]);
        return Edit { text: out, anchor: start, cursor: start + inner.len() };
    }
    let out = format!("{}{open}{selected}{close}{}", &text[..start], &text[end..]);
    Edit { text: out, anchor: start + open.len(), cursor: end + open.len() }
}

enum Prefix {
    Fixed(&'static str),
    Numbered,
    Heading,
}

/// Add a prefix to every line touched by the selection, or remove it if all lines already have it.
fn prefix_lines(text: &str, start: usize, end: usize, prefix: Prefix) -> Edit {
    let first = text[..start].rfind('\n').map_or(0, |i| i + 1);
    let last = text[end..].find('\n').map_or(text.len(), |i| end + i);
    let lines: Vec<&str> = text[first..last].split('\n').collect();

    let existing = |line: &str| -> Option<usize> {
        match prefix {
            Prefix::Fixed(p) => line.starts_with(p).then_some(p.len()),
            Prefix::Numbered => {
                let digits = line.bytes().take_while(u8::is_ascii_digit).count();
                (digits > 0 && line[digits..].starts_with(". ")).then_some(digits + 2)
            }
            Prefix::Heading => {
                let hashes = line.bytes().take_while(|&b| b == b'#').count();
                (hashes > 0 && line[hashes..].starts_with(' ')).then_some(hashes + 1)
            }
        }
    };
    let remove = lines.iter().all(|line| existing(line).is_some());

    let new_lines: Vec<String> = lines
        .iter()
        .enumerate()
        .map(|(i, line)| match (remove, existing(line)) {
            (true, Some(len)) => line[len..].to_string(),
            _ => match prefix {
                Prefix::Fixed(p) => format!("{p}{line}"),
                Prefix::Numbered => format!("{}. {line}", i + 1),
                Prefix::Heading => format!("# {line}"),
            },
        })
        .collect();
    let block = new_lines.join("\n");
    let out = format!("{}{block}{}", &text[..first], &text[last..]);

    // No selection on one line: keep the cursor at the same spot in the text, shifted by the prefix.
    if start == end && lines.len() == 1 {
        let shifted = start as isize + block.len() as isize - lines[0].len() as isize;
        let cursor = (shifted.max(first as isize) as usize).min(first + block.len());
        return Edit { text: out, anchor: cursor, cursor };
    }
    Edit { text: out, anchor: first, cursor: first + block.len() }
}

/// The list marker at the start of a line: (indent, marker, rest). Markers: `- [ ] `, `- `, `3. `, `> `.
fn list_marker(line: &str) -> Option<(&str, String, &str)> {
    let indent_len = line.len() - line.trim_start_matches(' ').len();
    let (indent, rest) = line.split_at(indent_len);
    for bullet in ["- ", "* ", "+ "] {
        if let Some(after) = rest.strip_prefix(bullet) {
            for task in ["[ ] ", "[x] ", "[X] "] {
                if let Some(after_task) = after.strip_prefix(task) {
                    return Some((indent, format!("{bullet}{task}"), after_task));
                }
            }
            return Some((indent, bullet.to_string(), after));
        }
    }
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if digits > 0 && rest[digits..].starts_with(". ") {
        return Some((indent, rest[..digits + 2].to_string(), &rest[digits + 2..]));
    }
    rest.strip_prefix("> ").map(|after| (indent, "> ".to_string(), after))
}

/// Smart lists. Call right after Enter inserted a newline before `cursor`:
/// continue the list on the new line, or end it if the item was empty.
pub fn continue_list(text: &str, cursor: usize) -> Option<Edit> {
    if cursor == 0 || !text.is_char_boundary(cursor) || text.as_bytes()[cursor - 1] != b'\n' {
        return None;
    }
    let line_end = cursor - 1;
    let line_start = text[..line_end].rfind('\n').map_or(0, |i| i + 1);
    let (indent, marker, rest) = list_marker(&text[line_start..line_end])?;

    if rest.trim().is_empty() {
        // Enter on an empty item ends the list: drop the marker and the new line.
        let out = format!("{}{}", &text[..line_start], &text[cursor..]);
        return Some(Edit { text: out, anchor: line_start, cursor: line_start });
    }
    let next = if marker.ends_with("] ") {
        format!("{}[ ] ", &marker[..2]) // a new task starts unchecked
    } else if marker.as_bytes()[0].is_ascii_digit() {
        let n: u64 = marker.trim_end_matches(". ").parse().ok()?;
        format!("{}. ", n + 1)
    } else {
        marker
    };
    let insert = format!("{indent}{next}");
    let out = format!("{}{insert}{}", &text[..cursor], &text[cursor..]);
    let at = cursor + insert.len();
    Some(Edit { text: out, anchor: at, cursor: at })
}

/// Tab / Shift+Tab: indent or outdent every selected line by two spaces.
pub fn indent(text: &str, anchor: usize, cursor: usize, outdent: bool) -> Option<Edit> {
    if anchor > text.len() || cursor > text.len() {
        return None;
    }
    let (start, end) = (anchor.min(cursor), anchor.max(cursor));
    let first = text[..start].rfind('\n').map_or(0, |i| i + 1);
    let last = text[end..].find('\n').map_or(text.len(), |i| end + i);
    let mut removed_first = 0usize;
    let lines: Vec<String> = text[first..last]
        .split('\n')
        .enumerate()
        .map(|(i, line)| {
            if outdent {
                let n = line.len() - line.trim_start_matches(' ').len();
                let n = n.min(2);
                if i == 0 {
                    removed_first = n;
                }
                line[n..].to_string()
            } else {
                format!("  {line}")
            }
        })
        .collect();
    let block = lines.join("\n");
    let out = format!("{}{block}{}", &text[..first], &text[last..]);
    if start == end {
        let at = if outdent { start.saturating_sub(removed_first).max(first) } else { start + 2 };
        return Some(Edit { text: out, anchor: at, cursor: at });
    }
    Some(Edit { text: out, anchor: first, cursor: first + block.len() })
}

/// Typing a bracket or backtick: wrap the selection, insert the pair, or step over an existing closer.
/// `None` means "let the text field insert the character normally".
pub fn auto_pair(text: &str, anchor: usize, cursor: usize, typed: &str) -> Option<Edit> {
    if anchor > text.len() || cursor > text.len() {
        return None;
    }
    let (start, end) = (anchor.min(cursor), anchor.max(cursor));
    let closer = match typed {
        "(" => Some(")"),
        "[" => Some("]"),
        "`" => Some("`"),
        "*" | "_" => None,
        ")" | "]" => {
            // Step over a closer that's already there.
            return (start == end && text[end..].starts_with(typed)).then(|| Edit { text: text.to_string(), anchor: end + 1, cursor: end + 1 });
        }
        _ => return None,
    };
    if start != end {
        let close = closer.unwrap_or(typed);
        let out = format!("{}{typed}{}{close}{}", &text[..start], &text[start..end], &text[end..]);
        return Some(Edit { text: out, anchor: start + 1, cursor: end + 1 });
    }
    let close = closer?;
    // Backtick: step over an existing one instead of doubling it.
    if typed == "`" && text[end..].starts_with('`') {
        return Some(Edit { text: text.to_string(), anchor: end + 1, cursor: end + 1 });
    }
    // Only pair before whitespace / end of text, so typing in the middle of words stays normal.
    let next = text[end..].chars().next();
    if next.is_some_and(|c| !c.is_whitespace() && !")]".contains(c)) {
        return None;
    }
    let out = format!("{}{typed}{close}{}", &text[..start], &text[end..]);
    Some(Edit { text: out, anchor: start + 1, cursor: start + 1 })
}

/// Pasting a URL over selected text makes a Markdown link.
pub fn paste_link(text: &str, anchor: usize, cursor: usize, clipboard: &str) -> Option<Edit> {
    let url = clipboard.trim();
    let is_url = (url.starts_with("http://") || url.starts_with("https://")) && !url.contains(char::is_whitespace);
    let (start, end) = (anchor.min(cursor), anchor.max(cursor));
    if !is_url || start == end || end > text.len() {
        return None;
    }
    let label = &text[start..end];
    let link = format!("[{label}]({url})");
    let out = format!("{}{link}{}", &text[..start], &text[end..]);
    let at = start + link.len();
    Some(Edit { text: out, anchor: at, cursor: at })
}

/// Byte ranges of each line (without the newline).
fn line_ranges(text: &str) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut start = 0;
    for (i, b) in text.bytes().enumerate() {
        if b == b'\n' {
            ranges.push((start, i));
            start = i + 1;
        }
    }
    ranges.push((start, text.len()));
    ranges
}

fn line_of(ranges: &[(usize, usize)], offset: usize) -> usize {
    ranges.iter().position(|&(s, e)| offset >= s && offset <= e).unwrap_or(ranges.len() - 1)
}

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start_matches(' ').len()
}

/// End of a selection for line operations: a selection ending at the very start of a line
/// doesn't include that line (as in other editors).
fn selection_end(text: &str, anchor: usize, cursor: usize) -> usize {
    let (start, end) = (anchor.min(cursor), anchor.max(cursor));
    if end > start && text[..end].ends_with('\n') { end - 1 } else { end }
}

/// Alt+↑ / Alt+↓: move the selected lines (plus any more-indented lines under them) past the neighbouring block.
pub fn move_lines(text: &str, anchor: usize, cursor: usize, up: bool) -> Option<Edit> {
    let ranges = line_ranges(text);
    let lines: Vec<&str> = ranges.iter().map(|&(s, e)| &text[s..e]).collect();
    let (start, end) = (anchor.min(cursor), selection_end(text, anchor, cursor));
    let first = line_of(&ranges, start);
    let mut last = line_of(&ranges, end);
    // Children: following lines indented deeper than the first selected line.
    let base = indent_of(lines[first]);
    while last + 1 < lines.len() && !lines[last + 1].trim().is_empty() && indent_of(lines[last + 1]) > base {
        last += 1;
    }

    // The neighbouring block to swap with.
    let (block_start, block_end) = if up {
        if first == 0 {
            return None;
        }
        let mut s = first - 1;
        while s > 0 && indent_of(lines[s]) > base && !lines[s].trim().is_empty() {
            s -= 1;
        }
        (s, first - 1)
    } else {
        if last + 1 >= lines.len() {
            return None;
        }
        let s = last + 1;
        let mut e = s;
        let neighbour_indent = indent_of(lines[s]);
        while e + 1 < lines.len() && !lines[e + 1].trim().is_empty() && indent_of(lines[e + 1]) > neighbour_indent {
            e += 1;
        }
        (s, e)
    };

    let moving: Vec<&str> = lines[first..=last].to_vec();
    let other: Vec<&str> = lines[block_start..=block_end].to_vec();
    let mut new_lines: Vec<&str> = Vec::with_capacity(lines.len());
    let moved_first;
    if up {
        new_lines.extend(&lines[..block_start]);
        moved_first = new_lines.len();
        new_lines.extend(&moving);
        new_lines.extend(&other);
        new_lines.extend(&lines[last + 1..]);
    } else {
        new_lines.extend(&lines[..first]);
        new_lines.extend(&other);
        moved_first = new_lines.len();
        new_lines.extend(&moving);
        new_lines.extend(&lines[block_end + 1..]);
    }
    let out = new_lines.join("\n");
    // Keep the selection on the moved text, same columns.
    let new_start_of = |line: usize| new_lines[..line].iter().map(|l| l.len() + 1).sum::<usize>();
    let shift = new_start_of(moved_first) as isize - ranges[first].0 as isize;
    let anchor = (anchor as isize + shift) as usize;
    let cursor = (cursor as isize + shift) as usize;
    Some(Edit { text: out, anchor, cursor })
}

/// Ctrl+Shift+D: duplicate the selected lines below themselves; the selection moves to the copy.
pub fn duplicate_lines(text: &str, anchor: usize, cursor: usize) -> Option<Edit> {
    let ranges = line_ranges(text);
    let (start, end) = (anchor.min(cursor), selection_end(text, anchor, cursor));
    let (first, last) = (line_of(&ranges, start), line_of(&ranges, end));
    let (block_start, block_end) = (ranges[first].0, ranges[last].1);
    let block = &text[block_start..block_end];
    let out = format!("{}\n{block}{}", &text[..block_end], &text[block_end..]);
    let shift = block.len() + 1;
    Some(Edit { text: out, anchor: anchor + shift, cursor: cursor + shift })
}

/// The Markdown table around `offset`: its line range and cells (trimmed), or `None`.
fn table_at(text: &str, offset: usize) -> Option<(usize, usize, Vec<Vec<String>>)> {
    let ranges = line_ranges(text);
    let lines: Vec<&str> = ranges.iter().map(|&(s, e)| &text[s..e]).collect();
    let here = line_of(&ranges, offset);
    let is_row = |l: &str| l.trim_start().starts_with('|');
    if !is_row(lines[here]) {
        return None;
    }
    let mut first = here;
    while first > 0 && is_row(lines[first - 1]) {
        first -= 1;
    }
    let mut last = here;
    while last + 1 < lines.len() && is_row(lines[last + 1]) {
        last += 1;
    }
    let rows = lines[first..=last]
        .iter()
        .map(|l| {
            let inner = l.trim().trim_start_matches('|');
            let inner = inner.strip_suffix('|').unwrap_or(inner);
            inner.split('|').map(|c| c.trim().to_string()).collect()
        })
        .collect();
    Some((first, last, rows))
}

fn is_separator(row: &[String]) -> bool {
    !row.is_empty() && row.iter().all(|c| !c.is_empty() && c.chars().all(|ch| ch == '-' || ch == ':'))
}

/// Render rows with aligned columns. Returns the text and each cell's content start offset (row, col).
fn render_table(rows: &[Vec<String>]) -> (String, Vec<Vec<usize>>) {
    let cols = rows.iter().map(|r| r.len()).max().unwrap_or(1).max(1);
    let widths: Vec<usize> = (0..cols)
        .map(|c| {
            rows.iter()
                .filter(|r| !is_separator(r))
                .map(|r| r.get(c).map_or(0, |s| s.chars().count()))
                .max()
                .unwrap_or(0)
                .max(3)
        })
        .collect();
    let mut out = String::new();
    let mut starts = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let mut row_starts = Vec::new();
        out.push('|');
        for (c, width) in widths.iter().enumerate() {
            out.push(' ');
            row_starts.push(out.len());
            if is_separator(row) {
                out.push_str(&"-".repeat(*width));
            } else {
                let cell = row.get(c).map_or("", |s| s.as_str());
                out.push_str(cell);
                out.push_str(&" ".repeat(width - cell.chars().count()));
            }
            out.push_str(" |");
        }
        starts.push(row_starts);
    }
    (out, starts)
}

/// Tab / Shift+Tab inside a table: align the columns and move to the next / previous cell
/// (Tab in the last cell adds a row). `None` if the cursor isn't in a table.
pub fn table_tab(text: &str, cursor: usize, back: bool) -> Option<Edit> {
    let (first, last, mut rows) = table_at(text, cursor)?;
    let ranges = line_ranges(text);
    let row = line_of(&ranges, cursor) - first;
    let col = text[ranges[first + row].0..cursor].matches('|').count().saturating_sub(1);

    // Next/previous cell, skipping the separator row.
    let cols = rows.iter().map(|r| r.len()).max().unwrap_or(1);
    let mut target = (row, col);
    loop {
        let (r, c) = target;
        target = if back {
            if c > 0 { (r, c - 1) } else if r > 0 { (r - 1, cols - 1) } else { return None }
        } else if c + 1 < cols {
            (r, c + 1)
        } else {
            (r + 1, 0)
        };
        if target.0 >= rows.len() {
            rows.push(vec![String::new(); cols]);
        }
        if !is_separator(&rows[target.0]) {
            break;
        }
    }
    let (table, starts) = render_table(&rows);
    let at = ranges[first].0 + starts[target.0][target.1];
    let out = format!("{}{table}{}", &text[..ranges[first].0], &text[ranges[last].1..]);
    // Select the cell's text so typing replaces it.
    let cell_len = rows[target.0].get(target.1).map_or(0, |s| s.len());
    Some(Edit { text: out, anchor: at, cursor: at + cell_len })
}

/// Enter at the end of a table row: align and add an empty row below, cursor in its first cell.
pub fn table_enter(text: &str, cursor: usize) -> Option<Edit> {
    let ranges = line_ranges(text);
    let line = line_of(&ranges, cursor);
    if text[cursor..ranges[line].1].trim() != "" {
        return None; // only at the end of a row
    }
    let (first, last, mut rows) = table_at(text, cursor)?;
    let cols = rows.iter().map(|r| r.len()).max().unwrap_or(1);
    let row = line - first;
    rows.insert(row + 1, vec![String::new(); cols]);
    let (table, starts) = render_table(&rows);
    let at = ranges[first].0 + starts[row + 1][0];
    let out = format!("{}{table}{}", &text[..ranges[first].0], &text[ranges[last].1..]);
    Some(Edit { text: out, anchor: at, cursor: at })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(kind: &str, text: &str, anchor: usize, cursor: usize) -> Edit {
        apply(kind, text, anchor, cursor).unwrap()
    }

    #[test]
    fn colour_wraps_and_unwraps() {
        let e = run("colour:red", "a hot day", 2, 5);
        assert_eq!(e, Edit { text: "a ==red:hot== day".into(), anchor: 8, cursor: 11 });
        // The same colour again takes it off.
        let e = run("colour:red", &e.text, e.anchor, e.cursor);
        assert_eq!(e, Edit { text: "a hot day".into(), anchor: 2, cursor: 5 });
        let e = run("highlight", "a hot day", 2, 5);
        assert_eq!(e, Edit { text: "a ==hot== day".into(), anchor: 4, cursor: 7 });
    }

    #[test]
    fn colour_replaces_another_colour() {
        // Marks outside the selection.
        let e = run("colour:blue", "a ==red:hot== day", 8, 11);
        assert_eq!(e, Edit { text: "a ==blue:hot== day".into(), anchor: 9, cursor: 12 });
        let e = run("highlight", &e.text, e.anchor, e.cursor);
        assert_eq!(e, Edit { text: "a ==hot== day".into(), anchor: 4, cursor: 7 });
        let e = run("colour:green", &e.text, e.anchor, e.cursor);
        assert_eq!(e, Edit { text: "a ==green:hot== day".into(), anchor: 10, cursor: 13 });
        // Marks inside the selection.
        let e = run("colour:pink", "a ==red:hot== day", 2, 13);
        assert_eq!(e, Edit { text: "a ==pink:hot== day".into(), anchor: 2, cursor: 14 });
        let e = run("colour:pink", &e.text, e.anchor, e.cursor);
        assert_eq!(e, Edit { text: "a hot day".into(), anchor: 2, cursor: 5 });
        let e = run("colour:red", "a ==hot== day", 2, 9);
        assert_eq!(e, Edit { text: "a ==red:hot== day".into(), anchor: 2, cursor: 13 });
    }

    #[test]
    fn colour_edges() {
        // No selection: empty marks with the cursor between them, ready to type.
        assert_eq!(run("colour:red", "ab", 1, 1), Edit { text: "a==red:==b".into(), anchor: 7, cursor: 7 });
        assert_eq!(run("colour:red", "", 0, 0), Edit { text: "==red:==".into(), anchor: 6, cursor: 6 });
        // Unknown colours do nothing.
        assert_eq!(apply("colour:teal", "abc", 0, 3), None);
        assert_eq!(apply("colour:", "abc", 0, 3), None);
        // Bold marks around the selection aren't mistaken for colour.
        assert_eq!(run("colour:red", "**hot**", 2, 5).text, "**==red:hot==**");
    }

    #[test]
    fn bold_wraps_and_unwraps() {
        let e = run("bold", "make this bold", 5, 9);
        assert_eq!(e, Edit { text: "make **this** bold".into(), anchor: 7, cursor: 11 });
        // Same selection again removes it.
        let e = run("bold", &e.text, e.anchor, e.cursor);
        assert_eq!(e, Edit { text: "make this bold".into(), anchor: 5, cursor: 9 });
        // Selecting the markers too also removes them.
        let e = run("bold", "make **this** bold", 5, 13);
        assert_eq!(e.text, "make this bold");
    }

    #[test]
    fn empty_selection_inserts_markers() {
        let e = run("italic", "ab", 1, 1);
        assert_eq!(e, Edit { text: "a**b".into(), anchor: 2, cursor: 2 });
        let e = run("underline", "", 0, 0);
        assert_eq!(e, Edit { text: "<u></u>".into(), anchor: 3, cursor: 3 });
    }

    #[test]
    fn lists() {
        let e = run("bullet", "one\ntwo\nthree", 0, 7);
        assert_eq!(e.text, "- one\n- two\nthree");
        let e = run("bullet", &e.text, e.anchor, e.cursor);
        assert_eq!(e.text, "one\ntwo\nthree");
        let e = run("number", "a\nb\nc", 0, 5);
        assert_eq!(e.text, "1. a\n2. b\n3. c");
        let e = run("number", &e.text, 0, e.text.len());
        assert_eq!(e.text, "a\nb\nc");
    }

    #[test]
    fn prefix_on_empty_line_puts_cursor_after_it() {
        let e = run("task", "x\n", 2, 2);
        assert_eq!(e, Edit { text: "x\n- [ ] ".into(), anchor: 8, cursor: 8 });
        let e = run("heading", "Title", 5, 5);
        assert_eq!(e, Edit { text: "# Title".into(), anchor: 7, cursor: 7 });
        let e = run("heading", "# Title", 7, 7);
        assert_eq!(e, Edit { text: "Title".into(), anchor: 5, cursor: 5 });
    }

    #[test]
    fn smart_lists() {
        let e = continue_list("- milk\n", 7).unwrap();
        assert_eq!(e, Edit { text: "- milk\n- ".into(), anchor: 9, cursor: 9 });
        let e = continue_list("  3. c\n", 7).unwrap();
        assert_eq!(e.text, "  3. c\n  4. ");
        let e = continue_list("- [x] done\n", 11).unwrap();
        assert_eq!(e.text, "- [x] done\n- [ ] ");
        let e = continue_list("> quote\n", 8).unwrap();
        assert_eq!(e.text, "> quote\n> ");
        // Empty item ends the list.
        let e = continue_list("- a\n- \n", 7).unwrap();
        assert_eq!(e, Edit { text: "- a\n".into(), anchor: 4, cursor: 4 });
        assert!(continue_list("plain\n", 6).is_none());
    }

    #[test]
    fn indenting() {
        let e = indent("- a\n- b", 5, 5, false).unwrap();
        assert_eq!(e, Edit { text: "- a\n  - b".into(), anchor: 7, cursor: 7 });
        let e = indent(&e.text, 7, 7, true).unwrap();
        assert_eq!(e, Edit { text: "- a\n- b".into(), anchor: 5, cursor: 5 });
        let e = indent("x\ny", 0, 3, false).unwrap();
        assert_eq!(e.text, "  x\n  y");
    }

    #[test]
    fn pairs() {
        assert_eq!(auto_pair("ab ", 3, 3, "(").unwrap(), Edit { text: "ab ()".into(), anchor: 4, cursor: 4 });
        assert_eq!(auto_pair("ab", 0, 2, "*").unwrap(), Edit { text: "*ab*".into(), anchor: 1, cursor: 3 });
        assert_eq!(auto_pair("ab", 0, 2, "[").unwrap().text, "[ab]");
        assert!(auto_pair("ab", 0, 0, "(").is_none()); // before a word: normal typing
        assert!(auto_pair("ab", 1, 1, "*").is_none());
        assert_eq!(auto_pair("()", 1, 1, ")").unwrap(), Edit { text: "()".into(), anchor: 2, cursor: 2 });
        assert!(auto_pair("(x", 2, 2, ")").is_none());
    }

    #[test]
    fn links() {
        let e = paste_link("see docs", 4, 8, "https://a.b/c").unwrap();
        assert_eq!(e.text, "see [docs](https://a.b/c)");
        assert!(paste_link("see docs", 4, 8, "not a url").is_none());
        assert!(paste_link("see docs", 4, 4, "https://a.b").is_none());
    }

    #[test]
    fn moving_lines() {
        let e = move_lines("a\nb\nc", 2, 2, true).unwrap();
        assert_eq!(e, Edit { text: "b\na\nc".into(), anchor: 0, cursor: 0 });
        let e = move_lines("a\nb\nc", 2, 2, false).unwrap();
        assert_eq!(e, Edit { text: "a\nc\nb".into(), anchor: 4, cursor: 4 });
        assert!(move_lines("a\nb", 0, 0, true).is_none());
        // A list item carries its children; it swaps with the whole sibling block.
        let text = "- one\n  - child\n- two\n  - kid";
        let e = move_lines(text, 0, 0, false).unwrap();
        assert_eq!(e.text, "- two\n  - kid\n- one\n  - child");
        let e = move_lines(&e.text, e.anchor, e.cursor, true).unwrap();
        assert_eq!(e.text, text);
    }

    #[test]
    fn selection_ending_at_line_start_skips_that_line() {
        // "second line" selected from its start to the start of the empty line after it.
        let text = "alpha\nfirst line\nsecond line\n";
        let e = move_lines(text, 29, 17, true).unwrap();
        assert_eq!(e.text, "alpha\nsecond line\nfirst line\n");
    }

    #[test]
    fn duplicating() {
        let e = duplicate_lines("a\nb", 0, 1).unwrap();
        assert_eq!(e, Edit { text: "a\na\nb".into(), anchor: 2, cursor: 3 });
    }

    #[test]
    fn tables() {
        let text = "| A | B |\n|---|---|\n| x | yy |";
        // Tab from A moves to B (and aligns).
        let e = table_tab(text, 2, false).unwrap();
        assert_eq!(e.text, "| A   | B   |\n| --- | --- |\n| x   | yy  |");
        assert_eq!(&e.text[e.anchor..e.cursor], "B");
        // Tab from B skips the separator and lands on x.
        let e = table_tab(&e.text, e.anchor, false).unwrap();
        assert_eq!(&e.text[e.anchor..e.cursor], "x");
        // Tab from the last cell adds a row.
        let last = e.text.rfind("yy").unwrap();
        let e = table_tab(&e.text, last, false).unwrap();
        assert_eq!(e.text.lines().count(), 4);
        // Enter at the end of a row inserts one below.
        let e = table_enter("| A | B |\n|---|---|\n| x | y |", 29).unwrap();
        assert_eq!(e.text.lines().count(), 4);
        assert!(table_tab("not a table", 2, false).is_none());
    }

}

