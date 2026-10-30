//! File system operations: path policy (markdown-only, size guard) and the
//! authorization set that mirrors `electron/security.cjs`.
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::config::{MARKDOWN_EXTENSIONS, MAX_MARKDOWN_BYTES};
use crate::error::MarkflyError;

/// True when the path looks like a markdown file (extension check, case-insensitive).
///
/// Covers both `/` and `\` separators, so it works with Windows paths and with
/// command-line arguments such as `C:\Users\me\notes\todo.md`.
pub fn is_markdown_path(path: &str) -> bool {
    if path.is_empty() {
        return false;
    }
    let file_name = path
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default();
    match file_name.rsplit_once('.') {
        Some((_, ext)) => {
            let ext = ext.to_ascii_lowercase();
            MARKDOWN_EXTENSIONS.contains(&ext.as_str())
        }
        None => false,
    }
}

/// True when the string is an http(s) URL (used to keep remote tabs in prefs).
pub fn is_remote_url(value: &str) -> bool {
    let lower = value.trim().to_ascii_lowercase();
    lower.starts_with("https://") || lower.starts_with("http://")
}

/// Rejects sizes above the markdown limit.
pub fn assert_markdown_size(size: u64) -> Result<(), MarkflyError> {
    if size > MAX_MARKDOWN_BYTES {
        return Err(MarkflyError::TooLarge {
            size,
            limit: MAX_MARKDOWN_BYTES,
        });
    }
    Ok(())
}

/// Tracks which files the renderer is allowed to read.
///
/// Port of `AuthorizedFiles` in the original app: paths are normalized to an
/// absolute path (lexically, without touching the filesystem) and stored in a
/// set. `std::path::absolute` handles both `/` and `\` separators on Windows.
#[derive(Debug, Default)]
pub struct AuthorizedFiles {
    paths: HashSet<PathBuf>,
}

impl AuthorizedFiles {
    pub fn new() -> Self {
        Self::default()
    }

    /// Validates the extension, normalizes and registers the path.
    pub fn authorize(&mut self, file_path: &str) -> Result<PathBuf, MarkflyError> {
        if !is_markdown_path(file_path) {
            return Err(MarkflyError::NotMarkdown(file_path.to_string()));
        }
        let normalized = normalize_path(file_path)?;
        self.paths.insert(normalized.clone());
        Ok(normalized)
    }

    pub fn has(&self, file_path: &str) -> bool {
        match normalize_path(file_path) {
            Ok(normalized) => self.paths.contains(&normalized),
            Err(_) => false,
        }
    }

    pub fn len(&self) -> usize {
        self.paths.len()
    }

    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }
}

/// Lexically normalizes a path to an absolute form without filesystem access.
pub fn normalize_path(path: &str) -> Result<PathBuf, MarkflyError> {
    let p = Path::new(path);
    let absolute = std::path::absolute(p).map_err(MarkflyError::Io)?;
    Ok(absolute)
}

/// Reads a markdown file if (and only if) it was authorized first.
pub fn read_authorized_markdown(
    authorized: &AuthorizedFiles,
    file_path: &str,
) -> Result<String, MarkflyError> {
    if !authorized.has(file_path) {
        return Err(MarkflyError::Unauthorized);
    }
    read_markdown_file(file_path)
}

/// Appends a `.md` extension when the chosen save path has none.
///
/// A different, non-markdown extension is rejected so the write path keeps the
/// same markdown-only policy as reads. `C:\notes\journal` becomes
/// `C:\notes\journal.md`; `C:\notes\journal.txt` is refused.
pub fn ensure_markdown_extension(file_path: &str) -> Result<String, MarkflyError> {
    if is_markdown_path(file_path) {
        return Ok(file_path.to_string());
    }
    let file_name = file_path.rsplit(['/', '\\']).next().unwrap_or_default();
    if file_name.is_empty() || file_name.contains('.') {
        return Err(MarkflyError::NotMarkdown(file_path.to_string()));
    }
    Ok(format!("{file_path}.md"))
}

/// Creates an empty markdown file if it does not exist yet.
///
/// Never truncates an existing file: picking a name that already exists simply
/// opens that file, so no content can be lost by choosing "New".
pub fn create_markdown_file(file_path: &str) -> Result<(), MarkflyError> {
    if !is_markdown_path(file_path) {
        return Err(MarkflyError::NotMarkdown(file_path.to_string()));
    }
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(file_path)
    {
        Ok(_) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(err) => Err(MarkflyError::Io(err)),
    }
}

/// Writes markdown content, enforcing the extension and size policies.
pub fn write_markdown_file(file_path: &str, content: &str) -> Result<(), MarkflyError> {
    if !is_markdown_path(file_path) {
        return Err(MarkflyError::NotMarkdown(file_path.to_string()));
    }
    assert_markdown_size(content.len() as u64)?;
    std::fs::write(file_path, content.as_bytes())?;
    Ok(())
}

/// Writes a markdown file if (and only if) the path was authorized first.
pub fn write_authorized_markdown(
    authorized: &AuthorizedFiles,
    file_path: &str,
    content: &str,
) -> Result<(), MarkflyError> {
    if !authorized.has(file_path) {
        return Err(MarkflyError::Unauthorized);
    }
    write_markdown_file(file_path, content)
}

/// Reads and size-checks a markdown file. Invalid UTF-8 is replaced the same
/// way `Buffer.toString('utf-8')` does in the original implementation.
pub fn read_markdown_file(file_path: &str) -> Result<String, MarkflyError> {
    let path = Path::new(file_path);
    let metadata = std::fs::metadata(path)?;
    if !metadata.is_file() {
        return Err(MarkflyError::Io(std::io::Error::other(format!(
            "Selected path is not a file: {file_path}"
        ))));
    }
    assert_markdown_size(metadata.len())?;
    let bytes = std::fs::read(path)?;
    assert_markdown_size(bytes.len() as u64)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}
