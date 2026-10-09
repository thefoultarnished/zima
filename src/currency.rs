//! `@curr` / `@currency`: convert money between currencies with today's exchange rates.
//!
//! ```text
//! @curr 100 usd to inr          100 USD = 8,350.20 INR
//! @currency 50 euros in yen     names, codes and symbols ($, €, £, ¥, ₹) all work
//! ```
//! Rates come from ExchangeRate-API's free open service (Frankfurter as a backup), are downloaded
//! at most once a day, and are kept in `rates.json` next to the app's own files, so conversions work
//! offline. Only "today's rates" is asked for: no amounts and nothing about the user is sent.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// Today's rates, as units of each currency per US dollar.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rates {
    /// When Zima downloaded them (unix ms).
    pub fetched: i64,
    /// When the provider last updated them (unix ms).
    pub updated: i64,
    pub per_usd: BTreeMap<String, f64>,
}

impl Rates {
    /// Older than a day: worth downloading again (the old ones still work meanwhile).
    pub fn stale(&self, now: i64) -> bool {
        now - self.fetched > DAY_MS
    }
}

/// What `@curr` asked for: an amount and two currency codes (not yet checked against the rates).
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub amount: f64,
    pub from: String,
    pub to: String,
}

/// Names, plurals and symbols people use, as currency codes. Any other three-letter code is used as typed.
const NAMES: &[(&[&str], &str)] = &[
    (&["$", "dollar", "us dollar", "usd", "buck"], "USD"),
    (&["€", "euro"], "EUR"),
    (&["£", "pound", "sterling", "british pound", "pound sterling", "quid"], "GBP"),
    (&["¥", "yen", "japanese yen"], "JPY"),
    (&["₹", "rupee", "indian rupee", "rs"], "INR"),
    (&["yuan", "renminbi", "rmb", "chinese yuan"], "CNY"),
    (&["₩", "won", "korean won", "south korean won"], "KRW"),
    (&["₽", "ruble", "rouble", "russian ruble"], "RUB"),
    (&["swiss franc", "franc"], "CHF"),
    (&["canadian dollar"], "CAD"),
    (&["australian dollar", "aussie dollar"], "AUD"),
    (&["new zealand dollar", "kiwi dollar"], "NZD"),
    (&["singapore dollar"], "SGD"),
    (&["hong kong dollar"], "HKD"),
    (&["taiwan dollar", "new taiwan dollar"], "TWD"),
    (&["mexican peso", "peso"], "MXN"),
    (&["philippine peso"], "PHP"),
    (&["brazilian real", "real", "reais"], "BRL"),
    (&["₺", "lira", "turkish lira"], "TRY"),
    (&["dirham", "uae dirham"], "AED"),
    (&["riyal", "saudi riyal"], "SAR"),
    (&["ringgit"], "MYR"),
    (&["baht", "thai baht"], "THB"),
    (&["rand"], "ZAR"),
    (&["swedish krona", "krona"], "SEK"),
    (&["norwegian krone"], "NOK"),
    (&["danish krone", "krone"], "DKK"),
    (&["zloty"], "PLN"),
    (&["₪", "shekel", "sheqel"], "ILS"),
    (&["taka"], "BDT"),
    (&["pakistani rupee"], "PKR"),
    (&["nepali rupee", "nepalese rupee"], "NPR"),
    (&["sri lankan rupee"], "LKR"),
    (&["₫", "dong"], "VND"),
    (&["rupiah"], "IDR"),
    (&["₦", "naira"], "NGN"),
    (&["kenyan shilling", "shilling"], "KES"),
    (&["forint"], "HUF"),
    (&["koruna", "czech koruna"], "CZK"),
    (&["hryvnia"], "UAH"),
];

/// Words that carry no meaning here: "convert 100 dollars to rupees", "how much is 5 euro in yen".
const FILLER: &[&str] = &["convert", "what", "what's", "whats", "is", "how", "much", "many", "the", "worth", "of", "are", "a", "an"];

/// Read a request like "100 usd to inr", "$5 in rupees" or "usd to eur" (an amount of 1).
pub fn parse(request: &str) -> Option<Request> {
    let text = request.to_lowercase().replace(['?'], " ").replace("->", " to ").replace(['\u{2192}', '='], " to ");
    let words: Vec<String> = split_numbers(&text).into_iter().filter(|w| !FILLER.contains(&w.as_str())).collect();
    let at = words.iter().rposition(|w| matches!(w.as_str(), "to" | "into" | "in" | "as"))?;
    let (left, right) = (&words[..at], &words[at + 1..]);
    let mut amount = None;
    let mut name = Vec::new();
    for word in left {
        match number(word) {
            Some(n) if amount.is_none() => amount = Some(n),
            _ => name.push(word.as_str()),
        }
    }
    let right: Vec<&str> = right.iter().map(String::as_str).collect();
    Some(Request { amount: amount.unwrap_or(1.0), from: code(&name.join(" "))?, to: code(&right.join(" "))? })
}

/// Words, with numbers split from what's stuck to them ("$100" → "$", "100"; "100usd" → "100", "usd")
/// and thousands commas dropped ("1,000.50" → "1000.50").
fn split_numbers(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    for raw in text.split_whitespace() {
        let mut word = String::new();
        let mut in_number = false;
        let chars: Vec<char> = raw.chars().collect();
        for (i, &c) in chars.iter().enumerate() {
            // A comma inside a number ("1,000") is a thousands separator; anywhere else it's punctuation.
            if c == ',' {
                if !(in_number && chars.get(i + 1).is_some_and(char::is_ascii_digit)) && !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                    in_number = false;
                }
                continue;
            }
            let digit = c.is_ascii_digit() || (c == '.' && in_number);
            if !word.is_empty() && digit != in_number {
                words.push(std::mem::take(&mut word));
            }
            in_number = digit;
            word.push(c);
        }
        if !word.is_empty() {
            words.push(word);
        }
    }
    words
}

fn number(word: &str) -> Option<f64> {
    let n: f64 = word.parse().ok()?;
    (n.is_finite() && n >= 0.0).then_some(n)
}

/// A currency code from what was typed: a name (singular or plural), a symbol, or a three-letter code.
fn code(name: &str) -> Option<String> {
    let name = name.trim();
    let singular = name.strip_suffix('s').unwrap_or(name);
    for candidate in [name, singular] {
        if let Some((_, code)) = NAMES.iter().find(|(names, _)| names.contains(&candidate)) {
            return Some((*code).to_string());
        }
    }
    (name.len() == 3 && name.chars().all(|c| c.is_ascii_alphabetic())).then(|| name.to_ascii_uppercase())
}

/// The line `@curr` becomes: "100 USD = 8,350.20 INR", with the rates' date when they're more than
/// two days old. `Err` names a currency the rates don't have.
pub fn convert(request: &Request, rates: &Rates, now: i64) -> Result<String, String> {
    let rate = |code: &str| rates.per_usd.get(code).copied().filter(|r| *r > 0.0).ok_or_else(|| code.to_string());
    let (from, to) = (rate(&request.from)?, rate(&request.to)?);
    let result = request.amount / from * to;
    let age = if now - rates.updated > 2 * DAY_MS {
        chrono::DateTime::from_timestamp_millis(rates.updated).map(|d| format!(" (rates from {})", d.format("%-d %b"))).unwrap_or_default()
    } else {
        String::new()
    };
    Ok(format!("{} {} = {} {}{age}", amount_text(request.amount), request.from, money(result), request.to))
}

/// An amount as typed: no ".00" on whole numbers.
fn amount_text(amount: f64) -> String {
    let text = money(amount);
    text.strip_suffix(".00").map(str::to_string).unwrap_or(text)
}

/// Money with thousands commas and two decimals; small amounts get more ("0.0123").
fn money(value: f64) -> String {
    if value != 0.0 && value.abs() < 0.01 {
        // Enough decimals to show four significant digits.
        let decimals = (-value.abs().log10()).ceil() as usize + 3;
        return format!("{value:.decimals$}");
    }
    let text = format!("{value:.2}");
    let (whole, cents) = text.split_once('.').unwrap_or((&text, "00"));
    let (sign, digits) = whole.strip_prefix('-').map_or(("", whole), |d| ("-", d));
    let mut grouped = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(c);
    }
    format!("{sign}{grouped}.{cents}")
}

/// `%APPDATA%\Zima\rates.json` (or under `$ZIMA_DATA_DIR`): local only, never synced.
fn cache_path() -> Option<PathBuf> {
    Some(crate::store::home_dir().ok()?.join("rates.json"))
}

/// The saved rates, if any. A damaged file is just a cache: it's replaced by the next download.
pub fn load() -> Option<Rates> {
    serde_json::from_str(&std::fs::read_to_string(cache_path()?).ok()?).ok()
}

fn save(rates: &Rates) -> std::io::Result<()> {
    let path = cache_path().ok_or_else(|| std::io::Error::other("no data folder"))?;
    crate::store::write_atomic(&path, &serde_json::to_vec_pretty(rates).map_err(std::io::Error::other)?)
}

/// Download today's rates and save them. Runs a network request: call from a background thread.
pub fn download(now: i64) -> Result<Rates, String> {
    let agent = crate::system::http_agent();
    let get = |url: &str| -> Result<serde_json::Value, String> {
        let mut response = agent.get(url).call().map_err(|e| e.to_string())?;
        serde_json::from_str(&response.body_mut().read_to_string().map_err(|e| e.to_string())?).map_err(|e| e.to_string())
    };
    let rates = get("https://open.er-api.com/v6/latest/USD")
        .and_then(|json| from_exchangerate_api(&json, now))
        .or_else(|first| get("https://api.frankfurter.dev/v1/latest?base=USD").and_then(|json| from_frankfurter(&json, now)).map_err(|_| first))?;
    if let Err(e) = save(&rates) {
        eprintln!("couldn't save exchange rates: {e}");
    }
    Ok(rates)
}

fn rates_object(json: &serde_json::Value) -> Result<BTreeMap<String, f64>, String> {
    let rates: BTreeMap<String, f64> = json["rates"]
        .as_object()
        .ok_or("no rates in the answer")?
        .iter()
        .filter_map(|(code, rate)| Some((code.to_ascii_uppercase(), rate.as_f64()?)))
        .collect();
    if rates.is_empty() { Err("no rates in the answer".into()) } else { Ok(rates) }
}

/// `open.er-api.com/v6/latest/USD`: `{"result": "success", "time_last_update_unix": …, "rates": {"USD": 1, …}}`.
fn from_exchangerate_api(json: &serde_json::Value, now: i64) -> Result<Rates, String> {
    if json["result"] != "success" {
        return Err(format!("the rates service said {}", json["error-type"].as_str().unwrap_or("no")));
    }
    let updated = json["time_last_update_unix"].as_i64().map_or(now, |s| s * 1000);
    Ok(Rates { fetched: now, updated, per_usd: rates_object(json)? })
}

/// `api.frankfurter.dev/v1/latest?base=USD`: `{"base": "USD", "date": "2026-10-09", "rates": {"EUR": 0.89, …}}`.
fn from_frankfurter(json: &serde_json::Value, now: i64) -> Result<Rates, String> {
    let mut per_usd = rates_object(json)?;
    per_usd.insert("USD".into(), 1.0);
    let updated = json["date"]
        .as_str()
        .and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .map_or(now, |d| d.and_utc().timestamp_millis());
    Ok(Rates { fetched: now, updated, per_usd })
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_791_504_151_000; // 9 Oct 2026

    fn rates() -> Rates {
        let per_usd = [("USD", 1.0), ("INR", 83.5), ("EUR", 0.9), ("JPY", 150.0), ("GBP", 0.75), ("BTC", 0.0)]
            .into_iter()
            .map(|(c, r)| (c.to_string(), r))
            .collect();
        Rates { fetched: NOW, updated: NOW, per_usd }
    }

    fn run(request: &str) -> Result<String, String> {
        convert(&parse(request).ok_or("not understood")?, &rates(), NOW)
    }

    #[test]
    fn codes_names_and_symbols() {
        assert_eq!(run("100 usd to inr").unwrap(), "100 USD = 8,350.00 INR");
        assert_eq!(run("50 euros in yen").unwrap(), "50 EUR = 8,333.33 JPY");
        assert_eq!(run("$100 to rupees").unwrap(), "100 USD = 8,350.00 INR");
        assert_eq!(run("₹500 -> £").unwrap(), "500 INR = 4.49 GBP");
        assert_eq!(run("convert 1,250.50 Pounds into Dollars").unwrap(), "1,250.50 GBP = 1,667.33 USD");
        assert_eq!(run("how much is 20usd in eur?").unwrap(), "20 USD = 18.00 EUR");
    }

    #[test]
    fn amount_defaults_to_one() {
        assert_eq!(run("usd to inr").unwrap(), "1 USD = 83.50 INR");
        assert_eq!(run("yen to usd").unwrap(), "1 JPY = 0.006667 USD");
    }

    #[test]
    fn not_understood_or_unknown() {
        assert_eq!(parse(""), None);
        assert_eq!(parse("100 usd"), None);
        assert_eq!(parse("100 usd to"), None);
        assert_eq!(parse("100 bananas to inr"), None);
        // A real-looking code the rates don't have, or a broken rate.
        assert_eq!(run("100 xyz to inr"), Err("XYZ".into()));
        assert_eq!(run("1 btc to usd"), Err("BTC".into()));
    }

    #[test]
    fn money_formatting() {
        assert_eq!(money(0.0), "0.00");
        assert_eq!(money(999.999), "1,000.00");
        assert_eq!(money(1234567.891), "1,234,567.89");
        assert_eq!(money(0.00012346), "0.0001235");
        assert_eq!(money(-1500.0), "-1,500.00");
        assert_eq!(amount_text(100.0), "100");
        assert_eq!(amount_text(99.5), "99.50");
    }

    #[test]
    fn old_rates_say_their_date() {
        let mut old = rates();
        old.updated = NOW - 5 * DAY_MS;
        let request = parse("1 usd to inr").unwrap();
        assert_eq!(convert(&request, &old, NOW).unwrap(), "1 USD = 83.50 INR (rates from 4 Oct)");
        assert!(!old.stale(NOW));
        assert!(old.stale(NOW + 2 * DAY_MS));
    }

    #[test]
    fn reads_both_services() {
        let er: serde_json::Value = serde_json::from_str(r#"{"result":"success","time_last_update_unix":1791504151,"rates":{"USD":1,"INR":83.5}}"#).unwrap();
        let rates = from_exchangerate_api(&er, NOW).unwrap();
        assert_eq!((rates.per_usd["INR"], rates.updated), (83.5, 1_791_504_151_000));
        let failed: serde_json::Value = serde_json::from_str(r#"{"result":"error","error-type":"unsupported-code"}"#).unwrap();
        assert!(from_exchangerate_api(&failed, NOW).is_err());

        let fr: serde_json::Value = serde_json::from_str(r#"{"base":"USD","date":"2026-10-09","rates":{"EUR":0.89}}"#).unwrap();
        let rates = from_frankfurter(&fr, NOW).unwrap();
        assert_eq!((rates.per_usd["EUR"], rates.per_usd["USD"]), (0.89, 1.0));
        let empty: serde_json::Value = serde_json::from_str(r#"{"rates":{}}"#).unwrap();
        assert!(from_frankfurter(&empty, NOW).is_err());
    }
}
