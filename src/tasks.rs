//! Tasks across notes: checkboxes with `@due <date>` and `@every <schedule>`.

use chrono::{DateTime, Duration, Local, NaiveDate};

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

/// What an `@due` line asks for: a task with this text, due on this day (if one was given).
#[derive(Debug, Clone, PartialEq)]
pub struct DueLine {
    pub text: String,
    pub due: Option<NaiveDate>,
}

/// `@due rent tomorrow`, `@due fri call the bank`, `@due taxes by 2026-10-20`: the task, with its
/// day taken out of the text wherever it is. `None` if the line doesn't start with `@due`.
/// Only clear day words count (today, tomorrow, weekdays, "next monday", "3 days", "in 2 hours",
/// a YYYY-MM-DD date), so numbers in the text ("pay 3 bills") aren't read as a time.
pub fn parse_due_line(line: &str, now: DateTime<Local>) -> Option<DueLine> {
    let trimmed = line.trim();
    let head = trimmed.get(..4)?;
    if !head.eq_ignore_ascii_case("@due") || trimmed[4..].chars().next().is_some_and(|c| !c.is_whitespace()) {
        return None;
    }
    let words: Vec<&str> = trimmed[4..].split_whitespace().collect();
    // The last day phrase in the line wins: (first word, word count, date).
    let mut found: Option<(usize, usize, NaiveDate)> = None;
    let mut start = 0;
    while start < words.len() {
        match day_phrase(&words[start..], now) {
            // Carry on after the phrase, so "monday" isn't found again inside "next monday".
            Some((len, date)) => {
                found = Some((start, len, date));
                start += len;
            }
            None => start += 1,
        }
    }
    let (text, due) = match found {
        Some((at, len, date)) => {
            // "by friday" / "on friday": the little word goes too.
            let from = match at.checked_sub(1) {
                Some(i) if matches!(words[i].to_lowercase().as_str(), "by" | "on") => i,
                _ => at,
            };
            let kept: Vec<&str> = words[..from].iter().chain(&words[at + len..]).copied().collect();
            (kept.join(" "), Some(date))
        }
        None => (words.join(" "), None),
    };
    let text = text.trim_matches(|c: char| c == ',' || c == '-' || c.is_whitespace()).to_string();
    Some(DueLine { text, due })
}

/// A day phrase at the start of `words`: how many words it takes, and the date.
fn day_phrase(words: &[&str], now: DateTime<Local>) -> Option<(usize, NaiveDate)> {
    let word = |i: usize| words.get(i).map(|w| w.trim_end_matches([',', '.', '!']).to_lowercase());
    let first = word(0)?;
    if let Some(found) = relative_phrase(&word, now) {
        return Some(found);
    }
    let len = match first.as_str() {
        "today" | "tomorrow" | "tmrw" | "tmr" | "tonight" => 1,
        "next" if word(1).is_some_and(|w| reminders::parse_weekday(&w).is_some()) => 2,
        w if reminders::parse_weekday(w).is_some() => 1,
        w if NaiveDate::parse_from_str(w, "%Y-%m-%d").is_ok() => 1,
        _ => return None,
    };
    let phrase: Vec<String> = (0..len).filter_map(word).collect();
    let date = if first == "tonight" { Some(now.date_naive()) } else { reminders::parse_date(&phrase.join(" "), now) }?;
    Some((len, date))
}

/// "3 days", "in 3 days", "two weeks from now", "a week later", and with "in" also hours and
/// minutes ("in 2 hours", which can land tomorrow late at night). How many words, and the day.
/// Without "in", hours and minutes stay text: "practise 2 hours" isn't a deadline.
fn relative_phrase(word: &dyn Fn(usize) -> Option<String>, now: DateTime<Local>) -> Option<(usize, NaiveDate)> {
    let with_in = word(0).as_deref() == Some("in");
    let at = with_in as usize;
    let count: i64 = match word(at)?.as_str() {
        "a" | "an" | "one" => 1,
        "two" => 2,
        "three" => 3,
        "four" => 4,
        "five" => 5,
        "six" => 6,
        "seven" => 7,
        "eight" => 8,
        "nine" => 9,
        "ten" => 10,
        n if !n.is_empty() && n.len() <= 4 && n.chars().all(|c| c.is_ascii_digit()) => n.parse().ok()?,
        _ => return None,
    };
    let when = match word(at + 1)?.as_str() {
        "day" | "days" => now + Duration::days(count),
        "week" | "weeks" => now + Duration::weeks(count),
        "hour" | "hours" | "hr" | "hrs" if with_in => now + Duration::hours(count),
        "minute" | "minutes" | "min" | "mins" if with_in => now + Duration::minutes(count),
        _ => return None,
    };
    let mut len = at + 2;
    if word(len).as_deref() == Some("later") {
        len += 1;
    } else if word(len).as_deref() == Some("from") && word(len + 1).as_deref() == Some("now") {
        len += 2;
    }
    Some((len, when.date_naive()))
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

    fn due(line: &str) -> Option<(String, Option<NaiveDate>)> {
        parse_due_line(line, now()).map(|d| (d.text, d.due))
    }

    #[test]
    fn due_line_takes_the_day_out_of_the_text() {
        assert_eq!(due("@due rent tomorrow"), Some(("rent".into(), Some(date(8)))));
        assert_eq!(due("@due tomorrow rent"), Some(("rent".into(), Some(date(8)))));
        assert_eq!(due("@due call the bank by fri"), Some(("call the bank".into(), Some(date(9)))));
        assert_eq!(due("@due taxes on 2026-10-20"), Some(("taxes".into(), Some(date(20)))));
        assert_eq!(due("@due dentist next monday"), Some(("dentist".into(), Some(date(12)))));
        assert_eq!(due("@due report in 3 days"), Some(("report".into(), Some(date(10)))));
        assert_eq!(due("  @DUE Rent, Tomorrow"), Some(("Rent".into(), Some(date(8)))));
    }

    #[test]
    fn due_line_numbers_and_times_stay_text() {
        // "3" and "5pm" aren't days; without a day word there's no deadline.
        assert_eq!(due("@due pay 3 bills fri"), Some(("pay 3 bills".into(), Some(date(9)))));
        assert_eq!(due("@due call at 5pm"), Some(("call at 5pm".into(), None)));
        // Hours only count after "in": this is how long, not when.
        assert_eq!(due("@due practise 2 hours"), Some(("practise 2 hours".into(), None)));
        // "by" in the middle of the text stays when it isn't before the day.
        assert_eq!(due("@due stand by the door"), Some(("stand by the door".into(), None)));
    }

    #[test]
    fn due_line_counts_from_now() {
        // Now is Wednesday 7 Oct, 2pm.
        assert_eq!(due("@due rent 3 days"), Some(("rent".into(), Some(date(10)))));
        assert_eq!(due("@due rent in two weeks"), Some(("rent".into(), Some(date(21)))));
        assert_eq!(due("@due rent 3 days from now"), Some(("rent".into(), Some(date(10)))));
        assert_eq!(due("@due report a week later"), Some(("report".into(), Some(date(14)))));
        assert_eq!(due("@due 1 day pack"), Some(("pack".into(), Some(date(8)))));
        assert_eq!(due("@due call in 2 hours"), Some(("call".into(), Some(date(7)))));
        // 14:00 + 12 hours is 2am tomorrow.
        assert_eq!(due("@due check oven in 12 hours"), Some(("check oven".into(), Some(date(8)))));
        assert_eq!(due("@due tea in 30 min"), Some(("tea".into(), Some(date(7)))));
    }

    #[test]
    fn due_line_needs_due_at_the_start() {
        assert_eq!(due("rent @due tomorrow"), None);
        assert_eq!(due("@duet song"), None);
        assert_eq!(due("- [ ] rent @due fri"), None);
        assert_eq!(due("plain text"), None);
    }

    #[test]
    fn due_line_edges() {
        assert_eq!(due("@due"), Some((String::new(), None)));
        assert_eq!(due("@due tomorrow"), Some((String::new(), Some(date(8)))));
        // Two days: the last one is the deadline, the first stays in the text.
        assert_eq!(due("@due plan friday party tomorrow"), Some(("plan friday party".into(), Some(date(8)))));
        assert_eq!(due("@due tonight"), Some((String::new(), Some(date(7)))));
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
