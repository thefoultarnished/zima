//! Spell checking with Windows' built-in spell checker (ISpellChecker), and word lookup.

/// A misspelled word in the note (byte range).
#[derive(Debug, Clone, PartialEq)]
pub struct Misspelling {
    pub start: usize,
    pub end: usize,
    pub word: String,
}

/// Blank out things that shouldn't be spell-checked (code, URLs, link targets, tags, [[links]]),
/// keeping every character's position so results map straight back onto the note.
pub fn mask(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut keep = vec![true; chars.len()];
    let mut i = 0;
    let mut in_fence = false;
    let mut line_start = true;
    while i < chars.len() {
        // Fenced code blocks.
        if line_start && chars[i..].starts_with(&['`', '`', '`']) {
            in_fence = !in_fence;
        }
        if in_fence {
            if chars[i] != '\n' {
                keep[i] = false;
            }
            line_start = chars[i] == '\n';
            i += 1;
            continue;
        }
        let rest: String = chars[i..chars.len().min(i + 8)].iter().collect();
        let blank_until = |keep: &mut Vec<bool>, from: usize, stop: &dyn Fn(char) -> bool| {
            let mut j = from;
            while j < chars.len() && !stop(chars[j]) {
                keep[j] = false;
                j += 1;
            }
            j
        };
        if chars[i] == '`' {
            // Inline code: up to the next backtick on the line.
            let end = chars[i + 1..].iter().position(|&c| c == '`' || c == '\n').map(|p| i + 1 + p);
            match end {
                Some(e) if chars[e] == '`' => {
                    for k in &mut keep[i..=e] {
                        *k = false;
                    }
                    i = e + 1;
                }
                _ => i += 1,
            }
        } else if rest.starts_with("http://") || rest.starts_with("https://") || rest.starts_with("www.") {
            i = blank_until(&mut keep, i, &|c: char| c.is_whitespace() || c == ')');
        } else if chars[i] == ']' && chars.get(i + 1) == Some(&'(') {
            // Link target: ](...)
            i = blank_until(&mut keep, i + 1, &|c: char| c == ')' || c == '\n');
        } else if chars[i] == '#' && (i == 0 || chars[i - 1].is_whitespace()) && chars.get(i + 1).is_some_and(|c| c.is_alphabetic()) {
            i = blank_until(&mut keep, i, &|c: char| c.is_whitespace());
        } else if chars[i..].starts_with(&['[', '[']) {
            i = blank_until(&mut keep, i, &|c: char| c == '\n' || c == ']');
        } else {
            line_start = chars[i] == '\n';
            i += 1;
            continue;
        }
        line_start = false;
    }
    chars.iter().zip(keep).map(|(&c, k)| if k || c == '\n' { c } else { ' ' }).collect()
}

/// Map UTF-16 offsets in `masked` (same characters as `original`) to byte offsets in `original`.
fn utf16_to_byte(original: &str, utf16: usize) -> usize {
    let mut units = 0;
    for (byte, c) in original.char_indices() {
        if units >= utf16 {
            return byte;
        }
        units += c.len_utf16();
    }
    original.len()
}

#[cfg(windows)]
pub use imp::Checker;

#[cfg(windows)]
mod imp {
    use windows::Win32::Globalization::{
        CORRECTIVE_ACTION_DELETE, CORRECTIVE_ACTION_GET_SUGGESTIONS, CORRECTIVE_ACTION_REPLACE, ISpellChecker,
        ISpellCheckerFactory, SpellCheckerFactory,
    };
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree};
    use windows::core::{HSTRING, PWSTR};

    use super::{Misspelling, mask, utf16_to_byte};

    pub struct Checker {
        inner: ISpellChecker,
    }

    impl Checker {
        /// The checker for the user's language (falling back to US English).
        pub fn new() -> Option<Self> {
            unsafe {
                // Usually already initialised by the windowing code; harmless if so.
                let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
                let factory: ISpellCheckerFactory = CoCreateInstance(&SpellCheckerFactory, None, CLSCTX_INPROC_SERVER).ok()?;
                let tag = HSTRING::from(user_language().unwrap_or_else(|| "en-US".into()));
                let inner = factory.CreateSpellChecker(&tag).or_else(|_| factory.CreateSpellChecker(&HSTRING::from("en-US"))).ok()?;
                Some(Self { inner })
            }
        }

        pub fn check(&self, text: &str) -> Vec<Misspelling> {
            let masked = mask(text);
            let mut out = Vec::new();
            unsafe {
                let Ok(errors) = self.inner.Check(&HSTRING::from(masked.as_str())) else { return out };
                loop {
                    let mut error = None;
                    if errors.Next(&mut error).is_err() {
                        break;
                    }
                    let Some(error) = error else { break };
                    let (Ok(start), Ok(len), Ok(action)) = (error.StartIndex(), error.Length(), error.CorrectiveAction()) else { continue };
                    if action != CORRECTIVE_ACTION_GET_SUGGESTIONS && action != CORRECTIVE_ACTION_REPLACE && action != CORRECTIVE_ACTION_DELETE {
                        continue;
                    }
                    let (start, end) = (utf16_to_byte(text, start as usize), utf16_to_byte(text, (start + len) as usize));
                    out.push(Misspelling { start, end, word: text[start..end].to_string() });
                }
            }
            out
        }

        pub fn suggest(&self, word: &str, max: usize) -> Vec<String> {
            let mut out = Vec::new();
            unsafe {
                let Ok(list) = self.inner.Suggest(&HSTRING::from(word)) else { return out };
                while out.len() < max {
                    let mut item = [PWSTR::null()];
                    let mut fetched = 0u32;
                    if list.Next(&mut item, Some(&mut fetched)).is_err() || fetched == 0 {
                        break;
                    }
                    if let Ok(s) = item[0].to_string() {
                        out.push(s);
                    }
                    CoTaskMemFree(Some(item[0].0 as *const _));
                }
            }
            out
        }

        /// Add to the user's dictionary (shared with other Windows apps).
        pub fn add(&self, word: &str) {
            unsafe {
                let _ = self.inner.Add(&HSTRING::from(word));
            }
        }

        /// Ignore for this session.
        pub fn ignore(&self, word: &str) {
            unsafe {
                let _ = self.inner.Ignore(&HSTRING::from(word));
            }
        }
    }

    /// The Windows display language, e.g. "en-GB".
    fn user_language() -> Option<String> {
        use windows::Win32::Globalization::GetUserDefaultLocaleName;
        let mut buffer = [0u16; 85];
        let len = unsafe { GetUserDefaultLocaleName(&mut buffer) };
        (len > 1).then(|| String::from_utf16_lossy(&buffer[..len as usize - 1]))
    }
}

/// A word's definitions from the free Dictionary API (dictionaryapi.dev). Runs a network request:
/// call from a background thread.
pub fn look_up(word: &str) -> Result<Vec<(String, Vec<String>, Vec<String>)>, String> {
    let word: String = word.trim().chars().filter(|c| c.is_alphabetic() || *c == '-' || *c == '\'').collect();
    if word.is_empty() {
        return Err("Select a word to look up".into());
    }
    let url = format!("https://api.dictionaryapi.dev/api/v2/entries/en/{word}");
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(25)))
        // Windows' own secure connection code and trusted certificates (a smaller exe than bundling them).
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .provider(ureq::tls::TlsProvider::NativeTls)
                .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                .build(),
        )
        .build()
        .into();
    // The free service is sometimes slow; try twice before giving up.
    let mut attempt = 0;
    let mut response = loop {
        attempt += 1;
        match agent.get(&url).call() {
            Ok(response) => break response,
            Err(ureq::Error::StatusCode(404)) => return Err(format!("No definition found for \u{201c}{word}\u{201d}")),
            Err(_) if attempt < 2 => continue,
            Err(ureq::Error::StatusCode(code)) if code >= 500 => {
                return Err("The dictionary service is busy right now. Try again in a moment.".into());
            }
            Err(e) => return Err(format!("Couldn't reach the dictionary: {e}")),
        }
    };
    let body = response.body_mut().read_to_string().map_err(|e| e.to_string())?;
    let value: serde_json::Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    // [(part of speech, definitions, synonyms)]
    let mut meanings = Vec::new();
    for entry in value.as_array().into_iter().flatten() {
        for meaning in entry["meanings"].as_array().into_iter().flatten() {
            let part = meaning["partOfSpeech"].as_str().unwrap_or("").to_string();
            let definitions: Vec<String> = meaning["definitions"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|d| d["definition"].as_str().map(str::to_string))
                .take(3)
                .collect();
            let synonyms: Vec<String> = meaning["synonyms"].as_array().into_iter().flatten().filter_map(|s| s.as_str().map(str::to_string)).take(6).collect();
            if !definitions.is_empty() {
                meanings.push((part, definitions, synonyms));
            }
        }
    }
    if meanings.is_empty() { Err(format!("No definition found for \u{201c}{word}\u{201d}")) } else { Ok(meanings) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_code_links_and_tags() {
        let text = "Teh `codd` see https://exmple.com [link](http://x.y) #tagg [[Notte]]\n```\nfnn x\n```\nend";
        let masked = mask(text);
        assert_eq!(masked.chars().count(), text.chars().count());
        assert!(masked.starts_with("Teh "));
        for hidden in ["codd", "exmple", "http://x.y", "tagg", "Notte", "fnn"] {
            assert!(!masked.contains(hidden), "{hidden} should be masked: {masked}");
        }
        assert!(masked.contains("[link"));
        assert!(masked.ends_with("end"));
    }

    #[test]
    fn utf16_offsets() {
        assert_eq!(utf16_to_byte("héllo wörld", 6), 7);
        assert_eq!(utf16_to_byte("a😀b", 3), 5);
    }

    #[cfg(windows)]
    #[test]
    fn windows_checker_finds_typos() {
        let Some(checker) = Checker::new() else { return }; // no spell checker on this machine
        let found = checker.check("This sentance has a typo.");
        assert!(found.iter().any(|m| m.word == "sentance"), "{found:?}");
        assert!(checker.suggest("sentance", 3).iter().any(|s| s == "sentence"));
    }
}
