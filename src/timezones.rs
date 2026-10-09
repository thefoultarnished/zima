//! `@time`: convert a time between time zones, written the way people say it.
//!
//! ```text
//! @time 3pm IST to PST            3:00 PM IST = 2:30 AM PDT (same day or not)
//! @time India time to Estonia     a country, a city, a short name or "UTC+5:30"
//! @time tokyo                     the time in Tokyo now
//! @time 9am london                9am in London, in your own time
//! ```
//! Zones come from `chrono-tz` (the IANA database), so summer time is right on any date.

use chrono::{DateTime, Duration, FixedOffset, Local, NaiveDateTime, NaiveTime, TimeZone, Utc};
use chrono_tz::{TZ_VARIANTS, Tz};

/// Where a time is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Zone {
    Named(Tz),
    Fixed(FixedOffset),
    /// This PC's own time zone.
    Here,
}

/// How a zone is named in the answer.
#[derive(Debug, Clone, PartialEq)]
enum Label {
    /// The zone's own short name at that moment ("IST", "PDT"), for zones typed as short names.
    Short,
    /// A country or city, plus its short name when it has a real one: "Estonia (EEST)".
    Place(String),
    /// Exactly as given ("UTC", "UTC+5:30").
    Plain(String),
}

#[derive(Debug, Clone, PartialEq)]
struct Side {
    zone: Zone,
    label: Label,
}

/// Short names. Where one means several zones, the most common one wins (IST is India).
const SHORT_NAMES: &[(&str, &str)] = &[
    ("ist", "Asia/Kolkata"),
    ("pst", "America/Los_Angeles"),
    ("pdt", "America/Los_Angeles"),
    ("pt", "America/Los_Angeles"),
    ("pct", "America/Los_Angeles"),
    ("mst", "America/Denver"),
    ("mdt", "America/Denver"),
    ("mt", "America/Denver"),
    ("cst", "America/Chicago"),
    ("cdt", "America/Chicago"),
    ("ct", "America/Chicago"),
    ("est", "America/New_York"),
    ("edt", "America/New_York"),
    ("et", "America/New_York"),
    ("akst", "America/Anchorage"),
    ("akdt", "America/Anchorage"),
    ("hst", "Pacific/Honolulu"),
    ("ast", "America/Halifax"),
    ("adt", "America/Halifax"),
    ("nst", "America/St_Johns"),
    ("ndt", "America/St_Johns"),
    ("bst", "Europe/London"),
    ("wet", "Europe/Lisbon"),
    ("west", "Europe/Lisbon"),
    ("cet", "Europe/Paris"),
    ("cest", "Europe/Paris"),
    ("eet", "Europe/Athens"),
    ("eest", "Europe/Athens"),
    ("msk", "Europe/Moscow"),
    ("gst", "Asia/Dubai"),
    ("pkt", "Asia/Karachi"),
    ("npt", "Asia/Kathmandu"),
    ("ict", "Asia/Bangkok"),
    ("wib", "Asia/Jakarta"),
    ("sgt", "Asia/Singapore"),
    ("hkt", "Asia/Hong_Kong"),
    ("pht", "Asia/Manila"),
    ("myt", "Asia/Kuala_Lumpur"),
    ("jst", "Asia/Tokyo"),
    ("kst", "Asia/Seoul"),
    ("awst", "Australia/Perth"),
    ("acst", "Australia/Adelaide"),
    ("acdt", "Australia/Adelaide"),
    ("aest", "Australia/Sydney"),
    ("aedt", "Australia/Sydney"),
    ("nzst", "Pacific/Auckland"),
    ("nzdt", "Pacific/Auckland"),
    ("sast", "Africa/Johannesburg"),
    ("wat", "Africa/Lagos"),
    ("cat", "Africa/Maputo"),
    ("eat", "Africa/Nairobi"),
    ("art", "America/Argentina/Buenos_Aires"),
    ("brt", "America/Sao_Paulo"),
    ("clt", "America/Santiago"),
    ("cot", "America/Bogota"),
    ("pet", "America/Lima"),
    ("irst", "Asia/Tehran"),
    ("idt", "Asia/Jerusalem"),
    ("aft", "Asia/Kabul"),
    ("mmt", "Asia/Yangon"),
    ("pacific", "America/Los_Angeles"),
    ("mountain", "America/Denver"),
    ("central", "America/Chicago"),
    ("eastern", "America/New_York"),
];

/// Countries (by their capital's zone, or the biggest one) and places the zone database doesn't
/// name itself: (names, how the answer shows it, zone).
const PLACES: &[(&[&str], &str, &str)] = &[
    (&["afghanistan"], "Afghanistan", "Asia/Kabul"),
    (&["albania"], "Albania", "Europe/Tirane"),
    (&["algeria"], "Algeria", "Africa/Algiers"),
    (&["andorra"], "Andorra", "Europe/Andorra"),
    (&["angola"], "Angola", "Africa/Luanda"),
    (&["argentina"], "Argentina", "America/Argentina/Buenos_Aires"),
    (&["armenia"], "Armenia", "Asia/Yerevan"),
    (&["australia", "aus", "canberra"], "Australia", "Australia/Sydney"),
    (&["austria"], "Austria", "Europe/Vienna"),
    (&["azerbaijan"], "Azerbaijan", "Asia/Baku"),
    (&["bahamas"], "Bahamas", "America/Nassau"),
    (&["bahrain"], "Bahrain", "Asia/Bahrain"),
    (&["bangladesh"], "Bangladesh", "Asia/Dhaka"),
    (&["barbados"], "Barbados", "America/Barbados"),
    (&["belarus"], "Belarus", "Europe/Minsk"),
    (&["belgium"], "Belgium", "Europe/Brussels"),
    (&["belize"], "Belize", "America/Belize"),
    (&["benin"], "Benin", "Africa/Porto-Novo"),
    (&["bhutan"], "Bhutan", "Asia/Thimphu"),
    (&["bolivia"], "Bolivia", "America/La_Paz"),
    (&["bosnia", "bosnia and herzegovina"], "Bosnia", "Europe/Sarajevo"),
    (&["botswana"], "Botswana", "Africa/Gaborone"),
    (&["brazil", "rio", "rio de janeiro", "brasilia"], "Brazil", "America/Sao_Paulo"),
    (&["brunei"], "Brunei", "Asia/Brunei"),
    (&["bulgaria"], "Bulgaria", "Europe/Sofia"),
    (&["burkina faso"], "Burkina Faso", "Africa/Ouagadougou"),
    (&["burundi"], "Burundi", "Africa/Bujumbura"),
    (&["cambodia"], "Cambodia", "Asia/Phnom_Penh"),
    (&["cameroon"], "Cameroon", "Africa/Douala"),
    (&["canada", "ottawa", "montreal"], "Canada", "America/Toronto"),
    (&["cape verde"], "Cape Verde", "Atlantic/Cape_Verde"),
    (&["central african republic"], "Central African Republic", "Africa/Bangui"),
    (&["chad"], "Chad", "Africa/Ndjamena"),
    (&["chile"], "Chile", "America/Santiago"),
    (&["china", "prc", "beijing", "shenzhen", "guangzhou"], "China", "Asia/Shanghai"),
    (&["colombia"], "Colombia", "America/Bogota"),
    (&["congo"], "Congo", "Africa/Brazzaville"),
    (&["dr congo", "drc", "democratic republic of the congo"], "DR Congo", "Africa/Kinshasa"),
    (&["costa rica"], "Costa Rica", "America/Costa_Rica"),
    (&["croatia"], "Croatia", "Europe/Zagreb"),
    (&["cuba"], "Cuba", "America/Havana"),
    (&["cyprus"], "Cyprus", "Asia/Nicosia"),
    (&["czechia", "czech republic"], "Czechia", "Europe/Prague"),
    (&["denmark"], "Denmark", "Europe/Copenhagen"),
    (&["djibouti"], "Djibouti", "Africa/Djibouti"),
    (&["dominican republic"], "Dominican Republic", "America/Santo_Domingo"),
    (&["ecuador"], "Ecuador", "America/Guayaquil"),
    (&["egypt"], "Egypt", "Africa/Cairo"),
    (&["el salvador"], "El Salvador", "America/El_Salvador"),
    (&["estonia"], "Estonia", "Europe/Tallinn"),
    (&["eswatini", "swaziland"], "Eswatini", "Africa/Mbabane"),
    (&["ethiopia"], "Ethiopia", "Africa/Addis_Ababa"),
    (&["fiji"], "Fiji", "Pacific/Fiji"),
    (&["finland"], "Finland", "Europe/Helsinki"),
    (&["france"], "France", "Europe/Paris"),
    (&["gabon"], "Gabon", "Africa/Libreville"),
    (&["gambia"], "Gambia", "Africa/Banjul"),
    (&["georgia"], "Georgia", "Asia/Tbilisi"),
    (&["germany", "munich", "frankfurt", "hamburg"], "Germany", "Europe/Berlin"),
    (&["ghana"], "Ghana", "Africa/Accra"),
    (&["greece"], "Greece", "Europe/Athens"),
    (&["greenland"], "Greenland", "America/Nuuk"),
    (&["guatemala"], "Guatemala", "America/Guatemala"),
    (&["guinea"], "Guinea", "Africa/Conakry"),
    (&["haiti"], "Haiti", "America/Port-au-Prince"),
    (&["honduras"], "Honduras", "America/Tegucigalpa"),
    (&["hungary"], "Hungary", "Europe/Budapest"),
    (&["iceland"], "Iceland", "Atlantic/Reykjavik"),
    (
        &["india", "delhi", "new delhi", "mumbai", "bombay", "bangalore", "bengaluru", "chennai", "hyderabad", "pune"],
        "India",
        "Asia/Kolkata",
    ),
    (&["indonesia"], "Indonesia", "Asia/Jakarta"),
    (&["bali"], "Bali", "Asia/Makassar"),
    (&["iran"], "Iran", "Asia/Tehran"),
    (&["iraq"], "Iraq", "Asia/Baghdad"),
    (&["ireland"], "Ireland", "Europe/Dublin"),
    (&["israel"], "Israel", "Asia/Jerusalem"),
    (&["italy", "milan"], "Italy", "Europe/Rome"),
    (&["ivory coast", "cote d'ivoire"], "Ivory Coast", "Africa/Abidjan"),
    (&["jamaica"], "Jamaica", "America/Jamaica"),
    (&["japan", "osaka", "kyoto"], "Japan", "Asia/Tokyo"),
    (&["jordan"], "Jordan", "Asia/Amman"),
    (&["kazakhstan"], "Kazakhstan", "Asia/Almaty"),
    (&["kenya"], "Kenya", "Africa/Nairobi"),
    (&["kosovo"], "Kosovo", "Europe/Belgrade"),
    (&["kuwait"], "Kuwait", "Asia/Kuwait"),
    (&["kyrgyzstan"], "Kyrgyzstan", "Asia/Bishkek"),
    (&["laos"], "Laos", "Asia/Vientiane"),
    (&["latvia"], "Latvia", "Europe/Riga"),
    (&["lebanon"], "Lebanon", "Asia/Beirut"),
    (&["lesotho"], "Lesotho", "Africa/Maseru"),
    (&["liberia"], "Liberia", "Africa/Monrovia"),
    (&["libya"], "Libya", "Africa/Tripoli"),
    (&["liechtenstein"], "Liechtenstein", "Europe/Vaduz"),
    (&["lithuania"], "Lithuania", "Europe/Vilnius"),
    (&["luxembourg"], "Luxembourg", "Europe/Luxembourg"),
    (&["madagascar"], "Madagascar", "Indian/Antananarivo"),
    (&["malawi"], "Malawi", "Africa/Blantyre"),
    (&["malaysia"], "Malaysia", "Asia/Kuala_Lumpur"),
    (&["maldives"], "Maldives", "Indian/Maldives"),
    (&["mali"], "Mali", "Africa/Bamako"),
    (&["malta"], "Malta", "Europe/Malta"),
    (&["mauritania"], "Mauritania", "Africa/Nouakchott"),
    (&["mauritius"], "Mauritius", "Indian/Mauritius"),
    (&["mexico"], "Mexico", "America/Mexico_City"),
    (&["moldova"], "Moldova", "Europe/Chisinau"),
    (&["monaco"], "Monaco", "Europe/Monaco"),
    (&["mongolia"], "Mongolia", "Asia/Ulaanbaatar"),
    (&["montenegro"], "Montenegro", "Europe/Podgorica"),
    (&["morocco"], "Morocco", "Africa/Casablanca"),
    (&["mozambique"], "Mozambique", "Africa/Maputo"),
    (&["myanmar", "burma"], "Myanmar", "Asia/Yangon"),
    (&["namibia"], "Namibia", "Africa/Windhoek"),
    (&["nepal"], "Nepal", "Asia/Kathmandu"),
    (&["netherlands", "holland"], "Netherlands", "Europe/Amsterdam"),
    (&["new zealand", "nz", "wellington"], "New Zealand", "Pacific/Auckland"),
    (&["nicaragua"], "Nicaragua", "America/Managua"),
    (&["niger"], "Niger", "Africa/Niamey"),
    (&["nigeria"], "Nigeria", "Africa/Lagos"),
    (&["north korea"], "North Korea", "Asia/Pyongyang"),
    (&["north macedonia", "macedonia"], "North Macedonia", "Europe/Skopje"),
    (&["norway"], "Norway", "Europe/Oslo"),
    (&["oman"], "Oman", "Asia/Muscat"),
    (&["pakistan", "lahore", "islamabad"], "Pakistan", "Asia/Karachi"),
    (&["palestine"], "Palestine", "Asia/Gaza"),
    (&["panama"], "Panama", "America/Panama"),
    (&["papua new guinea"], "Papua New Guinea", "Pacific/Port_Moresby"),
    (&["paraguay"], "Paraguay", "America/Asuncion"),
    (&["peru"], "Peru", "America/Lima"),
    (&["philippines"], "Philippines", "Asia/Manila"),
    (&["poland"], "Poland", "Europe/Warsaw"),
    (&["portugal"], "Portugal", "Europe/Lisbon"),
    (&["puerto rico"], "Puerto Rico", "America/Puerto_Rico"),
    (&["qatar"], "Qatar", "Asia/Qatar"),
    (&["romania"], "Romania", "Europe/Bucharest"),
    (&["russia", "st petersburg", "saint petersburg"], "Russia", "Europe/Moscow"),
    (&["rwanda"], "Rwanda", "Africa/Kigali"),
    (&["saudi arabia", "saudi", "ksa", "mecca", "jeddah"], "Saudi Arabia", "Asia/Riyadh"),
    (&["senegal"], "Senegal", "Africa/Dakar"),
    (&["serbia"], "Serbia", "Europe/Belgrade"),
    (&["seychelles"], "Seychelles", "Indian/Mahe"),
    (&["sierra leone"], "Sierra Leone", "Africa/Freetown"),
    (&["slovakia"], "Slovakia", "Europe/Bratislava"),
    (&["slovenia"], "Slovenia", "Europe/Ljubljana"),
    (&["somalia"], "Somalia", "Africa/Mogadishu"),
    (&["south africa", "cape town"], "South Africa", "Africa/Johannesburg"),
    (&["south korea", "korea"], "South Korea", "Asia/Seoul"),
    (&["south sudan"], "South Sudan", "Africa/Juba"),
    (&["spain", "barcelona"], "Spain", "Europe/Madrid"),
    (&["sri lanka"], "Sri Lanka", "Asia/Colombo"),
    (&["sudan"], "Sudan", "Africa/Khartoum"),
    (&["suriname"], "Suriname", "America/Paramaribo"),
    (&["sweden"], "Sweden", "Europe/Stockholm"),
    (&["switzerland", "geneva"], "Switzerland", "Europe/Zurich"),
    (&["syria"], "Syria", "Asia/Damascus"),
    (&["taiwan"], "Taiwan", "Asia/Taipei"),
    (&["tajikistan"], "Tajikistan", "Asia/Dushanbe"),
    (&["tanzania"], "Tanzania", "Africa/Dar_es_Salaam"),
    (&["thailand"], "Thailand", "Asia/Bangkok"),
    (&["togo"], "Togo", "Africa/Lome"),
    (&["trinidad", "trinidad and tobago"], "Trinidad and Tobago", "America/Port_of_Spain"),
    (&["tunisia"], "Tunisia", "Africa/Tunis"),
    (&["turkey", "turkiye"], "Turkey", "Europe/Istanbul"),
    (&["turkmenistan"], "Turkmenistan", "Asia/Ashgabat"),
    (&["uganda"], "Uganda", "Africa/Kampala"),
    (&["ukraine", "kyiv", "kiev"], "Ukraine", "Europe/Kyiv"),
    (&["united arab emirates", "uae", "emirates", "abu dhabi"], "UAE", "Asia/Dubai"),
    (
        &["united kingdom", "uk", "britain", "great britain", "england", "scotland", "wales"],
        "UK",
        "Europe/London",
    ),
    (&["united states", "united states of america", "usa", "us", "america"], "US Eastern", "America/New_York"),
    (
        &["washington", "dc", "boston", "miami", "atlanta", "philadelphia", "new jersey", "florida"],
        "US Eastern",
        "America/New_York",
    ),
    (&["dallas", "houston", "austin", "texas", "minneapolis"], "US Central", "America/Chicago"),
    (&["colorado", "salt lake city", "utah"], "US Mountain", "America/Denver"),
    (&["arizona"], "Arizona", "America/Phoenix"),
    (
        &["san francisco", "seattle", "silicon valley", "california", "portland", "las vegas", "san diego"],
        "US Pacific",
        "America/Los_Angeles",
    ),
    (&["hawaii"], "Hawaii", "Pacific/Honolulu"),
    (&["alaska"], "Alaska", "America/Anchorage"),
    (&["calgary"], "Calgary", "America/Edmonton"),
    (&["uruguay"], "Uruguay", "America/Montevideo"),
    (&["uzbekistan"], "Uzbekistan", "Asia/Tashkent"),
    (&["venezuela"], "Venezuela", "America/Caracas"),
    (&["vietnam", "viet nam", "saigon", "hanoi"], "Vietnam", "Asia/Ho_Chi_Minh"),
    (&["yemen"], "Yemen", "Asia/Aden"),
    (&["zambia"], "Zambia", "Africa/Lusaka"),
    (&["zimbabwe"], "Zimbabwe", "Africa/Harare"),
];

/// Words that carry no meaning here: "what time is it in tokyo", "india time", "pacific standard time".
const FILLER: &[&str] = &["what", "what's", "whats", "is", "it", "the", "time", "now", "standard", "daylight", "zone", "o'clock"];

/// The line `@time …` becomes: "3:00 PM IST = 2:30 AM PDT (next day)". `None` when the request
/// can't be understood. `here` is this PC's zone ([`Zone::Here`] in the app; fixed in tests).
pub fn convert(request: &str, now: DateTime<Utc>, here: Zone) -> Option<String> {
    let text = request.to_lowercase().replace(['?', ','], " ").replace("->", " to ").replace('\u{2192}', " to ");
    let words: Vec<&str> = text.split_whitespace().filter(|w| !FILLER.contains(w)).collect();
    let here_side = Side { zone: here, label: Label::Plain("here".into()) };

    // "A to B" (the last "to"), else "A in B" (the last "in"): A is where the time is, B where it's wanted.
    let split = words.iter().rposition(|w| *w == "to").or_else(|| words.iter().rposition(|w| *w == "in"));
    let (time, from, to) = match split {
        Some(at) => {
            let (time, rest) = take_time(&words[..at]);
            let rest: Vec<&str> = rest.into_iter().filter(|w| !matches!(*w, "in" | "at" | "from")).collect();
            let from = if rest.is_empty() { here_side } else { side(&rest.join(" "))? };
            (time, from, side(&words[at + 1..].join(" "))?)
        }
        None => {
            let (time, rest) = take_time(&words);
            if rest.is_empty() {
                return None;
            }
            let zone = side(&rest.join(" "))?;
            // "9am london": 9am there, in your time. "tokyo": now, there.
            if time.is_some() { (time, zone, here_side) } else { (None, here_side, zone) }
        }
    };

    let instant = match time {
        Some(clock) => {
            let today = local_time(from.zone, now).date();
            to_utc(from.zone, today.and_time(clock))?
        }
        None => now,
    };
    let (start, end) = (local_time(from.zone, instant), local_time(to.zone, instant));
    let days = (end.date() - start.date()).num_days();
    let day = match days {
        0 => String::new(),
        1 => " (next day)".into(),
        -1 => " (previous day)".into(),
        n => format!(" ({n:+} days)"),
    };
    Some(format!(
        "{} {} = {} {}{day}",
        clock_text(start),
        name(&from, instant),
        clock_text(end),
        name(&to, instant)
    ))
}

/// A time at the start or end of the words ("3pm", "3 pm", "15:30", "9.30am", "noon"), and the other words.
fn take_time<'a>(words: &[&'a str]) -> (Option<NaiveTime>, Vec<&'a str>) {
    for len in [2, 1] {
        if words.len() >= len {
            if let Some(time) = parse_clock(&words[..len].join(" ")) {
                return (Some(time), words[len..].to_vec());
            }
            if let Some(time) = parse_clock(&words[words.len() - len..].join(" ")) {
                return (Some(time), words[..words.len() - len].to_vec());
            }
        }
    }
    (None, words.to_vec())
}

fn parse_clock(text: &str) -> Option<NaiveTime> {
    let text = text.replace(' ', "").replace('.', ":");
    match text.as_str() {
        "noon" | "midday" => return NaiveTime::from_hms_opt(12, 0, 0),
        "midnight" => return NaiveTime::from_hms_opt(0, 0, 0),
        _ => {}
    }
    let (number, half) = if let Some(n) = text.strip_suffix("am").or_else(|| text.strip_suffix("a:m:")) {
        (n, Some(false))
    } else if let Some(n) = text.strip_suffix("pm").or_else(|| text.strip_suffix("p:m:")) {
        (n, Some(true))
    } else {
        (text.as_str(), None)
    };
    let (hour, minute) = number.split_once(':').unwrap_or((number, "0"));
    let small = |s: &str| (1..=2).contains(&s.len()) && s.chars().all(|c| c.is_ascii_digit());
    if !small(hour) || !(small(minute) || minute == "0") {
        return None;
    }
    // A bare number is only a time with ":" ("9:00"), so "3 tokyo" isn't read as 3 o'clock.
    if half.is_none() && !number.contains(':') {
        return None;
    }
    let (hour, minute): (u32, u32) = (hour.parse().ok()?, minute.parse().ok()?);
    let hour = match half {
        Some(pm) if (1..=12).contains(&hour) => hour % 12 + if pm { 12 } else { 0 },
        Some(_) => return None,
        None => hour,
    };
    NaiveTime::from_hms_opt(hour, minute, 0)
}

/// A zone from what was typed: a short name, "UTC+5:30", a country or a city.
fn side(name: &str) -> Option<Side> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    if matches!(name, "here" | "local" | "my" | "me" | "mine") {
        return Some(Side { zone: Zone::Here, label: Label::Plain("here".into()) });
    }
    if let Some(offset) = utc_offset(name) {
        let label = name.replace(' ', "").to_uppercase();
        return Some(Side { zone: Zone::Fixed(offset), label: Label::Plain(label) });
    }
    if let Some((_, zone)) = SHORT_NAMES.iter().find(|(short, _)| *short == name) {
        return Some(Side { zone: Zone::Named(zone.parse().ok()?), label: Label::Short });
    }
    if let Some((_, shown, zone)) = PLACES.iter().find(|(names, _, _)| names.contains(&name)) {
        return Some(Side { zone: Zone::Named(zone.parse().ok()?), label: Label::Place((*shown).into()) });
    }
    // The zone database: "Europe/Tallinn", or just its city, "tallinn", "new york", "los angeles".
    let city = |tz: &Tz| tz.name().rsplit('/').next().unwrap_or("").replace('_', " ");
    let tz = TZ_VARIANTS
        .iter()
        .find(|tz| tz.name().eq_ignore_ascii_case(name))
        .or_else(|| TZ_VARIANTS.iter().find(|tz| city(tz).eq_ignore_ascii_case(name)))?;
    Some(Side { zone: Zone::Named(*tz), label: Label::Place(city(tz)) })
}

/// "utc", "gmt", "z", "utc+5", "gmt-3", "utc+5:30", "utc +0530".
fn utc_offset(name: &str) -> Option<FixedOffset> {
    let compact = name.replace(' ', "");
    let rest = ["utc", "gmt", "zulu", "z"].iter().find_map(|p| compact.strip_prefix(p))?;
    if rest.is_empty() {
        return FixedOffset::east_opt(0);
    }
    let (sign, digits) = match rest.chars().next()? {
        '+' => (1, &rest[1..]),
        '-' | '\u{2212}' => (-1, rest.trim_start_matches(['-', '\u{2212}'])),
        _ => return None,
    };
    let (hours, minutes) = match digits.split_once(':') {
        Some((h, m)) => (h, m),
        None if digits.len() > 2 => digits.split_at(digits.len() - 2),
        None => (digits, "0"),
    };
    let (hours, minutes): (i32, i32) = (hours.parse().ok()?, minutes.parse().ok()?);
    if hours > 14 || minutes >= 60 {
        return None;
    }
    FixedOffset::east_opt(sign * (hours * 3600 + minutes * 60))
}

/// The wall-clock time in `zone` at `instant`.
fn local_time(zone: Zone, instant: DateTime<Utc>) -> NaiveDateTime {
    match zone {
        Zone::Named(tz) => instant.with_timezone(&tz).naive_local(),
        Zone::Fixed(offset) => instant.with_timezone(&offset).naive_local(),
        Zone::Here => instant.with_timezone(&Local).naive_local(),
    }
}

/// The moment a wall-clock time in `zone` happens. A time skipped by the clocks going forward
/// counts as the hour after it.
fn to_utc(zone: Zone, time: NaiveDateTime) -> Option<DateTime<Utc>> {
    let at = |t: NaiveDateTime| match zone {
        Zone::Named(tz) => tz.from_local_datetime(&t).earliest().map(|d| d.with_timezone(&Utc)),
        Zone::Fixed(offset) => offset.from_local_datetime(&t).earliest().map(|d| d.with_timezone(&Utc)),
        Zone::Here => Local.from_local_datetime(&t).earliest().map(|d| d.with_timezone(&Utc)),
    };
    at(time).or_else(|| at(time + Duration::hours(1)))
}

fn clock_text(time: NaiveDateTime) -> String {
    time.format("%-I:%M %p").to_string()
}

/// How a side is named in the answer, at that moment (so PST shows as PDT in summer).
fn name(side: &Side, instant: DateTime<Utc>) -> String {
    let short = match side.zone {
        Zone::Named(tz) => instant.with_timezone(&tz).format("%Z").to_string(),
        _ => String::new(),
    };
    // Some zones have no letters, only "+04".
    let real_short = !short.is_empty() && short.chars().all(|c| c.is_ascii_alphabetic());
    match &side.label {
        Label::Short if real_short => short,
        Label::Short => format!("UTC{short}"),
        Label::Place(place) if real_short => format!("{place} ({short})"),
        Label::Place(place) => place.clone(),
        Label::Plain(text) => text.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A summer and a winter moment, and "here" fixed at UTC+2 so tests don't depend on this PC.
    fn july() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, 15, 9, 0, 0).unwrap()
    }
    fn january() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, 15, 9, 0, 0).unwrap()
    }
    fn here() -> Zone {
        Zone::Fixed(FixedOffset::east_opt(2 * 3600).unwrap())
    }
    fn run(request: &str, now: DateTime<Utc>) -> Option<String> {
        convert(request, now, here())
    }

    #[test]
    fn short_names_both_ways() {
        // India doesn't change its clocks; Los Angeles is on summer time in July.
        assert_eq!(run("3pm IST to PST", july()).unwrap(), "3:00 PM IST = 2:30 AM PDT");
        assert_eq!(run("3pm IST to PST", january()).unwrap(), "3:00 PM IST = 1:30 AM PST");
        assert_eq!(run("9am pst to ist", january()).unwrap(), "9:00 AM PST = 10:30 PM IST");
        // "PCT" isn't a real name, but people type it for Pacific time.
        assert_eq!(run("3pm IST to PCT", january()).unwrap(), "3:00 PM IST = 1:30 AM PST");
    }

    #[test]
    fn countries_in_plain_words() {
        assert_eq!(run("India time to Estonia time", january()).unwrap(), "2:30 PM India (IST) = 11:00 AM Estonia (EET)");
        assert_eq!(run("3pm India time to Estonia time", july()).unwrap(), "3:00 PM India (IST) = 12:30 PM Estonia (EEST)");
        assert_eq!(run("10:30 in new york to london", july()).unwrap(), "10:30 AM New York (EDT) = 3:30 PM London (BST)");
        assert_eq!(run("noon japan -> usa", january()).unwrap(), "12:00 PM Japan (JST) = 10:00 PM US Eastern (EST) (previous day)");
    }

    #[test]
    fn one_place_uses_your_own_time() {
        // Just a place: the time there now. 9:00 UTC is 11:00 here.
        assert_eq!(run("what time is it in tokyo?", january()).unwrap(), "11:00 AM here = 6:00 PM Tokyo (JST)");
        assert_eq!(run("tokyo", january()).unwrap(), "11:00 AM here = 6:00 PM Tokyo (JST)");
        // A time and a place: that time there, in your time.
        assert_eq!(run("9am london", january()).unwrap(), "9:00 AM London (GMT) = 11:00 AM here");
        assert_eq!(run("8pm to sydney", january()).unwrap(), "8:00 PM here = 5:00 AM Sydney (AEDT) (next day)");
    }

    #[test]
    fn offsets_and_zone_names() {
        assert_eq!(run("12:00 utc to utc+5:30", january()).unwrap(), "12:00 PM UTC = 5:30 PM UTC+5:30");
        assert_eq!(run("1am gmt-3 to utc", january()).unwrap(), "1:00 AM GMT-3 = 4:00 AM UTC");
        assert_eq!(run("6pm Europe/Tallinn to Asia/Kathmandu", january()).unwrap(), "6:00 PM Tallinn (EET) = 9:45 PM Kathmandu");
    }

    #[test]
    fn clock_formats() {
        let at = |h, m| NaiveTime::from_hms_opt(h, m, 0);
        assert_eq!(parse_clock("3pm"), at(15, 0));
        assert_eq!(parse_clock("3 pm"), at(15, 0));
        assert_eq!(parse_clock("12am"), at(0, 0));
        assert_eq!(parse_clock("12pm"), at(12, 0));
        assert_eq!(parse_clock("9.30am"), at(9, 30));
        assert_eq!(parse_clock("21:05"), at(21, 5));
        assert_eq!(parse_clock("noon"), at(12, 0));
        assert_eq!(parse_clock("midnight"), at(0, 0));
        // Not times.
        assert_eq!(parse_clock("3"), None);
        assert_eq!(parse_clock("13pm"), None);
        assert_eq!(parse_clock("25:00"), None);
        assert_eq!(parse_clock("9:75"), None);
        assert_eq!(parse_clock("tokyo"), None);
    }

    #[test]
    fn not_understood() {
        assert_eq!(run("", january()), None);
        assert_eq!(run("3pm", january()), None);
        assert_eq!(run("3pm IST to", january()), None);
        assert_eq!(run("3pm narnia to ist", january()), None);
        assert_eq!(run("utc+15 to ist", january()), None);
    }

    #[test]
    fn crossing_midnight_and_summer_time_gaps() {
        assert_eq!(run("11pm london to tokyo", january()).unwrap(), "11:00 PM London (GMT) = 8:00 AM Tokyo (JST) (next day)");
        // 2:30am on the night the US clocks go forward doesn't exist: it counts as 3:30.
        let spring = Utc.with_ymd_and_hms(2026, 3, 8, 12, 0, 0).unwrap();
        assert_eq!(run("2:30am new york to utc", spring).unwrap(), "3:30 AM New York (EDT) = 7:30 AM UTC");
    }

    #[test]
    fn every_listed_zone_exists() {
        for (short, zone) in SHORT_NAMES {
            assert!(zone.parse::<Tz>().is_ok(), "{short}: {zone}");
        }
        for (names, _, zone) in PLACES {
            assert!(zone.parse::<Tz>().is_ok(), "{names:?}: {zone}");
        }
    }
}
