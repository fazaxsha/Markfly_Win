//! Markdown -> sanitized HTML pipeline.
//!
//! Rust replacement for the original `marked` + renderer overrides +
//! `DOMPurify` chain (see `src/App.vue` and `src/markdown-security.ts`):
//! GFM-flavored parsing via `pulldown-cmark`, security policies reimplemented
//! (raw HTML escaping, base64-data-image-only, mermaid limits) and a final
//! sanitize pass with `ammonia` (allow-list based, like DOMPurify).
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use crate::config::{MAX_MERMAID_DIAGRAMS, MAX_MERMAID_SOURCE_BYTES};

/// Renders markdown to sanitized HTML, GitHub-compatible for
/// `github-markdown-css`. Heading ids are `markfly-heading-{index}` in
/// document order, matching the outline panel ids.
pub fn render_preview(markdown: &str) -> String {
    let (html, _outline) = render_preview_with_outline(markdown);
    html
}

/// Renders markdown to sanitized HTML while also returning the outline, in a
/// single parse pass.
pub fn render_preview_with_outline(markdown: &str) -> (String, Vec<crate::documents::Heading>) {
    let mut renderer = Renderer::default();
    let html = renderer.render(markdown);
    (sanitize(&html), renderer.outline)
}

/// Mirrors `canRenderMermaid` from `src/markdown-security.ts`.
pub fn can_render_mermaid(index: usize, source_bytes: usize) -> bool {
    index < MAX_MERMAID_DIAGRAMS && source_bytes <= MAX_MERMAID_SOURCE_BYTES
}

#[derive(Default)]
struct CodeBlockBuffer {
    language: String,
    is_mermaid: bool,
    text: String,
}

#[derive(Default)]
struct Renderer {
    out: String,
    outline: Vec<crate::documents::Heading>,
    heading_index: usize,
    mermaid_count: usize,
    current_heading: Option<(u8, String)>,
    code_block: Option<CodeBlockBuffer>,
    table_aligns: Vec<Alignment>,
    cell_index: usize,
    in_table_head: bool,
    // Task list handling: a <li> is written optimistically; when a task
    // marker follows immediately, the class is spliced into the tag and the
    // wrapping <p> is suppressed (matches marked's output shape).
    li_open_index: Option<usize>,
    in_item: bool,
    item_has_content: bool,
    task_item: bool,
    paragraph_pending: bool,
    paragraph_suppressed: bool,
}

impl Renderer {
    fn render(&mut self, markdown: &str) -> String {
        let options = Options::ENABLE_TABLES
            | Options::ENABLE_STRIKETHROUGH
            | Options::ENABLE_TASKLISTS
            | Options::ENABLE_MATH;
        let events: Vec<Event> = Parser::new_ext(markdown, options).collect();
        for event in events {
            self.handle_event(event);
        }
        if let Some((level, text)) = self.current_heading.take() {
            self.push_heading(level, &text);
        }
        std::mem::take(&mut self.out)
    }

    fn handle_event(&mut self, event: Event) {
        match event {
            Event::Start(tag) => self.start_tag(tag),
            Event::End(tag) => self.end_tag(tag),            Event::Text(text) => {
                if let Some(block) = self.code_block.as_mut() {
                    block.text.push_str(&text);
                    return;
                }
                self.flush_pending_paragraph();
                self.item_has_content = true;
                if let Some((_, buffer)) = self.current_heading.as_mut() {
                    buffer.push_str(&text);
                }
                let _ = write!(self.out, "{}", escape_html(&text));
            }
            Event::Code(code) => {
                if let Some(block) = self.code_block.as_mut() {
                    block.text.push_str(&code);
                    return;
                }
                self.flush_pending_paragraph();
                self.item_has_content = true;
                let _ = write!(self.out, "<code>{}</code>", escape_html(&code));
            }
            // Raw HTML (block and inline) is fully escaped, like escapeRawHtml.
            Event::Html(html) => {
                let _ = write!(self.out, "{}", escape_html(&html));
            }
            Event::InlineHtml(html) => {
                let _ = write!(self.out, "{}", escape_html(&html));
            }
            Event::SoftBreak => {
                let _ = write!(self.out, "\n");
            }
            Event::HardBreak => {
                let _ = write!(self.out, "<br>\n");
            }
            Event::Rule => {
                let _ = write!(self.out, "<hr>\n");
            }
            // Math is passed through as `$...$` / `$$...$$` text delimiters;
            // the renderer applies KaTeX to the resulting text nodes.
            Event::InlineMath(math) => {
                let _ = write!(self.out, "${}$", escape_html(&math));
            }
            Event::DisplayMath(math) => {
                let _ = write!(self.out, "$${}$$", escape_html(&math));
            }
            Event::TaskListMarker(checked) => self.task_list_marker(checked),
            Event::FootnoteReference(name) => {
                let _ = write!(
                    self.out,
                    "<sup class=\"footnote-ref\"><a href=\"#fn-{0}\" id=\"fnref-{0}\">{1}</a></sup>",
                    escape_html(&name),
                    escape_html(&name)
                );
            }
        }
    }

    fn start_tag(&mut self, tag: Tag) {
        // Flush a deferred <p> before writing any tag, so inline tags nest
        // inside the paragraph they belong to.
        self.flush_pending_paragraph();
        match tag {
            Tag::Paragraph => {
                self.paragraph_pending = true;
                self.paragraph_suppressed = false;
            }
            Tag::Heading { level, .. } => {
                let level = heading_level(level);
                self.current_heading = Some((level, String::new()));
            }
            Tag::BlockQuote(_) => self.push_line("<blockquote>"),
            Tag::CodeBlock(kind) => {
                let language = match kind {
                    CodeBlockKind::Fenced(lang) => lang
                        .trim()
                        .split_whitespace()
                        .next()
                        .unwrap_or_default()
                        .to_ascii_lowercase(),
                    CodeBlockKind::Indented => String::new(),
                };
                self.code_block = Some(CodeBlockBuffer {
                    is_mermaid: language == "mermaid",
                    language,
                    text: String::new(),
                });
            }
            Tag::List(first_number) => match first_number {
                Some(start) if start != 1 => {
                    let _ = write!(self.out, "<ol start=\"{}\">\n", start);
                }
                _ => self.push_line("<ol>"),
            },
            Tag::Item => {
                self.push_inline("<li>");
                self.li_open_index = Some(self.out.len() - 4); // index of "<li>"
                self.in_item = true;
                self.item_has_content = false;
                self.task_item = false;
            }
            Tag::Emphasis => self.push_inline("<em>"),
            Tag::Strong => self.push_inline("<strong>"),
            Tag::Strikethrough => self.push_inline("<del>"),
            Tag::Link { dest_url, title, .. } => {
                let title_attr = if title.is_empty() {
                    String::new()
                } else {
                    format!(" title=\"{}\"", escape_html(&title))
                };
                let _ = write!(
                    self.out,
                    "<a href=\"{}\"{}>",
                    escape_html(&dest_url),
                    title_attr
                );
            }
            Tag::Image { dest_url, title, .. } => self.image(&dest_url, &title),
            Tag::Table(alignments) => {
                self.table_aligns = alignments;
                self.cell_index = 0;
                self.push_line("<table>");
            }
            Tag::TableHead => {
                self.push_line("<thead>");
                self.in_table_head = true;
            }
            Tag::TableRow => {
                self.cell_index = 0;
                self.push_line("<tr>");
            }
            Tag::TableCell => {
                let align = self
                    .table_aligns
                    .get(self.cell_index)
                    .copied()
                    .unwrap_or(Alignment::None);
                let align_attr = match align {
                    Alignment::None | Alignment::Left => "",
                    Alignment::Center => " align=\"center\"",
                    Alignment::Right => " align=\"right\"",
                };
                self.cell_index += 1;
                let tag = if self.in_table_head { "th" } else { "td" };
                let _ = write!(self.out, "<{tag}{align_attr}>");
            }
            Tag::FootnoteDefinition(name) => {
                let _ = write!(
                    self.out,
                    "<section class=\"footnote\" id=\"fn-{}\">\n",
                    escape_html(&name)
                );
            }
            // HtmlBlock / definition lists / metadata are not rendered.
            _ => {}
        }
    }

    fn end_tag(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                self.end_paragraph();
            }
            TagEnd::Heading(_) => {
                self.close_heading();
            }
            TagEnd::BlockQuote(_) => self.push_line("</blockquote>"),
            TagEnd::CodeBlock => {
                if let Some(block) = self.code_block.take() {
                    self.emit_code_block(&block);
                }
            }
            TagEnd::List(_) => self.push_line("</ol>"),
            TagEnd::Item => {
                let _ = write!(self.out, "</li>\n");
                self.in_item = false;
                self.li_open_index = None;
                self.item_has_content = false;
            }
            TagEnd::Emphasis => self.push_inline("</em>"),
            TagEnd::Strong => self.push_inline("</strong>"),
            TagEnd::Strikethrough => self.push_inline("</del>"),
            TagEnd::Link => self.push_inline("</a>"),
            TagEnd::Image => {}
            TagEnd::Table => self.push_line("</table>"),
            TagEnd::TableHead => {
                self.in_table_head = false;
                self.push_line("</thead>");
            }
            TagEnd::TableRow => self.push_line("</tr>"),
            TagEnd::TableCell => {
                let tag = if self.in_table_head { "th" } else { "td" };
                let _ = write!(self.out, "</{tag}>\n");
            }
            TagEnd::FootnoteDefinition => self.push_line("</section>"),
            // HtmlBlock / definition lists / metadata are not rendered.
            _ => {}
        }
    }

    fn push_line(&mut self, line: &str) {
        let _ = write!(self.out, "{line}\n");
    }

    /// Writes an inline (non-block) tag without a trailing newline.
    fn push_inline(&mut self, tag: &str) {
        let _ = write!(self.out, "{tag}");
    }

    fn flush_pending_paragraph(&mut self) {
        // Called when real content arrives: emit the deferred <p> start.
        if self.paragraph_pending {
            self.paragraph_pending = false;
            if !self.paragraph_suppressed {
                let _ = write!(self.out, "<p>");
            }
        }
    }

    fn end_paragraph(&mut self) {
        if self.paragraph_pending {
            // An empty paragraph: emit nothing.
            self.paragraph_pending = false;
            self.paragraph_suppressed = false;
            return;
        }
        if !self.paragraph_suppressed {
            let _ = write!(self.out, "</p>\n");
        }
        self.paragraph_suppressed = false;
    }

    fn task_list_marker(&mut self, checked: bool) {
        // The marker follows immediately after the item/paragraph opens.
        if self.li_open_index.is_some() && !self.item_has_content {
            if let Some(index) = self.li_open_index.take() {
                self.out
                    .insert_str(index + 3, r#" class="task-list-item""#);
            }
            self.task_item = true;
        }
        // Suppress the <p> wrapper for task items.
        self.paragraph_pending = false;
        self.paragraph_suppressed = true;
        if checked {
            let _ = write!(self.out, r#"<input checked="" disabled="" type="checkbox">"#);
        } else {
            let _ = write!(self.out, r#"<input disabled="" type="checkbox">"#);
        }
    }

    fn close_heading(&mut self) {
        if let Some((level, text)) = self.current_heading.take() {
            self.push_heading(level, &text);
        }
    }

    fn push_heading(&mut self, level: u8, text: &str) {
        let id = format!("markfly-heading-{}", self.heading_index);
        self.heading_index += 1;
        let _ = write!(
            self.out,
            "<h{level} id=\"{id}\">{}</h{level}>\n",
            escape_html(text.trim())
        );
        self.outline.push(crate::documents::Heading {
            id,
            level,
            text: text.trim().to_string(),
        });
    }

    fn emit_code_block(&mut self, block: &CodeBlockBuffer) {
        if block.is_mermaid {
            let source = escape_html(&block.text);
            if !can_render_mermaid(self.mermaid_count, block.text.len()) {
                let _ = write!(
                    self.out,
                    "<pre><code class=\"language-mermaid\">{source}</code></pre>\n"
                );
                return;
            }
            let index = self.mermaid_count;
            self.mermaid_count += 1;
            let _ = write!(
                self.out,
                r#"<section class="mermaid-block" data-mermaid-index="{index}" data-mode="diagram">
<div class="mermaid-toolbar"><span class="mermaid-label">Mermaid</span><div class="mermaid-view-options" role="group" aria-label="Mermaid view">
<button type="button" data-mermaid-mode="diagram" aria-pressed="true">Diagram</button>
<button type="button" data-mermaid-mode="code" aria-pressed="false">Code</button>
<button type="button" data-mermaid-mode="split" aria-pressed="false">Diagram + Code</button>
</div></div>
<div class="mermaid-content"><div class="mermaid-diagram"><pre class="mermaid" data-mermaid-index="{index}">{source}</pre></div><pre class="mermaid-code"><code>{source}</code></pre></div>
</section>"#
            );
            return;
        }

        if block.language.is_empty() {
            let _ = write!(self.out, "<pre><code>{}</code></pre>\n", escape_html(&block.text));
        } else {
            let _ = write!(
                self.out,
                "<pre><code class=\"language-{}\">{}</code></pre>\n",
                escape_html(&block.language),
                escape_html(&block.text)
            );
        }
    }

    fn image(&mut self, dest_url: &str, title: &str) {
        // Only inline base64 data images are allowed; everything else is
        // blocked, like `isSafeImageHref` in the original implementation.
        if !is_safe_image_href(dest_url) {
            let _ = write!(
                self.out,
                r#"<span class="markfly-blocked-image" title="Remote image blocked">[Image blocked]</span>"#
            );
            return;
        }
        let title_attr = if title.is_empty() {
            String::new()
        } else {
            format!(" title=\"{}\"", escape_html(title))
        };
        let _ = write!(
            self.out,
            "<img src=\"{}\"{}>",
            escape_html(dest_url),
            title_attr
        );
    }
}

/// Sanitizes the rendered HTML with an allow-list; the role DOMPurify played
/// in the original renderer pipeline.
fn sanitize(html: &str) -> String {
    let tags: HashSet<&str> = [
        "a", "abbr", "annotation", "article", "aside", "b", "blockquote", "br", "button", "caption",
        "cite", "code", "col", "colgroup", "dd", "del", "details", "dfn", "div", "dl", "dt", "em",
        "figcaption", "figure", "h1", "h2", "h3", "h4", "h5", "h6", "hr", "i", "img", "input", "ins",
        "kbd", "li", "mark", "ol", "p", "pre", "q", "rp", "rt", "ruby", "s", "samp", "section",
        "small", "span", "strike", "strong", "sub", "summary", "sup", "table", "tbody", "td",
        "tfoot", "th", "thead", "time", "tr", "u", "ul", "var", "wbr",
    ]
    .into_iter()
    .collect();

    let mut tag_attributes: HashMap<&str, HashSet<&str>> = HashMap::new();
    // `rel` is intentionally omitted: ammonia's link_rel adds
    // "noopener noreferrer" itself and panics if `rel` is allow-listed.
    tag_attributes.insert("a", ["href", "title", "target"].into());
    tag_attributes.insert("img", ["src", "alt", "title", "width", "height"].into());
    tag_attributes.insert("input", ["type", "checked", "disabled"].into());
    tag_attributes.insert("button", ["type"].into());
    tag_attributes.insert("th", ["align", "colspan", "rowspan"].into());
    tag_attributes.insert("td", ["align", "colspan", "rowspan"].into());
    tag_attributes.insert("ol", ["start"].into());
    tag_attributes.insert("time", ["datetime"].into());
    tag_attributes.insert("abbr", ["title"].into());
    tag_attributes.insert("q", ["cite"].into());
    tag_attributes.insert("del", ["cite"].into());
    tag_attributes.insert("ins", ["cite"].into());
    tag_attributes.insert("details", ["open"].into());

    let mut builder = ammonia::Builder::new();
    builder.tags(tags);
    builder.tag_attributes(tag_attributes);
    builder.generic_attributes(["class", "id"].into_iter().collect());
    builder.generic_attribute_prefixes(["data-", "aria-"].into_iter().collect());
    // "data" is only needed for inline base64 images; our image policy
    // restricts it further to `data:image/*;base64,...` before sanitizing.
    builder.url_schemes(["http", "https", "mailto", "tel", "data"].into_iter().collect());
    builder.url_relative(ammonia::UrlRelative::PassThrough);
    builder.strip_comments(true);
    builder.clean_content_tags(["script", "style"].into_iter().collect());
    builder.clean(html).to_string()
}

/// Mirrors `isSafeImageHref` from `src/markdown-security.ts`.
fn is_safe_image_href(value: &str) -> bool {
    let Some(rest) = strip_image_prefix(value) else {
        return false;
    };
    match rest.split_once(',') {
        Some((encoding, payload))
            if encoding.eq_ignore_ascii_case("base64")
                && !payload.is_empty()
                && payload.len() % 4 == 0
                && payload
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/' || b == b'=') =>
        {
            true
        }
        _ => false,
    }
}

fn strip_image_prefix(value: &str) -> Option<&str> {
    let lower = value.to_ascii_lowercase();
    for prefix in [
        "data:image/png;",
        "data:image/gif;",
        "data:image/jpeg;",
        "data:image/jpg;",
        "data:image/webp;",
        "data:image/avif;",
    ] {
        if lower.starts_with(prefix) {
            return Some(&value[prefix.len()..]);
        }
    }
    None
}

/// HTML-escapes `& < > " '`, matching pulldown-cmark's `escape_html` and the
/// original `escapeRawHtml` policy.
fn escape_html(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}
