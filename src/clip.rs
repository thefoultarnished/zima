//! Web clipper receiver: a tiny HTTP server on 127.0.0.1 that accepts clips from the browser extension
//! (clipper/). Only local connections, only with the secret token shown in Settings.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{Receiver, Sender, channel};

pub const PORT: u16 = 47621;
const MAX_BODY: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub struct Clip {
    pub title: String,
    pub url: String,
    pub text: String,
}

/// Start listening on a background thread; clips arrive on the returned channel.
pub fn start(token: String) -> std::io::Result<Receiver<Clip>> {
    let listener = TcpListener::bind(("127.0.0.1", PORT))?;
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let (tx, token) = (tx.clone(), token.clone());
            std::thread::spawn(move || handle(stream, &token, &tx));
        }
    });
    Ok(rx)
}

fn handle(mut stream: TcpStream, token: &str, tx: &Sender<Clip>) {
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(5)));
    let status = match read_request(&stream) {
        Ok((method, path, headers, body)) => respond_to(&method, &path, &headers, &body, token, tx),
        Err(_) => "400 Bad Request",
    };
    let body = if status.starts_with('2') { "{\"ok\":true}" } else { "{\"ok\":false}" };
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Headers: Content-Type, X-Zima-Token\r\nAccess-Control-Allow-Methods: POST, OPTIONS\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
}

type Request = (String, String, Vec<(String, String)>, Vec<u8>);

fn read_request(stream: &TcpStream) -> std::io::Result<Request> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let (method, path) = (parts.next().unwrap_or("").to_string(), parts.next().unwrap_or("").to_string());
    let mut headers = Vec::new();
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 || header.trim().is_empty() {
            break;
        }
        if let Some((k, v)) = header.split_once(':') {
            headers.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
        }
    }
    let length = headers.iter().find(|(k, _)| k == "content-length").and_then(|(_, v)| v.parse::<usize>().ok()).unwrap_or(0);
    if length > MAX_BODY {
        return Err(std::io::Error::other("too large"));
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    Ok((method, path, headers, body))
}

/// The HTTP status for a request (and send the clip on if it's valid).
fn respond_to(method: &str, path: &str, headers: &[(String, String)], body: &[u8], token: &str, tx: &Sender<Clip>) -> &'static str {
    if method == "OPTIONS" {
        return "204 No Content";
    }
    if method != "POST" || path != "/clip" {
        return "404 Not Found";
    }
    let given = headers.iter().find(|(k, _)| k == "x-zima-token").map(|(_, v)| v.as_str());
    if given != Some(token) || token.is_empty() {
        return "401 Unauthorized";
    }
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(body) else { return "400 Bad Request" };
    let field = |k: &str| value[k].as_str().unwrap_or("").trim().to_string();
    let clip = Clip { title: field("title"), url: field("url"), text: field("text") };
    if clip.title.is_empty() && clip.url.is_empty() && clip.text.is_empty() {
        return "400 Bad Request";
    }
    match tx.send(clip) {
        Ok(()) => "200 OK",
        Err(_) => "503 Service Unavailable",
    }
}

/// A clip as note text: the excerpt as a quote, then the source link.
pub fn to_markdown(clip: &Clip) -> String {
    let mut body = String::new();
    if !clip.text.is_empty() {
        for line in clip.text.lines() {
            body.push_str(&format!("> {line}\n"));
        }
        body.push('\n');
    }
    if !clip.url.is_empty() {
        let label = if clip.title.is_empty() { clip.url.as_str() } else { clip.title.as_str() };
        body.push_str(&format!("Source: [{}]({})\n", label.replace(['[', ']'], ""), clip.url));
    }
    body
}

/// A random token for pairing the extension (OS-seeded randomness from the standard library).
pub fn new_token() -> String {
    use std::hash::{BuildHasher, Hasher};
    const ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";
    (0..24)
        .map(|i| {
            let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
            hasher.write_usize(i);
            ALPHABET[(hasher.finish() % ALPHABET.len() as u64) as usize] as char
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_valid_clips() {
        let (tx, rx) = channel();
        let ok_headers = vec![("x-zima-token".to_string(), "secret".to_string())];
        let body = br#"{"title":"Page","url":"https://a.b","text":"quote"}"#;
        assert_eq!(respond_to("POST", "/clip", &ok_headers, body, "secret", &tx), "200 OK");
        assert_eq!(rx.try_recv().unwrap().title, "Page");
        assert_eq!(respond_to("POST", "/clip", &[], body, "secret", &tx), "401 Unauthorized");
        let wrong = vec![("x-zima-token".to_string(), "nope".to_string())];
        assert_eq!(respond_to("POST", "/clip", &wrong, body, "secret", &tx), "401 Unauthorized");
        assert_eq!(respond_to("GET", "/clip", &ok_headers, body, "secret", &tx), "404 Not Found");
        assert_eq!(respond_to("POST", "/clip", &ok_headers, b"{}", "secret", &tx), "400 Bad Request");
    }

    #[test]
    fn markdown_and_tokens() {
        let md = to_markdown(&Clip { title: "A [page]".into(), url: "https://a.b".into(), text: "one\ntwo".into() });
        assert_eq!(md, "> one\n> two\n\nSource: [A page](https://a.b)\n");
        let (a, b) = (new_token(), new_token());
        assert_eq!(a.len(), 24);
        assert!(a.chars().all(|c| c.is_ascii_alphanumeric()));
        let _ = b;
    }
}
