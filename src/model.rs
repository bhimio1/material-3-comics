//! What sources hand back: series, chapters and pages, independent of
//! where they come from. The library (`db.rs`) stores these with its own ids.

use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    #[default]
    Unknown,
    Ongoing,
    Completed,
    Hiatus,
    Cancelled,
    Licensed,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Status::Unknown => "Unknown",
            Status::Ongoing => "Ongoing",
            Status::Completed => "Completed",
            Status::Hiatus => "On hiatus",
            Status::Cancelled => "Cancelled",
            Status::Licensed => "Licensed",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Status::Unknown => "unknown",
            Status::Ongoing => "ongoing",
            Status::Completed => "completed",
            Status::Hiatus => "hiatus",
            Status::Cancelled => "cancelled",
            Status::Licensed => "licensed",
        }
    }

    /// Lenient: MangaDex's, Suwayomi's and Mihon's spellings all map here.
    pub fn parse(s: &str) -> Status {
        match s.trim().to_ascii_lowercase().replace([' ', '-'], "_").as_str() {
            "ongoing" | "publishing" | "1" => Status::Ongoing,
            "completed" | "complete" | "finished" | "publishing_finished" | "2" | "4" => Status::Completed,
            "hiatus" | "on_hiatus" | "6" => Status::Hiatus,
            "cancelled" | "canceled" | "5" => Status::Cancelled,
            "licensed" | "3" => Status::Licensed,
            _ => Status::Unknown,
        }
    }
}

/// A series as a source describes it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MangaInfo {
    /// The source's own id for it (a UUID, a server id, a folder path).
    pub key: String,
    pub title: String,
    /// An `https://` URL, a `file://` path, or an `archive://` page (see
    /// `archive::cover_ref`).
    pub cover: Option<String>,
    pub author: String,
    pub artist: String,
    pub description: String,
    pub genres: Vec<String>,
    pub status: Status,
    /// The series' page on the web, for "Open in browser".
    pub web_url: Option<String>,
}

/// A chapter as a source describes it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ChapterInfo {
    pub key: String,
    pub name: String,
    /// The chapter number, or a negative value when it has none.
    pub number: f32,
    pub scanlator: String,
    /// Unix seconds; 0 when unknown.
    pub uploaded: i64,
    pub web_url: Option<String>,
}

/// One page of a chapter: where its image comes from.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum PageRef {
    Url { url: String, headers: Vec<(String, String)> },
    /// An entry inside a CBZ/CBR/CB7/… file.
    Archive { path: PathBuf, entry: String },
    File(PathBuf),
}

/// One page of browse or search results.
#[derive(Clone, Debug, Default)]
pub struct MangaList {
    pub items: Vec<MangaInfo>,
    pub has_next: bool,
}

/// "Chapter 12.5", "Vol. 2 Chapter 10: Title", or the given name.
pub fn chapter_title(volume: Option<&str>, number: Option<&str>, title: Option<&str>) -> String {
    let mut s = String::new();
    if let Some(v) = volume.filter(|v| !v.is_empty()) {
        s.push_str(&format!("Vol. {v} "));
    }
    match number.filter(|n| !n.is_empty()) {
        Some(n) => s.push_str(&format!("Chapter {n}")),
        None if s.is_empty() => s.push_str("Oneshot"),
        None => {}
    }
    if let Some(t) = title.map(str::trim).filter(|t| !t.is_empty()) {
        s = format!("{}: {t}", s.trim_end());
    }
    s.trim().to_string()
}

/// A chapter number from a file or folder name: "Ch. 12.5", "c012",
/// "Chapter 3 - Title", "Vol.1 Ch.4". Falls back to the last number.
pub fn number_from_name(name: &str) -> f32 {
    let lower = name.to_lowercase();
    let stem = lower
        .rsplit_once('.')
        .filter(|(_, ext)| ext.len() <= 4 && ext.chars().all(|c| c.is_ascii_alphanumeric()) && ext.chars().any(|c| c.is_ascii_alphabetic()))
        .map(|(s, _)| s)
        .unwrap_or(&lower);
    let numbers = numbers_in(stem);
    for marker in ["chapter", "chap", "ch.", "ch", "c", "#"] {
        if let Some(i) = stem.find(marker) {
            let after = i + marker.len();
            if let Some((_, n)) = numbers.iter().find(|(pos, _)| *pos >= after && stem[after..*pos].trim_matches(|c: char| c == ' ' || c == '.' || c == '_' || c == '-').is_empty()) {
                return *n;
            }
        }
    }
    // Skip volume numbers when there are other numbers.
    let vol = ["vol.", "vol", "v"].iter().find_map(|m| {
        let i = stem.find(m)?;
        let after = i + m.len();
        numbers.iter().find(|(pos, _)| *pos >= after && stem[after..*pos].trim().is_empty()).map(|(pos, _)| *pos)
    });
    numbers.iter().rev().find(|(pos, _)| Some(*pos) != vol).or(numbers.last()).map(|(_, n)| *n).unwrap_or(-1.0)
}

/// (byte offset, value) of every decimal number in `s`.
fn numbers_in(s: &str) -> Vec<(usize, f32)> {
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            if i + 1 < bytes.len() && bytes[i] == b'.' && bytes[i + 1].is_ascii_digit() {
                i += 1;
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
            }
            if let Ok(v) = s[start..i].parse::<f32>() {
                out.push((start, v));
            }
        } else {
            i += 1;
        }
    }
    out
}

/// Natural order: "page2" before "page10".
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut ai, mut bi) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (ai.peek().copied(), bi.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(ca), Some(cb)) if ca.is_ascii_digit() && cb.is_ascii_digit() => {
                let mut na = String::new();
                while let Some(c) = ai.peek().copied().filter(char::is_ascii_digit) {
                    na.push(c);
                    ai.next();
                }
                let mut nb = String::new();
                while let Some(c) = bi.peek().copied().filter(char::is_ascii_digit) {
                    nb.push(c);
                    bi.next();
                }
                let ta = na.trim_start_matches('0');
                let tb = nb.trim_start_matches('0');
                let ord = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(ca), Some(cb)) => {
                let (la, lb) = (ca.to_lowercase().next().unwrap_or(ca), cb.to_lowercase().next().unwrap_or(cb));
                if la != lb {
                    return la.cmp(&lb);
                }
                ai.next();
                bi.next();
            }
        }
    }
}

/// A name that is safe as a file or folder name.
pub fn safe_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control() { '_' } else { c })
        .collect();
    let trimmed = cleaned.trim().trim_matches('.').trim();
    let mut out: String = trimmed.chars().take(120).collect();
    if out.is_empty() {
        out = "_".into();
    }
    out
}

/// "2 days ago", "Yesterday", "3 Mar 2025".
pub fn relative_date(unix: i64, now: i64) -> String {
    if unix <= 0 {
        return String::new();
    }
    let days = (now - unix).div_euclid(86_400);
    match days {
        i64::MIN..=0 => "Today".into(),
        1 => "Yesterday".into(),
        2..=6 => format!("{days} days ago"),
        _ => chrono::DateTime::from_timestamp(unix, 0)
            .map(|d| d.with_timezone(&chrono::Local).format("%-d %b %Y").to_string())
            .unwrap_or_default(),
    }
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// "12" or "12.5"; empty for unknown numbers.
pub fn format_number(n: f32) -> String {
    if n < 0.0 {
        String::new()
    } else if n.fract() == 0.0 {
        format!("{}", n as i64)
    } else {
        let s = format!("{n:.2}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses() {
        assert_eq!(Status::parse("PUBLISHING_FINISHED"), Status::Completed);
        assert_eq!(Status::parse("ON_HIATUS"), Status::Hiatus);
        assert_eq!(Status::parse("ongoing"), Status::Ongoing);
        assert_eq!(Status::parse("2"), Status::Completed);
        assert_eq!(Status::parse("whatever"), Status::Unknown);
    }

    #[test]
    fn chapter_titles() {
        assert_eq!(chapter_title(Some("2"), Some("10"), Some("The End")), "Vol. 2 Chapter 10: The End");
        assert_eq!(chapter_title(None, Some("12.5"), None), "Chapter 12.5");
        assert_eq!(chapter_title(None, None, Some("Special")), "Oneshot: Special");
        assert_eq!(chapter_title(None, None, None), "Oneshot");
    }

    #[test]
    fn numbers_from_names() {
        assert_eq!(number_from_name("Ch. 12.5.cbz"), 12.5);
        assert_eq!(number_from_name("c012"), 12.0);
        assert_eq!(number_from_name("Chapter 3 - The 100 Swords"), 3.0);
        assert_eq!(number_from_name("Vol.1 Ch.4.cbz"), 4.0);
        assert_eq!(number_from_name("Vol 2 - 15"), 15.0);
        assert_eq!(number_from_name("Series 007.cbr"), 7.0);
        assert_eq!(number_from_name("Prologue"), -1.0);
    }

    #[test]
    fn natural_order() {
        let mut v = vec!["p10.jpg", "p2.jpg", "P1.jpg", "p02b.jpg"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, vec!["P1.jpg", "p2.jpg", "p02b.jpg", "p10.jpg"]);
    }

    #[test]
    fn file_names() {
        assert_eq!(safe_file_name("What/Is: This?"), "What_Is_ This_");
        assert_eq!(safe_file_name(" .. "), "_");
        assert_eq!(format_number(12.0), "12");
        assert_eq!(format_number(12.5), "12.5");
        assert_eq!(format_number(-1.0), "");
    }

    #[test]
    fn dates() {
        let now = 1_700_000_000;
        assert_eq!(relative_date(now - 100, now), "Today");
        assert_eq!(relative_date(now - 86_400 - 5, now), "Yesterday");
        assert_eq!(relative_date(now - 3 * 86_400, now), "3 days ago");
        assert_eq!(relative_date(0, now), "");
    }
}
