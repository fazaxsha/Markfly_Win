//! Markfly core: pure-Rust domain logic shared by the desktop shell.
//!
//! Modules mirror the responsibilities split across the original Electron
//! main process, preload script and security helpers:
//!
//! | Original (Electron)      | Rust replacement             |
//! |--------------------------|------------------------------|
//! | `marked` renderer chain | [`markdown`]                 |
//! | `markdown-security.ts`   | [`markdown`] policies        |
//! | `AuthorizedFiles`        | [`fsops`]                    |
//! | `sanitizePreferences`    | [`prefs`]                    |
//! | chokidar watcher         | [`watcher`]                  |
//! | `fetchMarkdownUrl`       | [`netfetch`]                 |
//! | outline panel lexer      | [`documents`]                |
pub mod config;
pub mod documents;
pub mod error;
pub mod fsops;
pub mod markdown;
pub mod netfetch;
pub mod prefs;
pub mod recent;
pub mod watcher;

pub use error::MarkflyError;
pub use fsops::{
    assert_markdown_size, create_markdown_file, ensure_markdown_extension, is_markdown_path,
    read_authorized_markdown, read_markdown_file, write_authorized_markdown, write_markdown_file,
    AuthorizedFiles,
};
pub use markdown::{render_preview, render_preview_with_outline};
pub use netfetch::fetch_markdown_url;
pub use prefs::Prefs;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_extension_policy() {
        assert!(is_markdown_path("notes.md"));
        assert!(is_markdown_path("README.MARKDOWN"));
        assert!(is_markdown_path(r"C:\Users\me\docs\todo.mdown"));
        assert!(is_markdown_path("/home/me/readme.markdn"));
        assert!(!is_markdown_path("photo.png"));
        assert!(!is_markdown_path("Makefile"));
        assert!(!is_markdown_path(""));
    }

    #[test]
    fn render_produces_github_compatible_html() {
        let html = render_preview("# Title\n\nHello **world** and `code`.\n");
        assert!(html.contains("<h1 id=\"markfly-heading-0\">Title</h1>"));
        assert!(html.contains("<strong>world</strong>"));
        assert!(html.contains("<code>code</code>"));
    }

    #[test]
    fn render_escapes_raw_html() {
        let html = render_preview("Hello <script>alert(1)</script>\n\n<div onclick=\"x\">y</div>\n");
        // Raw HTML is escaped to text, never emitted as live markup.
        assert!(!html.contains("<script>"));
        assert!(!html.contains("<div"));
        assert!(html.contains("&lt;div onclick="));
    }

    #[test]
    fn render_blocks_remote_images_but_allows_base64() {
        let blocked = render_preview("![alt](https://example.com/a.png)");
        assert!(blocked.contains("markfly-blocked-image"));

        let allowed = render_preview("![alt](data:image/png;base64,iVBORw0KGgo=)");
        assert!(allowed.contains("data:image/png;base64,iVBORw0KGgo="));
        assert!(!allowed.contains("markfly-blocked-image"));
    }

    #[test]
    fn render_builds_mermaid_scaffolding() {
        let html = render_preview("```mermaid\ngraph TD;\n  A-->B;\n```\n");
        assert!(html.contains("class=\"mermaid-block\""));
        assert!(html.contains("data-mermaid-index=\"0\""));
        assert!(html.contains("data-mermaid-mode=\"diagram\""));
        assert!(html.contains("<pre class=\"mermaid\" data-mermaid-index=\"0\">graph TD;"));
    }

    #[test]
    fn render_tables_strikethrough_and_tasklists() {
        let html = render_preview("| a | b |\n|---|---|\n| 1 | 2 |\n\n~~gone~~\n\n- [x] done\n- [ ] todo\n");
        assert!(html.contains("<table>"));
        assert!(html.contains("<th>"));
        assert!(html.contains("<del>gone</del>"));
        assert!(html.contains("class=\"task-list-item\""));
        assert!(html.contains("<input checked=\"\" disabled=\"\" type=\"checkbox\">"));
        assert!(html.contains("<input disabled=\"\" type=\"checkbox\">"));
    }

    #[test]
    fn outline_matches_renderer_heading_ids() {
        let doc = "# One\n\ntext\n\n## Two *em*\n\n### Three\n";
        let (html, outline) = render_preview_with_outline(doc);
        assert_eq!(outline.len(), 3);
        assert_eq!(outline[0].id, "markfly-heading-0");
        assert_eq!(outline[1].id, "markfly-heading-1");
        assert_eq!(outline[1].text, "Two em");
        assert!(html.contains("id=\"markfly-heading-1\""));
        assert!(html.contains("id=\"markfly-heading-2\""));
    }

    #[test]
    fn size_guard_matches_original() {
        assert!(assert_markdown_size(1024).is_ok());
        assert!(assert_markdown_size(config::MAX_MARKDOWN_BYTES + 1).is_err());
    }

    #[test]
    fn authorized_files_gate_reads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.md");
        std::fs::write(&path, "# hi").unwrap();
        let path_str = path.to_str().unwrap();

        let mut authorized = AuthorizedFiles::new();
        assert!(!authorized.has(path_str));
        let content = read_authorized_markdown(&authorized, path_str);
        assert!(content.is_err());

        let normalized = authorized.authorize(path_str).unwrap();
        let normalized_str = normalized.to_string_lossy().into_owned();
        assert!(authorized.has(&normalized_str));
        let content = read_authorized_markdown(&authorized, &normalized_str).unwrap();
        assert_eq!(content, "# hi");
    }

    #[test]
    fn write_roundtrip_respects_markdown_policy() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("new-note.md");
        let path_str = path.to_str().unwrap();

        fsops::write_markdown_file(path_str, "# Hello\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# Hello\n");

        let mut authorized = AuthorizedFiles::new();
        assert!(fsops::write_authorized_markdown(&authorized, path_str, "x").is_err());
        authorized.authorize(path_str).unwrap();
        fsops::write_authorized_markdown(&authorized, path_str, "# Updated\n").unwrap();
        assert_eq!(read_markdown_file(path_str).unwrap(), "# Updated\n");

        let png = dir.path().join("a.png");
        assert!(fsops::write_markdown_file(png.to_str().unwrap(), "x").is_err());
        assert!(fsops::write_markdown_file(path_str, &"a".repeat(config::MAX_MARKDOWN_BYTES as usize + 1)).is_err());
    }

    #[test]
    fn creates_empty_markdown_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.md");
        fsops::create_markdown_file(path.to_str().unwrap()).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);
        fsops::write_markdown_file(path.to_str().unwrap(), "# keep\n").unwrap();
        fsops::create_markdown_file(path.to_str().unwrap()).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# keep\n");
    }

    #[test]
    fn extension_is_appended_when_missing() {
        assert_eq!(
            fsops::ensure_markdown_extension(r"C:\notes\journal").unwrap(),
            r"C:\notes\journal.md"
        );
        assert_eq!(
            fsops::ensure_markdown_extension("notes").unwrap(),
            "notes.md"
        );
        assert_eq!(
            fsops::ensure_markdown_extension("notes.markdown").unwrap(),
            "notes.markdown"
        );
        assert!(fsops::ensure_markdown_extension(r"C:\notes\journal.txt").is_err());
        assert!(fsops::ensure_markdown_extension(r"C:\notes\").is_err());
    }

    #[test]
    fn github_blob_urls_convert_to_raw() {
        let blob = url::Url::parse(
            "https://github.com/owner/repo/blob/main/docs/manual.md",
        )
        .unwrap();
        let raw = netfetch::github_blob_to_raw(&blob).unwrap();
        assert_eq!(
            raw.as_str(),
            "https://raw.githubusercontent.com/owner/repo/main/docs/manual.md"
        );

        let not_blob =
            url::Url::parse("https://github.com/owner/repo/tree/main/docs").unwrap();
        assert!(netfetch::github_blob_to_raw(&not_blob).is_none());
    }

    #[test]
    fn ip_public_policy_matches_block_list() {
        use std::net::IpAddr;
        let public: IpAddr = "93.184.216.34".parse().unwrap();
        assert!(netfetch::is_public_ip(public));

        for private in [
            "127.0.0.1",
            "10.1.2.3",
            "192.168.1.1",
            "172.16.0.9",
            "169.254.1.1",
            "100.64.0.1",
            "0.0.0.0",
        ] {
            let ip: IpAddr = private.parse().unwrap();
            assert!(!netfetch::is_public_ip(ip), "{private} must be blocked");
        }

        assert!(!netfetch::is_public_ip("::1".parse().unwrap()));
        assert!(!netfetch::is_public_ip("fe80::1".parse().unwrap()));
        assert!(!netfetch::is_public_ip("::ffff:127.0.0.1".parse().unwrap()));
        assert!(netfetch::is_public_ip("2606:4700:4700::1111".parse().unwrap()));
    }

    #[test]
    fn external_url_policy() {
        assert!(netfetch::is_safe_external_url("https://example.com"));
        assert!(!netfetch::is_safe_external_url("http://example.com"));
        assert!(!netfetch::is_safe_external_url(
            "https://user:pass@example.com"
        ));
        assert!(!netfetch::is_safe_external_url("https://localhost:5173"));
        assert!(!netfetch::is_safe_external_url("not-a-url"));
    }

    #[test]
    fn prefs_sanitize_filters_and_clamps() {
        let payload = serde_json::json!({
            "dark": true,
            "sidebarWidth": 900,
            "tabs": [
                {"path": "C:\\docs\\a.md", "active": true},
                {"path": "C:\\docs\\b.md", "active": false},
                {"path": "https://example.com/x.md"},
                {"path": "C:\\docs\\c.png"}
            ],
            "activePath": "C:\\docs\\b.md",
            "views": {
                "C:\\docs\\a.md": {"zoom": 99.0, "showRaw": true},
                "C:\\docs\\zzz.md": {"zoom": 1.0}
            }
        });

        let prefs = Prefs::sanitize(&payload, |path| path.ends_with("a.md"));
        assert_eq!(prefs.dark, Some(true));
        assert_eq!(prefs.sidebar_width, Some(400));
        let paths: Vec<&str> = prefs.tabs.iter().map(|t| t.path.as_str()).collect();
        assert_eq!(paths, vec!["C:\\docs\\a.md", "https://example.com/x.md"]);
        assert_eq!(prefs.active_path, None); // not in filtered tabs
        let view = prefs.views.get("C:\\docs\\a.md").unwrap();
        assert_eq!(view.zoom, 3.0);
        assert!(view.show_raw);
        assert!(prefs.views.get("C:\\docs\\zzz.md").is_none());
    }

    #[test]
    fn recent_files_dedupe_and_cap() {
        use std::path::PathBuf;
        let mut recent = recent::RecentFiles::new();
        for index in 0..15 {
            recent.add(PathBuf::from(format!("file-{index}.md")));
        }
        assert_eq!(recent.paths().len(), config::RECENT_MAX);
        recent.add(PathBuf::from("file-0.md"));
        assert_eq!(recent.paths()[0], PathBuf::from("file-0.md"));
    }

    #[test]
    fn document_name_handles_windows_paths() {
        assert_eq!(
            documents::document_name(r"C:\Users\me\notes\todo.md"),
            "todo.md"
        );
        assert_eq!(documents::document_name("/home/me/readme.md"), "readme.md");
        assert_eq!(
            documents::document_name("https://example.com/docs/manual.md"),
            "manual.md"
        );
    }

    #[test]
    fn display_path_shortens_home_directories() {
        assert_eq!(
            documents::display_path(r"C:\Users\me\notes\todo.md"),
            r"~\notes\todo.md"
        );
        assert_eq!(documents::display_path("/Users/me/notes/a.md"), "~/notes/a.md");
        assert_eq!(documents::display_path(r"D:\work\a.md"), r"D:\work\a.md");
    }
}
