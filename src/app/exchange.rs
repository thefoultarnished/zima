//! `@curr` / `@currency` in a note: converting with the saved rates, and downloading new ones in
//! the background (the conversion logic is in `crate::currency`).

use std::sync::mpsc::{self, TryRecvError};
use std::time::Duration;

use slint::{Timer, TimerMode};

use super::{App, with_app};
use crate::currency::{self, Rates, Request};
use crate::model::{NoteId, now_ms};

/// How often to check whether a download finished. Only runs while one does.
const POLL: Duration = Duration::from_millis(200);

#[derive(Default)]
pub struct Exchange {
    /// Read from `rates.json` on first use, then kept up to date by downloads.
    rates: Option<Rates>,
    loaded: bool,
    downloading: bool,
    poll: Timer,
    /// Conversions waiting for the first rates: (note, the `@curr` line as typed, what it asked).
    waiting: Vec<(NoteId, String, Request)>,
}

impl App {
    /// Enter after an `@curr …` line in note `note`. Returns the line to put in its place, or `None`
    /// when there's nothing to replace yet: a message was shown, or the rates are being downloaded
    /// and the line is replaced once they arrive.
    pub(super) fn currency_line(&mut self, note: NoteId, line: &str, request: &str) -> Option<String> {
        let Some(request) = currency::parse(request) else {
            self.toast("Try \u{201c}@curr 100 usd to inr\u{201d} or \u{201c}@currency 50 euros in yen\u{201d}.", true);
            return None;
        };
        if !self.exchange.loaded {
            self.exchange.loaded = true;
            self.exchange.rates = currency::load();
        }
        let now = now_ms();
        let Some(rates) = &self.exchange.rates else {
            self.exchange.waiting.push((note, line.to_string(), request));
            self.download_rates();
            self.toast("Getting today\u{2019}s exchange rates\u{2026}", false);
            return None;
        };
        let result = currency::convert(&request, rates, now);
        // Old rates still answer right away; fresh ones are fetched for next time.
        if rates.stale(now) {
            self.download_rates();
        }
        match result {
            Ok(text) => Some(text),
            Err(code) => {
                self.toast(&format!("No exchange rate for {code}. Use a currency name or its 3-letter code, like USD or INR."), true);
                None
            }
        }
    }

    /// Download today's rates on a background thread, unless a download is already running.
    fn download_rates(&mut self) {
        if self.exchange.downloading {
            return;
        }
        self.exchange.downloading = true;
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(currency::download(now_ms()));
        });
        let app = self.this.clone();
        self.exchange.poll.start(TimerMode::Repeated, POLL, move || {
            let result = match receiver.try_recv() {
                Ok(result) => result,
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => Err("the download stopped".into()),
            };
            if let Some(app) = app.upgrade() {
                with_app(&app, move |a| a.rates_arrived(result));
            }
        });
    }

    fn rates_arrived(&mut self, result: Result<Rates, String>) {
        self.exchange.poll.stop();
        self.exchange.downloading = false;
        let waiting = std::mem::take(&mut self.exchange.waiting);
        match result {
            Ok(rates) => {
                self.exchange.rates = Some(rates);
                for (note, line, request) in waiting {
                    self.finish_currency(note, &line, &request);
                }
            }
            // A background refresh that fails stays quiet: the old rates keep working.
            Err(e) if !waiting.is_empty() => self.toast(&format!("Couldn\u{2019}t get exchange rates: {e}"), true),
            Err(e) => eprintln!("couldn't refresh exchange rates: {e}"),
        }
    }

    /// The rates arrived for a conversion that was waiting: put the answer in place of its line,
    /// or show it if the line was changed in the meantime.
    fn finish_currency(&mut self, note: NoteId, line: &str, request: &Request) {
        let Some(rates) = &self.exchange.rates else { return };
        let text = match currency::convert(request, rates, now_ms()) {
            Ok(text) => text,
            Err(code) => {
                self.toast(&format!("No exchange rate for {code}. Use a currency name or its 3-letter code, like USD or INR."), true);
                return;
            }
        };
        let body = self.find(note).map(|n| n.body.clone()).unwrap_or_default();
        let Some(new_body) = replace_line(&body, line, &text) else {
            self.toast(&text, false);
            return;
        };
        if self.state.current == Some(note) {
            self.replace_body(new_body, None);
        } else {
            self.set_body_of(note, new_body);
        }
    }
}

/// `body` with its first line that reads exactly `line` replaced by `with`.
fn replace_line(body: &str, line: &str, with: &str) -> Option<String> {
    let mut lines: Vec<&str> = body.split('\n').collect();
    let at = lines.iter().position(|l| *l == line)?;
    lines[at] = with;
    Some(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_only_the_matching_line() {
        assert_eq!(replace_line("a\n@curr 1 usd to inr\nb", "@curr 1 usd to inr", "1 USD = 83.50 INR").unwrap(), "a\n1 USD = 83.50 INR\nb");
        // Only the first copy, and only whole lines.
        assert_eq!(replace_line("@curr x\n@curr x", "@curr x", "y").unwrap(), "y\n@curr x");
        assert_eq!(replace_line("note: @curr x", "@curr x", "y"), None);
        assert_eq!(replace_line("", "@curr x", "y"), None);
    }
}
