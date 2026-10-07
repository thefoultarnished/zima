//! Tasks across notes: checkboxes with `@due <date>` and `@every <schedule>`.

use chrono::{DateTime, Local, NaiveDate};

use crate::reminders::{self, Repeat};

#[derive(Debug, Clone, PartialEq)]
pub struct Task {
    /// 0-based line in the note.
    pub line: usize,
    /// The task text without the checkbox and `@due` / `@every` parts.
    pub text: String,
    pub done: bool,
    pub due: Option<NaiveDate>,
    pub every: Option<String>,
}

/// The value of `@name …` in a line: up to the next ` @` or ` #`, or the end.
fn tag_value<'a>(line: &'a str, name: &str) -> Option<(usize, usize, &'a str)> {
    let needle = format!("@{name} ");
    let start = line.find(&needle)?;
    if start > 0 && !line[..start].ends_with(' ') {
        return None;
    }
    let value_start = start + needle.len();
    let rest = &line[value_start..];
    let end = [rest.find(" @"), rest.find(" #")].into_iter().flatten().min().map_or(line.len(), |i| value_start + i);
    Some((start, end, line[value_start..end].trim()))
}

/// Remove `@due …` and `@every …` from the visible text.
fn strip_tags(text: &str) -> String {
    let mut text = text.to_string();
    for name in ["due", "every"] {
        if let Some((start, end, _)) = tag_value(&text, name) {
            text.replace_range(start..end, "");
        }
    }
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Every checkbox in a note.
pub fn extract(body: &str, now: DateTime<Local>) -> Vec<Task> {
    let mut tasks = Vec::new();
    let mut in_code = false;
    for (line_no, line) in body.lines().enumerate() {
        if line.trim_start().starts_with("```") {
            in_code = !in_code;
        }
        if in_code {
            continue;
        }
        let trimmed = line.trim_start();
        let Some(after_bullet) = ["- ", "* ", "+ "].iter().find_map(|b| trimmed.strip_prefix(b)) else { continue };
        let (done, rest) = if let Some(rest) = after_bullet.strip_prefix("[ ] ") {
            (false, rest)
        } else if let Some(rest) = after_bullet.strip_prefix("[x] ").or_else(|| after_bullet.strip_prefix("[X] ")) {
            (true, rest)
        } else {
            continue;
        };
        let due = tag_value(rest, "due").and_then(|(_, _, value)| reminders::parse_date(value, now));
        let every = tag_value(rest, "every").map(|(_, _, value)| value.to_string());
        tasks.push(Task { line: line_no, text: strip_tags(rest), done, due, every });
    }
    tasks
}

/// Tick or untick the checkbox on `line`. A repeating task (`@every …`) doesn't stay ticked:
/// it moves its `@due` to the next occurrence instead. Returns the new body.
pub fn toggle(body: &str, line: usize, now: DateTime<Local>) -> Option<String> {
    let lines: Vec<&str> = body.split('\n').collect();
    let current = *lines.get(line)?;
    let indent = current.len() - current.trim_start().len();
    let trimmed = &current[indent..];
    let bullet = ["- ", "* ", "+ "].iter().find(|b| trimmed.starts_with(**b))?;
    let after = &trimmed[bullet.len()..];
    let (ticked, rest) = if let Some(rest) = after.strip_prefix("[ ] ") {
        (false, rest)
    } else {
        (true, after.strip_prefix("[x] ").or_else(|| after.strip_prefix("[X] "))?)
    };

    let new_line = match tag_value(rest, "every").and_then(|(_, _, spec)| Repeat::parse(spec)) {
        // Completing a repeating task: keep it open, due next time.
        Some(repeat) if !ticked => {
            let next = repeat.next_after(now)?.date_naive().format("%Y-%m-%d").to_string();
            let rest = match tag_value(rest, "due") {
                Some((start, end, _)) => format!("{}@due {next}{}", &rest[..start], &rest[end..]),
                None => format!("{rest} @due {next}"),
            };
            format!("{}{bullet}[ ] {rest}", &current[..indent])
        }
        _ => format!("{}{bullet}{} {rest}", &current[..indent], if ticked { "[ ]" } else { "[x]" }),
    };
    let mut out: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
    out[line] = new_line;
    Some(out.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    // Wednesday 7 Oct 2026, 2pm.
    fn now() -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 10, 7, 14, 0, 0).unwrap()
    }

    fn date(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, d).unwrap()
    }

    #[test]
    fn extracts_tasks_with_dates() {
        let body = "- [ ] send invoice @due fri #work\n- [x] done thing\ntext\n  - [ ] water plants @every monday @due 2026-10-05\n```\n- [ ] not a task\n```";
        let tasks = extract(body, now());
        assert_eq!(tasks.len(), 3);
        assert_eq!((tasks[0].text.as_str(), tasks[0].due, tasks[0].done), ("send invoice #work", Some(date(9)), false));
        assert_eq!((tasks[1].done, tasks[1].due), (true, None));
        assert_eq!((tasks[2].line, tasks[2].every.as_deref(), tasks[2].due), (3, Some("monday"), Some(date(5))));
    }

    #[test]
    fn due_dates() {
        assert_eq!(reminders::parse_date("today", now()), Some(date(7)));
        assert_eq!(reminders::parse_date("tomorrow", now()), Some(date(8)));
        assert_eq!(reminders::parse_date("wednesday", now()), Some(date(7)));
        assert_eq!(reminders::parse_date("next monday", now()), Some(date(12)));
        assert_eq!(reminders::parse_date("2026-12-25", now()).map(|d| d.to_string()), Some("2026-12-25".into()));
        assert_eq!(reminders::parse_date("in 3 days", now()), Some(date(10)));
    }

    #[test]
    fn toggling() {
        assert_eq!(toggle("- [ ] a\n- [ ] b", 1, now()).unwrap(), "- [ ] a\n- [x] b");
        assert_eq!(toggle("  * [x] a", 0, now()).unwrap(), "  * [ ] a");
        // Repeating: stays open, due moves to the next Monday.
        assert_eq!(toggle("- [ ] water @every monday @due 2026-10-05", 0, now()).unwrap(), "- [ ] water @every monday @due 2026-10-12");
        assert_eq!(toggle("- [ ] stretch @every day", 0, now()).unwrap(), "- [ ] stretch @every day @due 2026-10-08");
        assert!(toggle("plain", 0, now()).is_none());
    }
}
