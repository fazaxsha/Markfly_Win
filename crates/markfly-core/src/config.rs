//! Constants mirroring the security policy of the original Electron app
//! (`electron/security.cjs` and `src/markdown-security.ts`).

/// Maximum size of a markdown document (local file or downloaded), 5 MiB.
pub const MAX_MARKDOWN_BYTES: u64 = 5 * 1024 * 1024;

/// Accepted markdown extensions (lowercase, without dot).
pub const MARKDOWN_EXTENSIONS: [&str; 4] = ["md", "markdown", "mdown", "markdn"];

/// Maximum number of HTTP redirects followed for URL imports.
pub const MAX_REDIRECTS: u32 = 5;

/// Per-request timeout for URL imports.
pub const REQUEST_TIMEOUT_MS: u64 = 15_000;

/// Maximum number of recent files kept in memory / prefs.
pub const RECENT_MAX: usize = 10;

/// Maximum number of mermaid diagrams rendered per document.
pub const MAX_MERMAID_DIAGRAMS: usize = 24;

/// Maximum byte size of one mermaid diagram source, 100 KiB.
pub const MAX_MERMAID_SOURCE_BYTES: usize = 100 * 1024;

/// Sidebar width bounds (pixels), as enforced by the original UI.
pub const SIDEBAR_WIDTH_MIN: u16 = 180;
pub const SIDEBAR_WIDTH_MAX: u16 = 400;

/// Per-document zoom bounds, as enforced by the original UI.
pub const ZOOM_MIN: f32 = 0.5;
pub const ZOOM_MAX: f32 = 3.0;

/// GitHub repository used for update checks.
pub const GITHUB_REPO: &str = "rizkidarmawan21/markfly.md";

/// Hosts allowed for update lookups and installer downloads.
pub const UPDATE_API_HOST: &str = "api.github.com";
pub const UPDATE_DOWNLOAD_HOSTS: [&str; 2] = ["github.com", "release-assets.githubusercontent.com"];

/// Maximum installer size for updates, 250 MiB.
pub const MAX_PACKAGE_BYTES: u64 = 250 * 1024 * 1024;

/// Update check interval, 24 hours.
pub const UPDATE_CHECK_INTERVAL_MS: u64 = 24 * 60 * 60 * 1000;
