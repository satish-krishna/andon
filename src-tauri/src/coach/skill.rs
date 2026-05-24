//! Prompt-signature normaliser + discovery pass + examples reader.

use blake3::Hasher;
use once_cell::sync::Lazy;
use regex::Regex;

// Static 32-byte key — built into the binary, stable across runs but
// not portable across installs.
const NORM_KEY: &[u8; 32] = b"andon-coach-skill-finder-key-v1!";

static PATH_RE: Lazy<Regex> = Lazy::new(|| Regex::new(
    r"(?:@[\w./\\-]+|(?:^|\s)(?:/|[A-Za-z]:\\)[\w./\\-]+)"
).unwrap());
static UUID_RE: Lazy<Regex> = Lazy::new(|| Regex::new(
    r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}"
).unwrap());
static SHA_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\b[0-9a-fA-F]{7,40}\b").unwrap());
static NUM_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\d{4,}").unwrap());
static CODE_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?s)```.*?```").unwrap());
static WS_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s+").unwrap());

fn normalise(text: &str) -> String {
    let s = text.to_lowercase();
    let s = CODE_RE.replace_all(&s, "<code>");
    let s = PATH_RE.replace_all(&s, "<path>");
    let s = UUID_RE.replace_all(&s, "<id>");
    let s = SHA_RE.replace_all(&s, "<id>");
    let s = NUM_RE.replace_all(&s, "<num>");
    let s = WS_RE.replace_all(s.trim(), " ").into_owned();
    s.chars().take(1024).collect()
}

pub fn norm_hash(text: &str) -> String {
    let n = normalise(text);
    let mut h = Hasher::new_keyed(NORM_KEY);
    h.update(n.as_bytes());
    h.finalize().to_hex().to_string()
}
