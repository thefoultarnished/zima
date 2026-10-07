//! Search queries: plain words plus filters.
//!
//! - `#tag`
//! - `notebook:Work`
//! - `created:today` / `modified:last week` (today, yesterday, this week, last week, this month,
//!   last month, 2026-10-07, 2026-10)
//! - `has:tasks`, `has:open-tasks`, `has:links`, `has:tags`, `has:code`, `has:due`
//! - `is:pinned`, `is:favorite`, `is:archived`, `is:daily`

use chrono::{Datelike, Duration, Local, NaiveDate, TimeZone};

use crate::model::Note;

#[derive(Debug, Default, PartialEq)]
pub struct Query {
    /// Words that must all appear (in the title or body), lowercased.
    pub words: Vec<String>,
    pub tag: Option<String>,
    pub notebook: Option<String>,
    pub created: Option<(NaiveDate, NaiveDate)>,
    pub modified: Option<(NaiveDate, NaiveDate)>,
    pub has: Vec<String>,
    pub is: Vec<String>,
}

impl Query {
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        *self == Query::default()
    }

    /// The plain text part, for highlighting snippets.
    pub fn text(&self) -> String {
        self.words.join(" ")
    }

    pub fn matches(&self, note: &Note) -> bool {
        let title = note.title.to_lowercase();
        let body = note.body.to_lowercase();
        if !self.words.iter().all(|w| title.contains(w.as_str()) || body.contains(w.as_str())) {
            return false;
        }
        if let Some(tag) = &self.tag {
            if !crate::app::extract_tags(&note.body).iter().any(|t| t.to_lowercase() == *tag) {
                return false;
            }
        }
        if let Some(notebook) = &self.notebook {
            if note.notebook.as_deref().map(str::to_lowercase).as_deref() != Some(notebook.as_str()) {
                return false;
            }
        }
        let date_of = |ms: i64| Local.timestamp_millis_opt(ms).single().map(|d| d.date_naive());
        let created_ms = if note.created > 0 { note.created } else { note.modified };
        for (range, ms) in [(self.created, created_ms), (self.modified, note.modified)] {
            if let Some((from, to)) = range {
                if !date_of(ms).is_some_and(|d| d >= from && d <= to) {
                    return false;
                }
            }
        }
        let now = Local::now();
        self.has.iter().all(|h| match h.as_str() {
            "tasks" => !crate::tasks::extract(&note.body, now).is_empty(),
            "open-tasks" => crate::tasks::extract(&note.body, now).iter().any(|t| !t.done),
            "due" => crate::tasks::extract(&note.body, now).iter().any(|t| t.due.is_some() && !t.done),
            "links" => note.body.contains("[[") || note.body.contains("]("),
            "tags" => !crate::app::extract_tags(&note.body).is_empty(),
            "code" => note.body.contains("```") || note.body.contains('`'),
            _ => true,
        }) && self.is.iter().all(|i| match i.as_str() {
            "pinned" => note.pinned,
            "favorite" | "favourite" | "starred" => note.favorite,
            "archived" => note.archived,
            "daily" => note.daily.is_some(),
            _ => true,
        })
    }
}

/// A date range from words like "today", "last week", "2026-10".
fn date_range(value: &str, today: NaiveDate) -> Option<(NaiveDate, NaiveDate)> {
    let monday = today - Duration::days(today.weekday().num_days_from_monday() as i64);
    let first_of_month = today.with_day(1)?;
    Some(match value {
        "today" => (today, today),
        "yesterday" => (today - Duration::days(1), today - Duration::days(1)),
        "this week" | "week" => (monday, today),
        "last week" => (monday - Duration::days(7), monday - Duration::days(1)),
        "this month" | "month" => (first_of_month, today),
        "last month" => {
            let end = first_of_month - Duration::days(1);
            (end.with_day(1)?, end)
        }
        "this year" | "year" => (today.with_ordinal(1)?, today),
        other => {
            if let Ok(day) = NaiveDate::parse_from_str(other, "%Y-%m-%d") {
                (day, day)
            } else {
                let start = NaiveDate::parse_from_str(&format!("{other}-01"), "%Y-%m-%d").ok()?;
                let next = if start.month() == 12 {
                    NaiveDate::from_ymd_opt(start.year() + 1, 1, 1)?
                } else {
                    NaiveDate::from_ymd_opt(start.year(), start.month() + 1, 1)?
                };
                (start, next - Duration::days(1))
            }
        }
    })
}

pub fn parse(query: &str, today: NaiveDate) -> Query {
    let mut q = Query::default();
    let tokens: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    let mut i = 0;
    while i < tokens.len() {
        let token = &tokens[i];
        // "last week", "this month": the filter value can be two words.
        let two_words = |value: &str| -> (String, usize) {
            match (value, tokens.get(i + 1)) {
                ("last" | "this", Some(next)) => (format!("{value} {next}"), 2),
                _ => (value.to_string(), 1),
            }
        };
        if let Some(tag) = token.strip_prefix('#').filter(|t| !t.is_empty()) {
            q.tag = Some(tag.to_string());
        } else if let Some(value) = token.strip_prefix("notebook:").or_else(|| token.strip_prefix("nb:")) {
            q.notebook = Some(value.replace('_', " "));
        } else if let Some(value) = token.strip_prefix("created:") {
            let (value, used) = two_words(value);
            q.created = date_range(&value, today);
            i += used;
            continue;
        } else if let Some(value) = token.strip_prefix("modified:").or_else(|| token.strip_prefix("edited:")) {
            let (value, used) = two_words(value);
            q.modified = date_range(&value, today);
            i += used;
            continue;
        } else if let Some(value) = token.strip_prefix("has:") {
            q.has.push(value.to_string());
        } else if let Some(value) = token.strip_prefix("is:") {
            q.is.push(value.to_string());
        } else {
            q.words.push(token.clone());
        }
        i += 1;
    }
    q
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, m, day).unwrap()
    }

    #[test]
    fn parses_filters() {
        let today = d(10, 7); // Wednesday
        let q = parse("Atlas #work created:last week has:tasks is:pinned notebook:projects", today);
        assert_eq!(q.words, vec!["atlas"]);
        assert_eq!(q.tag.as_deref(), Some("work"));
        assert_eq!(q.created, Some((d(9, 28), d(10, 4))));
        assert_eq!((q.has.clone(), q.is.clone()), (vec!["tasks".to_string()], vec!["pinned".to_string()]));
        assert_eq!(q.notebook.as_deref(), Some("projects"));
        assert_eq!(parse("modified:this month", today).modified, Some((d(10, 1), d(10, 7))));
        assert_eq!(parse("created:2026-09", today).created, Some((d(9, 1), d(9, 30))));
        assert_eq!(parse("created:yesterday", today).created, Some((d(10, 6), d(10, 6))));
        assert!(parse("   ", today).is_empty());
    }

    #[test]
    fn matches_notes() {
        let note = Note { title: "Atlas".into(), body: "- [ ] ship #work".into(), pinned: true, ..Default::default() };
        let today = Local::now().date_naive();
        assert!(parse("atlas has:open-tasks is:pinned #work", today).matches(&note));
        assert!(!parse("atlas is:favorite", today).matches(&note));
        assert!(!parse("nothing", today).matches(&note));
    }
}
