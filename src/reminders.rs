//! `@remind` parsing and due-time formatting.
//!
//! Understands lines like:
//! - `@remind me to call mom at 5pm`
//! - `@remind stretch in 20 min`
//! - `@remind me to pay rent tomorrow at 9am`
//! - `@remind standup monday 10:30`
//! - `@remind take out the bins tonight`

use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, NaiveTime, TimeZone, Weekday};
use serde::{Deserialize, Serialize};

#[derive(Debug, PartialEq)]
pub struct Parsed {
    pub text: String,
    pub due: DateTime<Local>,
    pub repeat: Option<Repeat>,
}

/// `None` if `line` isn't an `@remind` command; `Some(Err(()))` if it is but the time wasn't understood.
pub fn parse_command(line: &str, now: DateTime<Local>) -> Option<Result<Parsed, ()>> {
    let rest = strip_prefix_ci(line.trim_start(), "@remind")?;
    if !(rest.is_empty() || rest.starts_with(char::is_whitespace)) {
        return None; // e.g. "@reminders" is not the command
    }
    Some(parse(rest, now).ok_or(()))
}

/// Parse what follows `@remind`: a description, then a time expression at the end.
/// The time can repeat: "every weekday at 9:30", "every monday", "every 2 hours", "daily at 8".
pub fn parse(input: &str, now: DateTime<Local>) -> Option<Parsed> {
    let input = input.trim();
    let input = ["me to ", "to ", "me "]
        .iter()
        .find_map(|prefix| strip_prefix_ci(input, prefix))
        .unwrap_or(input);
    let words: Vec<&str> = input.split_whitespace().collect();

    // Repeating: "<description> every …" / "<description> daily|weekdays …".
    if let Some(i) = words.iter().position(|w| matches!(w.to_lowercase().as_str(), "every" | "daily" | "weekdays")) {
        if i >= 1 {
            let spec = if words[i].eq_ignore_ascii_case("every") { words[i + 1..].join(" ") } else { words[i..].join(" ") };
            let repeat = Repeat::parse(&spec)?;
            let due = repeat.next_after(now)?;
            return Some(Parsed { text: words[..i].join(" "), due, repeat: Some(repeat) });
        }
    }

    // Take the longest time expression at the end, keeping at least one word of description.
    for start in 1..words.len() {
        // "at 5pm" alone has no description; don't treat "at" as one.
        let last = words[start - 1].to_lowercase();
        if matches!(last.as_str(), "at" | "on" | "by" | "@" | "in" | "this" | "next") {
            continue;
        }
        match parse_when(&words[start..].join(" "), now) {
            Some(Some(due)) => return Some(Parsed { text: words[..start].join(" "), due, repeat: None }),
            // A real time that has already passed: don't fall back to a shorter match.
            Some(None) => return None,
            None => {}
        }
    }
    None
}

/// Spot a time in an ordinary line ("call Sam tomorrow at 5") to offer a reminder. Conservative:
/// needs a clear time word, and a due time within the next 30 days.
pub fn spot(line: &str, now: DateTime<Local>) -> Option<Parsed> {
    let trimmed = line.trim().trim_start_matches(['-', '*', '>', ' ']).trim_start_matches("[ ] ");
    if trimmed.starts_with('@') || trimmed.starts_with('#') || trimmed.contains('`') || trimmed.contains('|') {
        return None;
    }
    let lower = format!(" {} ", trimmed.to_lowercase());
    let cues = [
        " tomorrow", " tonight", " today ", " at ", "pm ", "am ", " noon", " in ", " next ", " monday", " tuesday",
        " wednesday", " thursday", " friday", " saturday", " sunday",
    ];
    if !cues.iter().any(|cue| lower.contains(cue)) {
        return None;
    }
    let parsed = parse(trimmed, now)?;
    let soon = parsed.due - now <= Duration::days(30);
    (soon && parsed.repeat.is_none() && parsed.text.chars().filter(|c| c.is_alphabetic()).count() >= 3).then_some(parsed)
}

/// How a reminder repeats.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Repeat {
    Daily { hour: u32, minute: u32 },
    Weekdays { hour: u32, minute: u32 },
    /// `weekday`: 0 = Monday.
    Weekly { weekday: u32, hour: u32, minute: u32 },
    Interval { minutes: u32 },
}

impl Repeat {
    /// "day at 9", "weekday", "monday at 10:30", "2 hours", "30 min", "hour", "morning".
    pub fn parse(spec: &str) -> Option<Self> {
        let spec = spec.to_lowercase();
        let tokens: Vec<&str> = spec.split_whitespace().collect();
        let (head, rest) = tokens.split_first()?;
        // A time of day after the day word ("at 9:30", "9am"); 9am if none.
        let time_of_day = |rest: &[&str]| -> Option<(u32, u32)> {
            let rest: Vec<&str> = rest.iter().copied().filter(|t| *t != "at").collect();
            if rest.is_empty() {
                return Some((9, 0));
            }
            match rest[0] {
                "morning" => return Some((9, 0)),
                "noon" => return Some((12, 0)),
                "evening" => return Some((18, 0)),
                "night" => return Some((20, 0)),
                _ => {}
            }
            let (t, used) = parse_time(&rest)?;
            if used != rest.len() {
                return None;
            }
            // "every day at 5" means 5pm; mornings need "am".
            let hour = if t.ambiguous && t.hour < 7 { t.hour + 12 } else { t.hour };
            Some((hour, t.minute))
        };
        Some(match *head {
            "day" | "daily" => {
                let (hour, minute) = time_of_day(rest)?;
                Repeat::Daily { hour, minute }
            }
            "morning" => Repeat::Daily { hour: 9, minute: 0 },
            "evening" => Repeat::Daily { hour: 18, minute: 0 },
            "weekday" | "weekdays" => {
                let (hour, minute) = time_of_day(rest)?;
                Repeat::Weekdays { hour, minute }
            }
            "hour" => Repeat::Interval { minutes: 60 },
            "minute" => Repeat::Interval { minutes: 1 },
            word => {
                if let Some(weekday) = parse_weekday(word.trim_end_matches('s')) {
                    let (hour, minute) = time_of_day(rest)?;
                    Repeat::Weekly { weekday: weekday.num_days_from_monday(), hour, minute }
                } else {
                    let (duration, used) = parse_duration(&tokens)?;
                    let minutes = duration.num_minutes();
                    if used != tokens.len() || !(1..=60 * 24 * 30).contains(&minutes) {
                        return None;
                    }
                    Repeat::Interval { minutes: minutes as u32 }
                }
            }
        })
    }

    /// The first occurrence strictly after `now`.
    pub fn next_after(&self, now: DateTime<Local>) -> Option<DateTime<Local>> {
        let at = |date: NaiveDate, hour: u32, minute: u32| at_local(date, NaiveTime::from_hms_opt(hour, minute, 0)?);
        let today = now.date_naive();
        match *self {
            Repeat::Interval { minutes } => Some(now + Duration::minutes(minutes as i64)),
            Repeat::Daily { hour, minute } => (0..=1).filter_map(|d| at(today + Duration::days(d), hour, minute)).find(|t| *t > now),
            Repeat::Weekdays { hour, minute } => (0..=7)
                .map(|d| today + Duration::days(d))
                .filter(|date| date.weekday().num_days_from_monday() < 5)
                .filter_map(|date| at(date, hour, minute))
                .find(|t| *t > now),
            Repeat::Weekly { weekday, hour, minute } => (0..=7)
                .map(|d| today + Duration::days(d))
                .filter(|date| date.weekday().num_days_from_monday() == weekday)
                .filter_map(|date| at(date, hour, minute))
                .find(|t| *t > now),
        }
    }

    pub fn describe(&self) -> String {
        let time = |hour: u32, minute: u32| {
            NaiveTime::from_hms_opt(hour, minute, 0).map(|t| t.format("%-I:%M %p").to_string()).unwrap_or_default()
        };
        let days = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
        match *self {
            Repeat::Daily { hour, minute } => format!("every day at {}", time(hour, minute)),
            Repeat::Weekdays { hour, minute } => format!("weekdays at {}", time(hour, minute)),
            Repeat::Weekly { weekday, hour, minute } => {
                format!("every {} at {}", days[weekday as usize % 7], time(hour, minute))
            }
            Repeat::Interval { minutes } if minutes % 60 == 0 && minutes >= 60 => match minutes / 60 {
                1 => "every hour".into(),
                h => format!("every {h} hours"),
            },
            Repeat::Interval { minutes } => format!("every {minutes} min"),
        }
    }
}

/// A time of day; `ambiguous` means "5" could be 5am or 5pm.
#[derive(Clone, Copy)]
struct TimeOfDay {
    hour: u32,
    minute: u32,
    ambiguous: bool,
}

impl TimeOfDay {
    const fn exact(hour: u32, minute: u32) -> Self {
        Self { hour, minute, ambiguous: false }
    }
}

/// A date for `@due`: "2026-10-20", "today", "tomorrow", "fri", "next monday", "in 3 days" …
/// Unlike reminders, a past date is fine (the task is overdue).
pub fn parse_date(input: &str, now: DateTime<Local>) -> Option<NaiveDate> {
    let input = input.trim();
    if let Ok(date) = NaiveDate::parse_from_str(input, "%Y-%m-%d") {
        return Some(date);
    }
    match input.to_lowercase().as_str() {
        "today" => return Some(now.date_naive()),
        "yesterday" => return now.date_naive().pred_opt(),
        _ => {}
    }
    // Weekdays and relative dates; the time part doesn't matter.
    let noon = now.date_naive().and_hms_opt(0, 0, 1).and_then(|t| Local.from_local_datetime(&t).earliest()).unwrap_or(now);
    parse_when(input, noon).flatten().map(|dt| dt.date_naive())
}

/// `None`: not a time expression. `Some(None)`: a time expression, but already in the past.
fn parse_when(input: &str, now: DateTime<Local>) -> Option<Option<DateTime<Local>>> {
    let input = input.to_lowercase();
    let tokens: Vec<&str> = input
        .split_whitespace()
        .map(|t| t.trim_end_matches([',', '.', '!']))
        .filter(|t| !t.is_empty())
        .collect();
    let today = now.date_naive();

    let mut day: Option<NaiveDate> = None;
    let mut from_weekday = false;
    let mut time: Option<TimeOfDay> = None;
    let mut offset: Option<Duration> = None;

    let mut i = 0;
    while i < tokens.len() {
        let token = tokens[i];
        match token {
            "at" | "on" | "by" | "@" | "this" => {}
            "in" => {
                let (duration, used) = parse_duration(&tokens[i + 1..])?;
                offset = Some(offset.unwrap_or_else(Duration::zero) + duration);
                i += 1 + used;
                continue;
            }
            "today" => day = Some(today),
            "tomorrow" | "tmrw" | "tmr" => day = Some(today.succ_opt()?),
            "tonight" => {
                day = Some(today);
                time = Some(TimeOfDay::exact(20, 0));
            }
            "noon" | "midday" => time = Some(TimeOfDay::exact(12, 0)),
            "midnight" => time = Some(TimeOfDay::exact(0, 0)),
            "morning" => time = Some(TimeOfDay::exact(9, 0)),
            "afternoon" => time = Some(TimeOfDay::exact(15, 0)),
            "evening" => time = Some(TimeOfDay::exact(18, 0)),
            "night" => time = Some(TimeOfDay::exact(20, 0)),
            "next" => {
                // "next monday": the coming Monday, never today.
                let weekday = parse_weekday(tokens.get(i + 1)?)?;
                day = Some(next_weekday(today, weekday, false));
                i += 2;
                continue;
            }
            _ => {
                if let Some(weekday) = parse_weekday(token) {
                    day = Some(next_weekday(today, weekday, true));
                    from_weekday = true;
                } else if let Some((tod, used)) = parse_time(&tokens[i..]) {
                    time = Some(tod);
                    i += used;
                    continue;
                } else {
                    return None;
                }
            }
        }
        i += 1;
    }

    if let Some(offset) = offset {
        // "in 20 min" can't be combined with a day or time.
        if day.is_some() || time.is_some() {
            return None;
        }
        return Some((offset > Duration::zero()).then(|| now + offset));
    }
    if day.is_none() && time.is_none() {
        return None;
    }

    let candidates: Vec<NaiveTime> = match time {
        // A day on its own means 9am.
        None => vec![NaiveTime::from_hms_opt(9, 0, 0)?],
        // A bare 1–6 almost always means afternoon/evening ("tomorrow at 5").
        Some(t) if t.ambiguous && t.hour <= 6 => vec![NaiveTime::from_hms_opt(t.hour + 12, t.minute, 0)?],
        Some(t) if t.ambiguous && t.hour < 12 => vec![
            NaiveTime::from_hms_opt(t.hour, t.minute, 0)?,
            NaiveTime::from_hms_opt(t.hour + 12, t.minute, 0)?,
        ],
        Some(t) => vec![NaiveTime::from_hms_opt(t.hour, t.minute, 0)?],
    };
    let first_future = |date: NaiveDate| {
        candidates
            .iter()
            .filter_map(|&t| at_local(date, t))
            .find(|dt| *dt > now)
    };

    Some(match day {
        // No day: the next time that time comes round.
        None => first_future(today).or_else(|| at_local(today.succ_opt()?, candidates[0])),
        // "monday 9am" on a Monday after 9am means next week.
        Some(date) if from_weekday => first_future(date).or_else(|| first_future(date + Duration::days(7))),
        Some(date) => first_future(date),
    })
}

/// "5", "5pm", "5 pm", "5:30", "17:30", "5:30pm". Returns the time and how many tokens it used.
fn parse_time(tokens: &[&str]) -> Option<(TimeOfDay, usize)> {
    let token = *tokens.first()?;
    let mut body = token;
    let mut pm: Option<bool> = None;
    for (suffix, is_pm) in [("a.m", false), ("p.m", true), ("am", false), ("pm", true)] {
        if let Some(stripped) = token.strip_suffix(suffix) {
            body = stripped;
            pm = Some(is_pm);
            break;
        }
    }
    let mut used = 1;
    if pm.is_none() {
        match tokens.get(1).copied() {
            Some("am" | "a.m") => (pm, used) = (Some(false), 2),
            Some("pm" | "p.m") => (pm, used) = (Some(true), 2),
            _ => {}
        }
    }

    let (hour, minute): (u32, u32) = match body.split_once(':') {
        Some((h, m)) if m.len() == 2 => (h.parse().ok()?, m.parse().ok()?),
        Some(_) => return None,
        None => (body.parse().ok()?, 0),
    };
    if minute > 59 {
        return None;
    }
    let tod = match pm {
        Some(is_pm) => {
            if !(1..=12).contains(&hour) {
                return None;
            }
            TimeOfDay::exact(hour % 12 + if is_pm { 12 } else { 0 }, minute)
        }
        None if hour > 23 => return None,
        // 1–12 without am/pm could be either; 0 and 13–23 are clearly 24-hour.
        None => TimeOfDay { hour, minute, ambiguous: (1..=12).contains(&hour) },
    };
    Some((tod, used))
}

/// "20 min", "2 hours 30 minutes", "1h", "an hour", "3 days". Returns the duration and tokens used.
fn parse_duration(tokens: &[&str]) -> Option<(Duration, usize)> {
    let mut total = Duration::zero();
    let mut i = 0;
    let mut any = false;
    while i < tokens.len() {
        if tokens[i] == "and" && any {
            i += 1;
            continue;
        }
        // Compact form: "20min", "2h".
        let digits = tokens[i].trim_end_matches(|c: char| c.is_ascii_alphabetic());
        if !digits.is_empty() && digits.len() < tokens[i].len() {
            if let (Ok(n), Some(d)) = (digits.parse(), unit(&tokens[i][digits.len()..])) {
                total += d * n;
                i += 1;
                any = true;
                continue;
            }
        }
        let n: i32 = match tokens[i] {
            "a" | "an" | "one" => 1,
            t => match t.parse() {
                Ok(n) => n,
                Err(_) => break,
            },
        };
        let Some(d) = tokens.get(i + 1).and_then(|u| unit(u)) else { break };
        total += d * n;
        i += 2;
        any = true;
    }
    any.then_some((total, i))
}

fn unit(word: &str) -> Option<Duration> {
    Some(match word {
        "s" | "sec" | "secs" | "second" | "seconds" => Duration::seconds(1),
        "m" | "min" | "mins" | "minute" | "minutes" => Duration::minutes(1),
        "h" | "hr" | "hrs" | "hour" | "hours" => Duration::hours(1),
        "d" | "day" | "days" => Duration::days(1),
        "w" | "wk" | "week" | "weeks" => Duration::weeks(1),
        _ => return None,
    })
}

pub(crate) fn parse_weekday(word: &str) -> Option<Weekday> {
    Some(match word {
        "mon" | "monday" => Weekday::Mon,
        "tue" | "tues" | "tuesday" => Weekday::Tue,
        "wed" | "wednesday" => Weekday::Wed,
        "thu" | "thur" | "thurs" | "thursday" => Weekday::Thu,
        "fri" | "friday" => Weekday::Fri,
        "sat" | "saturday" => Weekday::Sat,
        "sun" | "sunday" => Weekday::Sun,
        _ => return None,
    })
}

fn next_weekday(from: NaiveDate, weekday: Weekday, allow_today: bool) -> NaiveDate {
    let mut days = (7 + weekday.num_days_from_monday() as i64 - from.weekday().num_days_from_monday() as i64) % 7;
    if days == 0 && !allow_today {
        days = 7;
    }
    from + Duration::days(days)
}

fn at_local(date: NaiveDate, time: NaiveTime) -> Option<DateTime<Local>> {
    Local.from_local_datetime(&date.and_time(time)).earliest()
}

fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let head = s.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix).then(|| &s[prefix.len()..])
}

/// "Today 2:00 PM", "Tomorrow 9:00 AM", "Mon 10:30 AM", "Oct 20, 9:00 AM".
pub fn format_due(due: DateTime<Local>, now: DateTime<Local>) -> String {
    let time = due.format("%-I:%M %p");
    let days = (due.date_naive() - now.date_naive()).num_days();
    match days {
        0 => format!("Today {time}"),
        1 => format!("Tomorrow {time}"),
        2..=6 => format!("{} {time}", due.format("%a")),
        _ if due.year() == now.year() => format!("{}, {time}", due.format("%b %-d")),
        _ => format!("{}, {time}", due.format("%b %-d %Y")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Wednesday, Oct 7 2026, 2:00 PM.
    fn now() -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 10, 7, 14, 0, 0).unwrap()
    }

    fn at(d: u32, h: u32, m: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 10, d, h, m, 0).unwrap()
    }

    fn check(input: &str, text: &str, due: DateTime<Local>) {
        let parsed = parse(input, now()).unwrap_or_else(|| panic!("failed to parse {input:?}"));
        assert_eq!(parsed, Parsed { text: text.into(), due, repeat: None }, "{input:?}");
    }

    #[test]
    fn times_of_day() {
        check("me to call mom at 5pm", "call mom", at(7, 17, 0));
        check("me to call mom at 5 pm", "call mom", at(7, 17, 0));
        check("call mom @ 5:30pm", "call mom", at(7, 17, 30));
        check("standup at 17:45", "standup", at(7, 17, 45));
        // Ambiguous "at 5" picks the next 5 o'clock: 5pm today.
        check("tea at 5", "tea", at(7, 17, 0));
        // 9am already passed today, so tomorrow.
        check("coffee at 9am", "coffee", at(8, 9, 0));
        // Ambiguous "at 11" at 2pm: 11pm today.
        check("sleep at 11", "sleep", at(7, 23, 0));
        check("lunch at noon", "lunch", at(8, 12, 0));
    }

    #[test]
    fn days() {
        check("me to pay rent tomorrow at 9am", "pay rent", at(8, 9, 0));
        check("pay rent tomorrow", "pay rent", at(8, 9, 0));
        check("bins tonight", "bins", at(7, 20, 0));
        check("review friday 10:30am", "review", at(9, 10, 30));
        check("review on fri at 3pm", "review", at(9, 15, 0));
        // Today is Wednesday and 9am has passed: next Wednesday.
        check("gym wednesday 9am", "gym", at(14, 9, 0));
        check("gym wednesday 6pm", "gym", at(7, 18, 0));
        check("plan next wednesday", "plan", at(14, 9, 0));
        check("call 3pm tomorrow", "call", at(8, 15, 0));
    }

    #[test]
    fn relative() {
        check("stretch in 20 min", "stretch", now() + Duration::minutes(20));
        check("me to stretch in 20min", "stretch", now() + Duration::minutes(20));
        check("check oven in an hour", "check oven", now() + Duration::hours(1));
        check("leave in 1 hour and 30 minutes", "leave", now() + Duration::minutes(90));
        check("renew in 2 weeks", "renew", now() + Duration::weeks(2));
    }

    #[test]
    fn descriptions_with_numbers() {
        check("buy 2 eggs at 5pm", "buy 2 eggs", at(7, 17, 0));
        check("read chapter 3 in 10 minutes", "read chapter 3", now() + Duration::minutes(10));
    }

    #[test]
    fn rejects() {
        assert!(parse("me to do something", now()).is_none());
        assert!(parse("at 5pm", now()).is_none()); // no description
        assert!(parse("thing at 25:00", now()).is_none());
        assert!(parse("thing today at 9am", now()).is_none()); // already passed
        assert!(parse_command("@reminders are cool", now()).is_none());
        assert_eq!(parse_command("@remind nonsense", now()), Some(Err(())));
    }

    #[test]
    fn repeating() {
        let p = parse("stand up every weekday at 9:30", now()).unwrap();
        assert_eq!((p.text.as_str(), p.repeat.clone()), ("stand up", Some(Repeat::Weekdays { hour: 9, minute: 30 })));
        assert_eq!(p.due, at(8, 9, 30)); // Wed 2pm now -> Thu 9:30
        let p = parse("water plants every monday", now()).unwrap();
        assert_eq!(p.due, at(12, 9, 0));
        let p = parse("stretch every 2 hours", now()).unwrap();
        assert_eq!((p.due, p.repeat.unwrap().describe()), (now() + Duration::hours(2), "every 2 hours".to_string()));
        let p = parse("journal daily at 9pm", now()).unwrap();
        assert_eq!(p.due, at(7, 21, 0));
        let p = parse("vitamins every day at 8am", now()).unwrap();
        assert_eq!(p.due, at(8, 8, 0));
        assert_eq!(Repeat::Weekly { weekday: 4, hour: 17, minute: 0 }.next_after(at(9, 18, 0)).unwrap(), at(16, 17, 0));
        assert!(parse("x every blue moon", now()).is_none());
    }

    #[test]
    fn spotting() {
        assert_eq!(spot("call Sam tomorrow at 5", now()).map(|p| (p.text, p.due)), Some(("call Sam".into(), at(8, 17, 0))));
        assert_eq!(spot("- dentist friday 10am", now()).map(|p| p.due), Some(at(9, 10, 0)));
        assert!(spot("meet at the park", now()).is_none());
        assert!(spot("just some text", now()).is_none());
        assert!(spot("@remind me at 5", now()).is_none());
    }

    #[test]
    fn formatting() {
        assert_eq!(format_due(at(7, 17, 0), now()), "Today 5:00 PM");
        assert_eq!(format_due(at(8, 9, 0), now()), "Tomorrow 9:00 AM");
        assert_eq!(format_due(at(9, 10, 30), now()), "Fri 10:30 AM");
        assert_eq!(format_due(at(20, 9, 0), now()), "Oct 20, 9:00 AM");
    }
}
