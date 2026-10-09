//! `@` commands: suggestions while typing, and `@table`.

pub struct Command {
    pub name: &'static str,
    pub hint: &'static str,
}

pub const COMMANDS: &[Command] = &[
    Command { name: "remind", hint: "me to stretch in 20 min" },
    Command { name: "due", hint: "rent tomorrow  (adds a task)" },
    Command { name: "table", hint: "3,4  (rows, columns)" },
    Command { name: "calc", hint: "12*3.5 + 8" },
    Command { name: "time", hint: "3pm IST to PST  (or India to Estonia)" },
    Command { name: "curr", hint: "100 usd to inr  (rates by exchangerate-api.com)" },
    Command { name: "timer", hint: "25  (minutes of focus)" },
    Command { name: "goal", hint: "500  (words for this note)" },
    Command { name: "today", hint: "insert today's date" },
    Command { name: "tomorrow", hint: "insert tomorrow's date" },
    Command { name: "now", hint: "insert the time" },
];

/// Commands that are replaced by their value right away instead of taking arguments.
pub fn expand(name: &str, now: chrono::DateTime<chrono::Local>) -> Option<String> {
    Some(match name {
        "today" => now.format("%a %-d %b %Y").to_string(),
        "tomorrow" => (now + chrono::Duration::days(1)).format("%a %-d %b %Y").to_string(),
        "now" => now.format("%-I:%M %p").to_string(),
        _ => return None,
    })
}

/// If the text just before `end` is `@today` / `@tomorrow` / `@now` (at a word start), its start offset and name.
pub fn date_token_before(text: &str, end: usize) -> Option<(usize, &'static str)> {
    let before = text.get(..end)?;
    for name in ["today", "tomorrow", "now"] {
        let token = format!("@{name}");
        if before.ends_with(&token) {
            let start = end - token.len();
            if before[..start].chars().next_back().is_none_or(char::is_whitespace) {
                return Some((start, name));
            }
        }
    }
    None
}

/// Strip `@name` from the start of a line; `None` if the line isn't that command.
fn command_args<'a>(line: &'a str, name: &str) -> Option<&'a str> {
    let rest = line.trim_start().strip_prefix('@')?.strip_prefix(name)?;
    (rest.is_empty() || rest.starts_with(char::is_whitespace)).then(|| rest.trim())
}

/// `@calc 12*3`: the line to replace it with ("12*3 = 36").
pub fn parse_calc(line: &str) -> Option<Result<String, ()>> {
    let expr = command_args(line, "calc")?;
    Some(crate::calc::evaluate(expr).map(|v| format!("{expr} = {}", crate::calc::format_number(v))).ok_or(()))
}

/// `@time 3pm IST to PST`: the line to replace it with ("3:00 PM IST = 2:30 AM PDT").
pub fn parse_time(line: &str, now: chrono::DateTime<chrono::Utc>) -> Option<Result<String, ()>> {
    let request = command_args(line, "time")?;
    Some(crate::timezones::convert(request, now, crate::timezones::Zone::Here).ok_or(()))
}

/// `@curr 100 usd to inr` or `@currency …`: what's asked (converted in `crate::currency`).
pub fn currency_request(line: &str) -> Option<&str> {
    command_args(line, "curr").or_else(|| command_args(line, "currency"))
}

/// `@goal 500` sets a word goal; `@goal off` (or 0) clears it.
pub fn parse_goal(line: &str) -> Option<Result<Option<u32>, ()>> {
    let arg = command_args(line, "goal")?;
    Some(match arg {
        "off" | "none" | "0" => Ok(None),
        n => n.parse::<u32>().map(Some).map_err(|_| ()),
    })
}

/// `@timer 25` (minutes; default 25), `@timer stop`. `Some(Ok(None))` = stop.
pub fn parse_timer(line: &str) -> Option<Result<Option<u32>, ()>> {
    let arg = command_args(line, "timer")?;
    Some(match arg {
        "" => Ok(Some(25)),
        "stop" | "off" => Ok(None),
        n => n.trim_end_matches("min").trim().parse::<u32>().ok().filter(|m| (1..=240).contains(m)).map(Some).ok_or(()),
    })
}

/// An entry in the `/` menu: what to insert, and where the cursor goes (byte offset into `insert`).
pub struct SlashItem {
    pub name: &'static str,
    pub hint: &'static str,
    pub insert: &'static str,
    pub cursor: usize,
}

pub const SLASH: &[SlashItem] = &[
    SlashItem { name: "heading", hint: "Big section heading", insert: "# ", cursor: 2 },
    SlashItem { name: "subheading", hint: "Smaller heading", insert: "## ", cursor: 3 },
    SlashItem { name: "bullet", hint: "Bulleted list", insert: "- ", cursor: 2 },
    SlashItem { name: "numbered", hint: "Numbered list", insert: "1. ", cursor: 3 },
    SlashItem { name: "checklist", hint: "To-do with checkboxes", insert: "- [ ] ", cursor: 6 },
    SlashItem { name: "table", hint: "2 \u{00d7} 2 table", insert: "| Column 1 | Column 2 |\n| --- | --- |\n|   |   |\n", cursor: 2 },
    SlashItem { name: "quote", hint: "Quotation", insert: "> ", cursor: 2 },
    SlashItem { name: "callout", hint: "Highlighted tip box", insert: "> [!tip]\n> ", cursor: 11 },
    SlashItem { name: "code", hint: "Code block", insert: "```\n\n```", cursor: 4 },
    SlashItem { name: "divider", hint: "Horizontal line", insert: "---\n", cursor: 4 },
    SlashItem { name: "date", hint: "Today's date", insert: "", cursor: 0 },
];

/// If the cursor is right after `/abc` at the start of a line, the `/` offset and matching items.
pub fn slash_suggestions(text: &str, cursor: usize) -> Option<(usize, Vec<&'static SlashItem>)> {
    let before = text.get(..cursor)?;
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    let line = &before[line_start..];
    let slash = line.find('/')?;
    if !line[..slash].trim().is_empty() {
        return None;
    }
    let query = line[slash + 1..].to_ascii_lowercase();
    if !query.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    // Match the name, or the start of a word in the hint ("/line" finds the divider).
    let matches: Vec<_> = SLASH
        .iter()
        .filter(|s| s.name.starts_with(&query) || s.hint.to_lowercase().split_whitespace().any(|w| w.starts_with(&query)))
        .collect();
    (!matches.is_empty()).then_some((line_start + slash, matches))
}

/// If the cursor is right after `@abc` (at a word start), returns the `@` offset and the commands matching `abc`.
pub fn suggestions(text: &str, cursor: usize) -> Option<(usize, Vec<&'static Command>)> {
    let before = text.get(..cursor)?;
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    let at = before[line_start..].rfind('@')? + line_start;
    let query = &before[at + 1..];
    if !query.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    // "@" must start a word, so emails like a@b don't trigger it.
    if before[..at].chars().next_back().is_some_and(|c| !c.is_whitespace()) {
        return None;
    }
    let query = query.to_ascii_lowercase();
    let matches: Vec<_> = COMMANDS.iter().filter(|c| c.name.starts_with(&query) && c.name != query).collect();
    (!matches.is_empty()).then_some((at, matches))
}

/// Parse `@table 3,4` (also `3x4` or `3 4`): data rows and columns.
pub fn parse_table(line: &str) -> Option<Result<(usize, usize), ()>> {
    let rest = line.trim_start().strip_prefix("@table")?;
    if !(rest.is_empty() || rest.starts_with(char::is_whitespace)) {
        return None;
    }
    let numbers: Vec<usize> = rest
        .split(|c: char| c == ',' || c == 'x' || c == 'X' || c == '×' || c.is_whitespace())
        .filter(|s| !s.is_empty())
        .map(|s| s.parse().map_err(|_| ()))
        .collect::<Result<_, _>>()
        .ok()
        .unwrap_or_default();
    match numbers[..] {
        [rows, cols] if (1..=20).contains(&rows) && (1..=10).contains(&cols) => Some(Ok((rows, cols))),
        _ => Some(Err(())),
    }
}

/// A Markdown table with a header row of "Column N" and `rows` empty rows.
/// Returns the text and the byte range of the first header cell (to select it).
pub fn table_markdown(rows: usize, cols: usize) -> (String, std::ops::Range<usize>) {
    let header: Vec<String> = (1..=cols).map(|i| format!("Column {i}")).collect();
    let mut out = format!("| {} |\n", header.join(" | "));
    out.push_str(&format!("|{}\n", "---|".repeat(cols)));
    for _ in 0..rows {
        out.push_str(&format!("|{}\n", "   |".repeat(cols)));
    }
    (out, 2..2 + header[0].len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(text: &str) -> Option<Vec<&'static str>> {
        suggestions(text, text.len()).map(|(_, m)| m.into_iter().map(|c| c.name).collect())
    }

    #[test]
    fn suggests() {
        assert_eq!(names("hello @").map(|n| n.len()), Some(COMMANDS.len()));
        assert_eq!(names("@re"), Some(vec!["remind"]));
        assert_eq!(names("x\n@Ta"), Some(vec!["table"]));
        assert_eq!(names("@remind"), None); // already complete
        assert_eq!(names("@remind me"), None);
        assert_eq!(names("mail a@b"), None);
        assert_eq!(names("@zz"), None);
    }

    #[test]
    fn slash_menu() {
        let names = |text: &str| slash_suggestions(text, text.len()).map(|(at, m)| (at, m.iter().map(|s| s.name).collect::<Vec<_>>()));
        assert_eq!(names("/che"), Some((0, vec!["checklist"])));
        assert_eq!(names("x\n  /ta"), Some((4, vec!["table"])));
        assert_eq!(names("text /h"), None); // not at line start
        assert_eq!(names("/").map(|(_, m)| m.len()), Some(SLASH.len()));
        for item in SLASH {
            assert!(item.cursor <= item.insert.len() && item.insert.is_char_boundary(item.cursor), "{}", item.name);
        }
    }

    #[test]
    fn small_commands() {
        assert_eq!(parse_calc("@calc 12*3.5 + 8"), Some(Ok("12*3.5 + 8 = 50".into())));
        assert_eq!(parse_calc("@calc oops"), Some(Err(())));
        assert_eq!(parse_calc("@calculate"), None);
        let now = chrono::Utc::now();
        assert!(parse_time("@time tokyo to london", now).is_some_and(|r| r.is_ok_and(|text| text.contains(" = "))));
        assert_eq!(parse_time("@time somewhere odd", now), Some(Err(())));
        // `@timer` is a different command.
        assert_eq!(parse_time("@timer 25", now), None);
        assert_eq!(parse_time("time tokyo", now), None);
        assert_eq!(currency_request("@curr 100 usd to inr"), Some("100 usd to inr"));
        assert_eq!(currency_request("  @currency 5 euro in yen"), Some("5 euro in yen"));
        assert_eq!(currency_request("@curr"), Some(""));
        assert_eq!(currency_request("@currently busy"), None);
        assert_eq!(currency_request("curr 100 usd to inr"), None);
        assert_eq!(parse_goal("@goal 500"), Some(Ok(Some(500))));
        assert_eq!(parse_goal("@goal off"), Some(Ok(None)));
        assert_eq!(parse_timer("@timer"), Some(Ok(Some(25))));
        assert_eq!(parse_timer("@timer 50 min"), Some(Ok(Some(50))));
        assert_eq!(parse_timer("@timer stop"), Some(Ok(None)));
        assert_eq!(date_token_before("due @today", 10), Some((4, "today")));
        assert_eq!(date_token_before("a@today", 7), None);
    }

    #[test]
    fn tables() {
        assert_eq!(parse_table("@table 3,4"), Some(Ok((3, 4))));
        assert_eq!(parse_table("@table 2x2"), Some(Ok((2, 2))));
        assert_eq!(parse_table("@table 2 3"), Some(Ok((2, 3))));
        assert_eq!(parse_table("@table 99,1"), Some(Err(())));
        assert_eq!(parse_table("@table"), Some(Err(())));
        assert_eq!(parse_table("@tables"), None);
        let (md, cell) = table_markdown(1, 2);
        assert_eq!(md, "| Column 1 | Column 2 |\n|---|---|\n|   |   |\n");
        assert_eq!(&md[cell], "Column 1");
    }
}
