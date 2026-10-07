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
            today: words_on(today) as i32,
            week: week as i32,
            streak,
            best: best as i32,
            notes: listed.len() as i32,
            words: listed.iter().map(|n| n.body.split_whitespace().count()).sum::<usize>() as i32,
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
                    label: if date > today { String::new() } else { format!("{}: {words} words", date.format("%a %-d %b")) }.into(),
                }
            })
            .collect();
        ui.set_stats_cells(ModelRc::new(VecModel::from(cells)));
        ui.set_stats_open(true);
    }
}
