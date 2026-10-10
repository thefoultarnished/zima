//! Writing stats: words written per day, streaks and a heatmap.

use chrono::{Datelike, Duration, Local, NaiveDate};
use slint::{ModelRc, VecModel};

use super::App;
use crate::{StatCell, StatSummary};

/// Weeks shown in the heatmap.
const WEEKS: i64 = 18;

impl App {
    /// Count words added by an edit (deletions don't subtract).
    pub fn record_words(&mut self, before: usize, after: usize) {
        if after > before {
            let today = Local::now().date_naive().format("%Y-%m-%d").to_string();
            *self.stats.entry(today).or_default() += (after - before) as u32;
            self.stats_dirty = true;
        }
    }

    /// A finished focus session.
    pub fn record_focus(&mut self, minutes: u32) {
        let today = Local::now().date_naive().format("%Y-%m-%d").to_string();
        *self.focus.entry(today).or_default() += minutes;
        if let Err(e) = self.store.save_focus(&self.focus) {
            eprintln!("failed to save focus history: {e}");
        }
    }

    pub fn save_stats(&mut self) {
        if self.stats_dirty {
            match self.store.save_stats(&self.stats) {
                Ok(()) => self.stats_dirty = false,
                Err(e) => eprintln!("failed to save stats: {e}"),
            }
        }
    }

    pub fn open_stats(&mut self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let today = Local::now().date_naive();
        let words_on = |date: NaiveDate| *self.stats.get(&date.format("%Y-%m-%d").to_string()).unwrap_or(&0);

        // Streak: consecutive days with writing, ending today (or yesterday, if nothing yet today).
        let mut day = if words_on(today) > 0 { today } else { today - Duration::days(1) };
        let mut streak = 0;
        while words_on(day) > 0 {
            streak += 1;
            day -= Duration::days(1);
        }
        let monday = today - Duration::days(today.weekday().num_days_from_monday() as i64);
        let week: u32 = (0..7).map(|d| words_on(monday + Duration::days(d))).sum();
        let best = self.stats.values().copied().max().unwrap_or(0);
        let listed: Vec<_> = self.notes.iter().filter(|n| n.is_listed()).collect();

        ui.set_stats_summary(StatSummary {
            today: thousands(u64::from(words_on(today))).into(),
            week: thousands(u64::from(week)).into(),
            streak,
            best: thousands(u64::from(best)).into(),
            notes: listed.len() as i32,
            words: thousands(listed.iter().map(|n| n.body.split_whitespace().count() as u64).sum()).into(),
            focus_today: *self.focus.get(&today.format("%Y-%m-%d").to_string()).unwrap_or(&0) as i32,
            focus_week: (0..7)
                .map(|d| *self.focus.get(&(monday + Duration::days(d)).format("%Y-%m-%d").to_string()).unwrap_or(&0))
                .sum::<u32>() as i32,
        });

        // Heatmap from the Monday WEEKS-1 weeks ago through this Sunday.
        let start = monday - Duration::weeks(WEEKS - 1);
        let max = best.max(1) as f32;
        let cells: Vec<StatCell> = (0..WEEKS * 7)
            .map(|i| {
                let date = start + Duration::days(i);
                let words = words_on(date);
                let level = if date > today || words == 0 { 0 } else { 1 + ((words as f32 / max) * 3.0).round() as i32 };
                StatCell {
                    level: level.min(4),
                    label: if date > today { String::new() } else { format!("{}: {} words", date.format("%a %-d %b"), thousands(u64::from(words))) }.into(),
                }
            })
            .collect();
        ui.set_stats_cells(ModelRc::new(VecModel::from(cells)));
        let months: Vec<slint::SharedString> = month_labels(start, WEEKS).into_iter().map(Into::into).collect();
        ui.set_stats_months(ModelRc::new(VecModel::from(months)));
        ui.set_stats_open(true);
    }
}

/// A count with thousands commas: 23100 → "23,100".
fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// For each week of the heatmap (starting on Monday `start`): the month's short name on the first
/// week that starts in a new month, empty otherwise, so the names sit above where each month begins.
fn month_labels(start: NaiveDate, weeks: i64) -> Vec<String> {
    (0..weeks)
        .map(|week| {
            let monday = start + Duration::weeks(week);
            let new_month = week == 0 || (monday - Duration::weeks(1)).month() != monday.month();
            if new_month { monday.format("%b").to_string() } else { String::new() }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_get_thousands_commas() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(2300), "2,300");
        assert_eq!(thousands(23100), "23,100");
        assert_eq!(thousands(1_234_567), "1,234,567");
    }

    #[test]
    fn month_names_mark_where_months_start() {
        // Mondays: 31 Aug, 7 Sep, 14 Sep, 21 Sep, 28 Sep, 5 Oct.
        let start = NaiveDate::from_ymd_opt(2026, 8, 31).unwrap();
        assert_eq!(month_labels(start, 6), vec!["Aug", "Sep", "", "", "", "Oct"]);
        assert!(month_labels(start, 0).is_empty());
    }
}
