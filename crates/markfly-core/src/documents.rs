//! Document model helpers: heading outline extraction and display naming.
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use serde::{Deserialize, Serialize};

/// A heading in the document outline. `id` matches the id emitted by the
/// renderer (`markfly-heading-{index}`) so the outline panel can jump to it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Heading {
    pub id: String,
    pub level: u8,
    pub text: String,
}

/// Extracts the heading outline of a markdown document.
pub fn extract_outline(markdown: &str) -> Vec<Heading> {
    let mut headings = Vec::new();
    let mut in_heading: Option<(u8, String)> = None;

    for event in parser(markdown) {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                in_heading = Some((heading_level(level), String::new()));
            }
            Event::End(TagEnd::Heading(_)) => {
                if let Some((level, text)) = in_heading.take() {
                    let text = text.trim().to_string();
                    headings.push(Heading {
                        id: format!("markfly-heading-{}", headings.len()),
                        level,
                        text,
                    });
                }
            }
            Event::Text(text) | Event::Code(text) => {
                if let Some((_, buffer)) = in_heading.as_mut() {
                    buffer.push_str(&text);
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if let Some((_, buffer)) = in_heading.as_mut() {
                    buffer.push(' ');
                }
            }
            _ => {}
        }
    }

    headings
}

/// Basename of a local path or the file name of an http(s) URL.
/// Handles both `/` and `\` separators (Windows).
pub fn document_name(path: &str) -> String {
    if is_remote(path) {
        if let Ok(url) = url::Url::parse(path) {
            let filename = url
                .path()
                .rsplit('/')
                .next()
                .filter(|part| !part.is_empty());
            if let Some(filename) = filename {
                return percent_decode(filename);
            }
            if let Some(host) = url.host_str() {
                return host.to_string();
            }
        }
        return path.to_string();
    }
    path.rsplit(['/', '\\'])
        .next()
        .filter(|part| !part.is_empty())
        .map(|part| part.to_string())
        .unwrap_or_else(|| "Unknown".to_string())
}

/// `~`-shortened path for the sidebar, mirroring the original UI behavior.
pub fn display_path(path: &str) -> String {
    if is_remote(path) {
        return path.to_string();
    }
    // macOS: /Users/<name>/... -> ~/...
    let macos = path.strip_prefix("/Users/").and_then(|rest| {
        let (_, after) = rest.split_once('/')?;
        Some(format!("~/{after}"))
    });
    if let Some(shortened) = macos {
        return shortened;
    }
    // Windows: C:\Users\<name>\... -> ~\...
    if let Some(rest) = strip_windows_home_prefix(path) {
        return rest;
    }
    path.to_string()
}

fn strip_windows_home_prefix(path: &str) -> Option<String> {
    let bytes = path.as_bytes();
    // Drive letter, e.g. "C:"
    if bytes.len() < 2 || !bytes[0].is_ascii_alphabetic() || bytes[1] != b':' {
        return None;
    }
    let mut idx = 2;
    if bytes.get(idx) == Some(&b'\\') || bytes.get(idx) == Some(&b'/') {
        idx += 1;
    } else {
        return None;
    }
    let rest = &path[idx..];
    let separator = if path.contains('\\') { '\\' } else { '/' };
    let mut parts = rest.split(['\\', '/']);
    let users = parts.next()?;
    if !users.eq_ignore_ascii_case("users") {
        return None;
    }
    let name = parts.next()?;
    if name.is_empty() {
        return None;
    }
    let tail = rest[users.len() + 1 + name.len()..]
        .trim_start_matches(['\\', '/'])
        .to_string();
    if tail.is_empty() {
        return Some("~".to_string());
    }
    Some(format!("~{separator}{tail}"))
}

fn is_remote(path: &str) -> bool {
    path.starts_with("https://") || path.starts_with("http://")
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(hi), Some(lo)) = (hi, lo) {
                out.push((hi * 16 + lo) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn heading_level(level: pulldown_cmark::HeadingLevel) -> u8 {
    use pulldown_cmark::HeadingLevel;
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn parser(markdown: &str) -> Parser<'_> {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_MATH;
    Parser::new_ext(markdown, options)
}
