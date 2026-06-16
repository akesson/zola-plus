use std::collections::{BTreeSet, HashMap};
use std::fmt::Write;
use std::sync::Mutex;

use crate::markdown::cmark::CowStr;
use errors::bail;
use gh_emoji::Replacer as EmojiReplacer;
use giallo::{HtmlRenderer, ParsedFence, parse_markdown_fence};
use log;
use once_cell::sync::Lazy;
use pulldown_cmark as cmark;
use pulldown_cmark_escape as cmark_escape;

use crate::context::RenderContext;
use errors::{Context, Error, Result};
use pulldown_cmark_escape::escape_html;
use regex::{Regex, RegexBuilder};
use utils::net::is_external_link;
use utils::site::resolve_internal_link;
use utils::slugs::slugify_anchors;
use utils::table_of_contents::{Heading, make_table_of_contents};
use utils::types::InsertAnchor;

use self::cmark::{Alignment, Event, LinkType, Options, Parser, Tag, TagEnd};
use crate::shortcode::{SHORTCODE_PLACEHOLDER, Shortcode};

const CONTINUE_READING: &str = "<span id=\"continue-reading\"></span>";
const SUMMARY_CUTOFF_TEMPLATE: &str = "summary-cutoff.html";
const ANCHOR_LINK_TEMPLATE: &str = "anchor-link.html";
static EMOJI_REPLACER: Lazy<EmojiReplacer> = Lazy::new(EmojiReplacer::new);

/// Set as a regex to help match some extra cases. This way, spaces and case don't matter.
static MORE_DIVIDER_RE: Lazy<Regex> = Lazy::new(|| {
    RegexBuilder::new(r#"<!--\s*more\s*-->"#)
        .case_insensitive(true)
        .dot_matches_new_line(true)
        .build()
        .unwrap()
});

static FOOTNOTES_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"<sup class="footnote-reference"( id=\s*.*?)?><a href=\s*.*?>\s*.*?</a></sup>"#)
        .unwrap()
});

/// Although there exists [a list of registered URI schemes][uri-schemes], a link may use arbitrary,
/// private schemes. This regex checks if the given string starts with something that just looks
/// like a scheme, i.e., a case-insensitive identifier followed by a colon.
///
/// [uri-schemes]: https://www.iana.org/assignments/uri-schemes/uri-schemes.xhtml
static STARTS_WITH_SCHEMA_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[0-9A-Za-z\-]+:").unwrap());

/// Matches a <a>..</a> tag, getting the opening tag in a capture group.
/// Used only with AnchorInsert::Heading to grab it from the template
static A_HTML_TAG: Lazy<Regex> = Lazy::new(|| Regex::new(r"(<\s*a[^>]*>).*?<\s*/\s*a>").unwrap());

/// Efficiently insert multiple element in their specified index.
/// The elements should sorted in ascending order by their index.
///
/// This is done in O(n) time.
fn insert_many<T>(input: &mut Vec<T>, elem_to_insert: Vec<(usize, T)>) {
    let mut inserted = vec![];
    let mut last_idx = 0;

    for (idx, elem) in elem_to_insert.into_iter() {
        let head_len = idx - last_idx;
        inserted.extend(input.splice(0..head_len, std::iter::empty()));
        inserted.push(elem);
        last_idx = idx;
    }
    let len = input.len();
    inserted.extend(input.drain(0..len));

    *input = inserted;
}

/// Colocated asset links refers to the files in the same directory.
fn is_colocated_asset_link(link: &str) -> bool {
    !link.starts_with('/')
        && !link.starts_with("..")
        && !link.starts_with('#')
        && !STARTS_WITH_SCHEMA_RE.is_match(link)
}

#[derive(Debug)]
pub struct Rendered {
    pub body: String,
    pub summary: Option<String>,
    pub toc: Vec<Heading>,
    /// Links to site-local pages: relative path plus optional anchor target.
    pub internal_links: Vec<(String, Option<String>)>,
    /// Outgoing links to external webpages (i.e. HTTP(S) targets).
    pub external_links: Vec<String>,
}

/// Tracks a heading in a slice of pulldown-cmark events
#[derive(Debug)]
struct HeadingRef {
    start_idx: usize,
    end_idx: usize,
    level: u32,
    id: Option<String>,
    classes: Vec<String>,
}

impl HeadingRef {
    fn new(start: usize, level: u32, anchor: Option<String>, classes: &[String]) -> HeadingRef {
        HeadingRef { start_idx: start, end_idx: 0, level, id: anchor, classes: classes.to_vec() }
    }

    fn to_html(&self, id: &str) -> String {
        let mut buffer = String::with_capacity(100);
        buffer.write_str("<h").unwrap();
        buffer.write_str(&format!("{}", self.level)).unwrap();

        buffer.write_str(" id=\"").unwrap();
        escape_html(&mut buffer, id).unwrap();
        buffer.write_str("\"").unwrap();

        if !self.classes.is_empty() {
            buffer.write_str(" class=\"").unwrap();
            let num_classes = self.classes.len();

            for (i, class) in self.classes.iter().enumerate() {
                escape_html(&mut buffer, class).unwrap();
                if i < num_classes - 1 {
                    buffer.write_str(" ").unwrap();
                }
            }

            buffer.write_str("\"").unwrap();
        }

        buffer.write_str(">").unwrap();
        buffer
    }
}

// We might have cases where the slug is already present in our list of anchor
// for example an article could have several titles named Example
// We add a counter after the slug if the slug is already present, which
// means we will have example, example-1, example-2 etc
fn find_anchor(anchors: &[String], name: String, level: u16) -> String {
    if level == 0 && !anchors.contains(&name) {
        return name;
    }

    let new_anchor = format!("{}-{}", name, level + 1);
    if !anchors.contains(&new_anchor) {
        return new_anchor;
    }

    find_anchor(anchors, name, level + 1)
}

fn fix_link(
    link_type: LinkType,
    link: &str,
    context: &RenderContext,
    internal_links: &mut Vec<(String, Option<String>)>,
    external_links: &mut Vec<String>,
) -> Result<String> {
    if link_type == LinkType::Email {
        return Ok(link.to_string());
    }

    // A few situations here:
    // - it could be a relative link (starting with `@/`)
    // - it could be a link to a co-located asset
    // - it could be a normal link
    let result = if link.starts_with("@/") {
        match resolve_internal_link(link, &context.permalinks) {
            Ok(resolved) => {
                internal_links.push((resolved.md_path, resolved.anchor));
                resolved.permalink
            }
            Err(_) => {
                let msg = format!(
                    "Broken relative link `{}` in {}",
                    link,
                    context.current_page_path.unwrap_or("unknown"),
                );
                match context.config.link_checker.internal_level {
                    config::LinkCheckerLevel::Error => bail!(msg),
                    config::LinkCheckerLevel::Warn => {
                        log::warn!("{msg}");
                        link.to_string()
                    }
                }
            }
        }
    } else if is_colocated_asset_link(link) {
        format!("{}{}", context.current_page_permalink, link)
    } else if is_external_link(link) {
        external_links.push(link.to_owned());
        link.to_owned()
    } else if link == "#" {
        link.to_string()
    } else if let Some(stripped_link) = link.strip_prefix('#') {
        // local anchor without the internal zola path
        if let Some(current_path) = context.current_page_path {
            internal_links.push((current_path.to_owned(), Some(stripped_link.to_owned())));
            format!("{}{}", context.current_page_permalink, &link)
        } else {
            link.to_string()
        }
    } else {
        link.to_string()
    };

    Ok(result)
}

/// get only text in a slice of events
fn get_text(parser_slice: &[Event]) -> String {
    let mut title = String::new();

    for event in parser_slice.iter() {
        match event {
            Event::Text(text) | Event::Code(text) => title += text,
            _ => continue,
        }
    }

    title
}

// --- Responsive tables (reflow + transpose) ----------------------------------
//
// A markdown table becomes responsive on narrow containers when it carries a
// directive on its own line directly above it (with a blank line in between):
//
//   * `<!-- reflow: <length> -->` — each row turns into a "card" of
//     `Header  Value` pairs. We wrap the table in
//     `<div class="reflow reflow-bp-<token>">`, give every body `<td>` a
//     `data-label` of its column header, and add `scope="col"` to the headers.
//   * `<!-- transpose: <length> -->` — the table visually flips so the header
//     row becomes a left-hand label column. We only wrap it in
//     `<div class="transpose transpose-bp-<token> transpose-cols-<n>">`; the
//     cells are left untouched (a pure CSS-grid flip, see `transpose_css`).
//
// The breakpoint lives in the class (`reflow-bp-65rem`), not in the markup's CSS.
// Each distinct breakpoint used across the site gets one container query in the
// generated stylesheet (see `reflow_css` / `transpose_css`), which the site links
// once as `responsive-tables.css` — exactly how class-based syntax highlighting
// ships `giallo.css`. (A container query can't read a custom property in its
// condition, so the value must live in a rule keyed by class.) The generated CSS
// does the reflow/transpose *only* (no margins/colours/fonts); the stable
// `.reflow` / `.transpose` classes are the author's hook for cosmetic styling.
//
// A table without a directive is left completely untouched. See
// docs/content/documentation/content/tables.md.

/// The responsive treatment a directive selects for the table that follows it.
#[derive(Clone, Copy)]
enum TableMode {
    /// `<!-- reflow: <length> -->` — rows become cards (see `emit_reflow_table`).
    Reflow,
    /// `<!-- transpose: <length> -->` — table flips (see `emit_transpose_table`).
    Transpose,
}

/// Matches a per-table `<!-- reflow: <length> -->` directive comment.
static REFLOW_DIRECTIVE_RE: Lazy<Regex> = Lazy::new(|| {
    RegexBuilder::new(r#"<!--\s*reflow:\s*(.*?)\s*-->"#)
        .case_insensitive(true)
        .dot_matches_new_line(true)
        .build()
        .unwrap()
});

/// Matches a per-table `<!-- transpose: <length> -->` directive comment.
static TRANSPOSE_DIRECTIVE_RE: Lazy<Regex> = Lazy::new(|| {
    RegexBuilder::new(r#"<!--\s*transpose:\s*(.*?)\s*-->"#)
        .case_insensitive(true)
        .dot_matches_new_line(true)
        .build()
        .unwrap()
});

/// The responsive mode a raw-HTML event selects, if any. `reflow` is checked first
/// so that, were both somehow present in one comment, it would win the match — but
/// the two are mutually exclusive in practice.
fn directive_mode(html: &str) -> Option<TableMode> {
    if REFLOW_DIRECTIVE_RE.is_match(html) {
        Some(TableMode::Reflow)
    } else if TRANSPOSE_DIRECTIVE_RE.is_match(html) {
        Some(TableMode::Transpose)
    } else {
        None
    }
}

/// A CSS length usable as a container-query breakpoint: a number plus a unit. The
/// value ends up in a generated CSS rule (and, sanitized, in a class name), so it
/// is validated against this allowlist to keep a stray annotation from injecting
/// CSS or producing an invalid class.
static REFLOW_LENGTH_RE: Lazy<Regex> = Lazy::new(|| {
    RegexBuilder::new(r"^\d+(\.\d+)?(px|rem|em|ch|ex|vw|vh|vmin|vmax|cqw|cqh|cqi|cqb)$")
        .case_insensitive(true)
        .build()
        .unwrap()
});

fn is_valid_css_length(value: &str) -> bool {
    REFLOW_LENGTH_RE.is_match(value)
}

/// Breakpoints (`<!-- reflow: <length> -->` values) seen while rendering. The site
/// build reads this afterwards to generate one shared `reflow.css`, mirroring how
/// class-based highlighting ships `giallo.css`. It is a process-global set rather
/// than threaded state because rendering runs in parallel across pages and the
/// rendering API is shared with `zola serve`; the set only ever grows, so even
/// incremental rebuilds keep a complete superset (at worst a stale, unused rule).
static REFLOW_BREAKPOINTS: Lazy<Mutex<BTreeSet<String>>> =
    Lazy::new(|| Mutex::new(BTreeSet::new()));

fn record_reflow_breakpoint(bp: &str) {
    REFLOW_BREAKPOINTS.lock().unwrap().insert(bp.to_string());
}

/// The breakpoints recorded so far, for the generated responsive-table stylesheet.
pub fn reflow_breakpoints() -> BTreeSet<String> {
    REFLOW_BREAKPOINTS.lock().unwrap().clone()
}

/// Transpose specs (`<!-- transpose: <length> -->` breakpoint paired with the
/// table's column count) seen while rendering. The column count is baked into the
/// generated CSS as a literal `grid-template-rows: repeat(<n>, auto)` — `repeat()`
/// can't reliably take a custom property as its count — so it is recorded here
/// alongside the breakpoint. Process-global for the same reasons as
/// `REFLOW_BREAKPOINTS`.
static TRANSPOSE_BREAKPOINTS: Lazy<Mutex<BTreeSet<(String, usize)>>> =
    Lazy::new(|| Mutex::new(BTreeSet::new()));

fn record_transpose_breakpoint(bp: &str, ncols: usize) {
    TRANSPOSE_BREAKPOINTS.lock().unwrap().insert((bp.to_string(), ncols));
}

/// The transpose specs recorded so far, for the generated responsive-table stylesheet.
pub fn transpose_breakpoints() -> BTreeSet<(String, usize)> {
    TRANSPOSE_BREAKPOINTS.lock().unwrap().clone()
}

/// Parse a `<!-- reflow: … -->` / `<!-- transpose: … -->` directive with its
/// matching `re`. Returns the breakpoint (the validated CSS length, or `None` when
/// the value isn't a valid length) together with whether the comment was the entire
/// HTML event (so it can be dropped).
fn parse_directive(html: &str, re: &Regex) -> Option<(Option<String>, bool)> {
    let caps = re.captures(html)?;
    let whole = caps.get(0).unwrap().as_str();
    let value = caps.get(1).unwrap().as_str();
    let standalone = html.trim() == whole;
    if is_valid_css_length(value) {
        // Normalize case so `40REM` and `40rem` share one class and one CSS rule.
        Some((Some(value.to_ascii_lowercase()), standalone))
    } else {
        log::warn!(
            "Ignoring responsive-table directive `{}`: `{}` is not a valid CSS length.",
            whole.trim(),
            value
        );
        Some((None, standalone))
    }
}

fn reflow_align_attr(align: Alignment) -> &'static str {
    match align {
        Alignment::Left => " style=\"text-align: left\"",
        Alignment::Center => " style=\"text-align: center\"",
        Alignment::Right => " style=\"text-align: right\"",
        Alignment::None => "",
    }
}

/// Per-column header text, used for the `data-label` on body cells.
fn collect_headers(table: &[Event], ncols: usize) -> Vec<String> {
    let mut headers = vec![String::new(); ncols];
    let mut in_head = false;
    let mut col = 0usize;
    let mut j = 0;
    while j < table.len() {
        match &table[j] {
            Event::Start(Tag::TableHead) => {
                in_head = true;
                col = 0;
            }
            Event::End(TagEnd::TableHead) => in_head = false,
            Event::Start(Tag::TableRow) => col = 0,
            Event::Start(Tag::TableCell) => {
                let mut k = j + 1;
                while k < table.len() && !matches!(table[k], Event::End(TagEnd::TableCell)) {
                    k += 1;
                }
                if in_head && col < ncols {
                    headers[col] = get_text(&table[j + 1..k]);
                }
                col += 1;
                j = k; // continue from the End(TableCell)
            }
            _ => {}
        }
        j += 1;
    }
    headers
}

/// Apply each per-table responsive directive to the table that *immediately*
/// follows it; every other table passes through untouched. A `reflow` directive
/// wraps the table and rewrites its cells (see `emit_reflow_table`); a `transpose`
/// directive only wraps it (see `emit_transpose_table`). When both somehow precede
/// one table the last one wins — they are mutually exclusive.
fn transform_tables<'a>(events: &mut Vec<Event<'a>>) {
    if !events
        .iter()
        .any(|e| matches!(e, Event::Html(html) if directive_mode(html).is_some()))
    {
        return;
    }

    let old = std::mem::take(events);
    let mut out: Vec<Event> = Vec::with_capacity(old.len() + 16);
    // A pending directive (mode + breakpoint), awaiting its table.
    let mut pending: Option<(TableMode, String)> = None;
    let mut it = old.into_iter();

    while let Some(ev) = it.next() {
        match ev {
            Event::Html(ref html) if directive_mode(html).is_some() => {
                let mode = directive_mode(html).expect("guard checked it is a directive");
                let re = match mode {
                    TableMode::Reflow => &*REFLOW_DIRECTIVE_RE,
                    TableMode::Transpose => &*TRANSPOSE_DIRECTIVE_RE,
                };
                if let Some((bp, standalone)) = parse_directive(html, re) {
                    pending = bp.map(|bp| (mode, bp));
                    // Keep the event only if it carried more than the directive.
                    if !standalone {
                        out.push(ev);
                    }
                } else {
                    out.push(ev);
                }
            }
            Event::Start(Tag::Table(aligns)) => {
                // Collect the table region (up to and including End(Table)).
                let mut table: Vec<Event> = Vec::new();
                for e in it.by_ref() {
                    let is_end = matches!(e, Event::End(TagEnd::Table));
                    table.push(e);
                    if is_end {
                        break;
                    }
                }
                match pending.take() {
                    Some((TableMode::Reflow, bp)) => {
                        record_reflow_breakpoint(&bp);
                        emit_reflow_table(&mut out, aligns, table, &bp);
                    }
                    Some((TableMode::Transpose, bp)) => {
                        let ncols = aligns.len().max(1);
                        record_transpose_breakpoint(&bp, ncols);
                        emit_transpose_table(&mut out, aligns, table, &bp, ncols);
                    }
                    None => {
                        out.push(Event::Start(Tag::Table(aligns)));
                        out.extend(table);
                    }
                }
            }
            other => {
                // A directive applies only to a table that *immediately* follows it.
                // The directive comment is itself wrapped in `HtmlBlock` start/end
                // events, so those don't count as intervening content; anything else
                // (a paragraph, heading, another block...) means the directive was
                // misplaced — or its table was since removed — so drop it rather than
                // let it silently attach to an unrelated table further down the page.
                if !matches!(other, Event::Start(Tag::HtmlBlock) | Event::End(TagEnd::HtmlBlock)) {
                    pending = None;
                }
                out.push(other);
            }
        }
    }

    *events = out;
}

/// Emit one reflowed table: a `<div class="reflow reflow-bp-{token}">` wrapper and
/// the table with `scope="col"` headers and `data-label` body cells. The matching
/// container query is shipped once in the generated `reflow.css` (see `reflow_css`);
/// nothing is inlined here. `table` holds the events after `Start(Table)` up to and
/// including `End(Table)`.
fn emit_reflow_table<'a>(
    out: &mut Vec<Event<'a>>,
    aligns: Vec<Alignment>,
    table: Vec<Event<'a>>,
    bp: &str,
) {
    let ncols = aligns.len().max(1);
    let headers = collect_headers(&table, ncols);
    let token = reflow_class_token(bp);

    out.push(Event::Html(format!(r#"<div class="reflow reflow-bp-{token}">"#).into()));
    out.push(Event::Start(Tag::Table(aligns.clone())));

    // Re-emit, replacing cell boundaries with attribute-carrying raw HTML.
    let mut in_head = false;
    let mut col = 0usize;
    for ev in table {
        match ev {
            Event::Start(Tag::TableHead) => {
                in_head = true;
                col = 0;
                out.push(ev);
            }
            Event::End(TagEnd::TableHead) => {
                in_head = false;
                out.push(ev);
            }
            Event::Start(Tag::TableRow) => {
                col = 0;
                out.push(ev);
            }
            Event::Start(Tag::TableCell) => {
                let align = aligns.get(col).copied().unwrap_or(Alignment::None);
                let align_attr = reflow_align_attr(align);
                let tag = if in_head {
                    format!("<th{align_attr} scope=\"col\">")
                } else {
                    let mut label = String::new();
                    let header = headers.get(col).map(String::as_str).unwrap_or("");
                    escape_html(&mut label, header).expect("writing to a String cannot fail");
                    format!("<td{align_attr} data-label=\"{label}\">")
                };
                out.push(Event::Html(tag.into()));
            }
            Event::End(TagEnd::TableCell) => {
                out.push(Event::Html(if in_head { "</th>".into() } else { "</td>".into() }));
                col += 1;
            }
            other => out.push(other),
        }
    }
    out.push(Event::Html("</div>".into()));
}

/// Emit one transposed table: a
/// `<div class="transpose transpose-bp-{token} transpose-cols-{ncols}">` wrapper
/// around the *unchanged* table events. The flip is purely visual, done by the
/// generated CSS container query (see `transpose_css`); the DOM and table semantics
/// are untouched, so screen readers still read it column-wise. GFM tables are always
/// rectangular (no colspan/rowspan), so `ncols` rows is exact. `table` holds the
/// events after `Start(Table)` up to and including `End(Table)`.
fn emit_transpose_table<'a>(
    out: &mut Vec<Event<'a>>,
    aligns: Vec<Alignment>,
    table: Vec<Event<'a>>,
    bp: &str,
    ncols: usize,
) {
    let token = reflow_class_token(bp);
    out.push(Event::Html(
        format!(r#"<div class="transpose transpose-bp-{token} transpose-cols-{ncols}">"#).into(),
    ));
    out.push(Event::Start(Tag::Table(aligns)));
    out.extend(table); // header + body events, native alignment styles intact
    out.push(Event::Html("</div>".into()));
}

/// Map a validated breakpoint length to its CSS class / selector token. Lengths are
/// `<number><unit>`; the only character invalid in a class name is the decimal
/// point, which becomes `_` (so `37.5rem` -> `37_5rem`). Unambiguous because a
/// length never otherwise contains `_`.
fn reflow_class_token(bp: &str) -> String {
    bp.replace('.', "_")
}

/// Generate the shared `reflow.css` for the breakpoints used across the site: one
/// container query per breakpoint, with reflow-only rules keyed by the matching
/// `reflow-bp-<token>` class. Authors style the cards themselves via the stable
/// `.reflow` class. Mirrors how class-based highlighting ships `giallo.css`.
pub fn reflow_css(breakpoints: &BTreeSet<String>) -> String {
    let mut css = String::from(
        "/* Table reflow — generated by zola-plus from `<!-- reflow: <length> -->` annotations.\n   Reflow-only rules; style the cards yourself via the `.reflow` class. */\n\n",
    );
    for bp in breakpoints {
        let t = reflow_class_token(bp);
        css.push_str(&format!(".reflow-bp-{t} {{ container-type: inline-size; }}\n"));
        css.push_str(&format!("@container (max-width: {bp}) {{\n"));
        css.push_str(&format!(
            "  .reflow-bp-{t} thead {{ position: absolute; width: 1px; height: 1px; margin: -1px; padding: 0; border: 0; overflow: hidden; clip: rect(0 0 0 0); white-space: nowrap; }}\n"
        ));
        css.push_str(&format!("  .reflow-bp-{t} tr {{ display: block; }}\n"));
        css.push_str(&format!(
            "  .reflow-bp-{t} td {{ display: grid; grid-template-columns: auto 1fr; }}\n"
        ));
        css.push_str(&format!("  .reflow-bp-{t} td::before {{ content: attr(data-label); }}\n"));
        css.push_str("}\n\n");
    }
    css
}

/// Generate the transpose rules for the responsive-table stylesheet: one container
/// query per `(breakpoint, column-count)` used across the site. Below the
/// breakpoint the table becomes a CSS grid laid out column-first, with
/// `thead`/`tbody`/`tr` collapsed via `display: contents` so the cells flow into a
/// `repeat(<ncols>, auto)` grid — which transposes them (the header row becomes the
/// left column). The column count is keyed into the class so `repeat()` gets a
/// literal count, never an unreliable `var()`. Authors style via the stable
/// `.transpose` class.
pub fn transpose_css(breakpoints: &BTreeSet<(String, usize)>) -> String {
    let mut css = String::from(
        "/* Table transpose — generated by zola-plus from `<!-- transpose: <length> -->` annotations.\n   Visual transpose only; the DOM/table semantics are unchanged. Style via the `.transpose` class. */\n\n",
    );
    for (bp, ncols) in breakpoints {
        let t = reflow_class_token(bp);
        css.push_str(&format!(".transpose-bp-{t} {{ container-type: inline-size; }}\n"));
        css.push_str(&format!("@container (max-width: {bp}) {{\n"));
        css.push_str(&format!(
            "  .transpose-bp-{t}.transpose-cols-{ncols} table {{ display: grid; grid-auto-flow: column; grid-template-rows: repeat({ncols}, auto); }}\n"
        ));
        css.push_str(&format!(
            "  .transpose-bp-{t} thead,\n  .transpose-bp-{t} tbody,\n  .transpose-bp-{t} tr {{ display: contents; }}\n"
        ));
        css.push_str("}\n\n");
    }
    css
}

fn get_heading_refs(events: &[Event]) -> Vec<HeadingRef> {
    let mut heading_refs = vec![];

    for (i, event) in events.iter().enumerate() {
        match event {
            Event::Start(Tag::Heading { level, id, classes, .. }) => {
                heading_refs.push(HeadingRef::new(
                    i,
                    *level as u32,
                    id.clone().map(|a| a.to_string()),
                    &classes.iter().map(|x| x.to_string()).collect::<Vec<_>>(),
                ));
            }
            Event::End(TagEnd::Heading { .. }) => {
                heading_refs.last_mut().expect("Heading end before start?").end_idx = i;
            }
            _ => (),
        }
    }

    heading_refs
}

fn convert_footnotes_to_github_style(old_events: &mut Vec<Event>) {
    let events = std::mem::take(old_events);
    // step 1: We need to extract footnotes from the event stream and tweak footnote references

    // footnotes bodies are stored in a stack of vectors, because it is possible to have footnotes
    // inside footnotes
    let mut footnote_bodies_stack = Vec::new();
    let mut footnotes = Vec::new();
    // this will allow to create a multiple back references
    let mut footnote_numbers = HashMap::new();
    let filtered_events = events.into_iter().filter_map(|event| {
        match event {
            // New footnote definition is pushed to the stack
            Event::Start(Tag::FootnoteDefinition(_)) => {
                footnote_bodies_stack.push(vec![event]);
                None
            }
            // The topmost footnote definition is popped from the stack
            Event::End(TagEnd::FootnoteDefinition) => {
                // unwrap will never fail, because Tag::FootnoteDefinition always comes before
                // TagEnd::FootnoteDefinition
                let mut footnote_body = footnote_bodies_stack.pop().unwrap();
                footnote_body.push(event);
                footnotes.push(footnote_body);
                None
            }
            Event::FootnoteReference(name) => {
                // n will be a unique index of the footnote
                let n = footnote_numbers.len() + 1;
                // nr is a number of references to this footnote
                let (n, nr) = footnote_numbers.entry(name.clone()).or_insert((n, 0usize));
                *nr += 1;
                let reference = Event::Html(format!(r##"<sup class="footnote-reference" id="fr-{name}-{nr}"><a href="#fn-{name}">{n}</a></sup>"##).into());

                if footnote_bodies_stack.is_empty() {
                    // we are in the main text, just output the reference
                    Some(reference)
                } else {
                    // we are inside other footnote, we have to push that reference into that
                    // footnote
                    footnote_bodies_stack.last_mut().unwrap().push(reference);
                    None
                }
            }
            _ if !footnote_bodies_stack.is_empty() => {
                footnote_bodies_stack.last_mut().unwrap().push(event);
                None
            }
            _ => Some(event),
        }
    }
    );

    old_events.extend(filtered_events);

    if footnotes.is_empty() {
        return;
    }

    old_events
        .push(Event::Html("<section class=\"footnotes\">\n<ol class=\"footnotes-list\">\n".into()));

    // Step 2: retain only footnotes which was actually referenced
    footnotes.retain(|f| match f.first() {
        Some(Event::Start(Tag::FootnoteDefinition(name))) => {
            footnote_numbers.get(name).unwrap_or(&(0, 0)).1 != 0
        }
        _ => false,
    });

    // Step 3: Sort footnotes in the order of their appearance
    footnotes.sort_by_cached_key(|f| match f.first() {
        Some(Event::Start(Tag::FootnoteDefinition(name))) => {
            footnote_numbers.get(name).unwrap_or(&(0, 0)).0
        }
        _ => unreachable!(),
    });

    // Step 4: Add backreferences to footnotes
    let footnotes = footnotes.into_iter().flat_map(|fl| {
        // To write backrefs, the name needs kept until the end of the footnote definition.
        let mut name = CowStr::from("");
        // Backrefs are included in the final paragraph of the footnote, if it's normal text.
        // For example, this DOM can be produced:
        //
        // Markdown:
        //
        //     five [^feet].
        //
        //     [^feet]:
        //         A foot is defined, in this case, as 0.3048 m.
        //
        //         Historically, the foot has not been defined this way, corresponding to many
        //         subtly different units depending on the location.
        //
        // HTML:
        //
        //     <p>five <sup class="footnote-reference" id="fr-feet-1"><a href="#fn-feet">1</a></sup>.</p>
        //
        //     <ol class="footnotes-list">
        //     <li id="fn-feet">
        //     <p>A foot is defined, in this case, as 0.3048 m.</p>
        //     <p>Historically, the foot has not been defined this way, corresponding to many
        //     subtly different units depending on the location. <a href="#fr-feet-1">↩</a></p>
        //     </li>
        //     </ol>
        //
        // This is mostly a visual hack, so that footnotes use less vertical space.
        //
        // If there is no final paragraph, such as a tabular, list, or image footnote, it gets
        // pushed after the last tag instead.
        let mut has_written_backrefs = false;
        let fl_len = fl.len();
        let footnote_numbers = &footnote_numbers;
        fl.into_iter().enumerate().map(move |(i, f)| match f {
            Event::Start(Tag::FootnoteDefinition(current_name)) => {
                name = current_name;
                has_written_backrefs = false;
                Event::Html(format!(r##"<li id="fn-{name}">"##).into())
            }
            Event::End(TagEnd::FootnoteDefinition) | Event::End(TagEnd::Paragraph)
                if !has_written_backrefs && i >= fl_len - 2 =>
            {
                let usage_count = footnote_numbers.get(&name).unwrap().1;
                let mut end = String::with_capacity(
                    name.len() + (r##" <a href="#fr--1">↩</a></li>"##.len() * usage_count),
                );
                for usage in 1..=usage_count {
                    if usage == 1 {
                        write!(&mut end, r##" <a href="#fr-{name}-{usage}">↩</a>"##).unwrap();
                    } else {
                        write!(&mut end, r##" <a href="#fr-{name}-{usage}">↩{usage}</a>"##)
                            .unwrap();
                    }
                }
                has_written_backrefs = true;
                if f == Event::End(TagEnd::FootnoteDefinition) {
                    end.push_str("</li>\n");
                } else {
                    end.push_str("</p>\n");
                }
                Event::Html(end.into())
            }
            Event::End(TagEnd::FootnoteDefinition) => Event::Html("</li>\n".into()),
            Event::FootnoteReference(_) => unreachable!("converted to HTML earlier"),
            f => f,
        })
    });

    old_events.extend(footnotes);
    old_events.push(Event::Html("</ol>\n</section>\n".into()));
}

pub fn markdown_to_html(
    content: &str,
    context: &RenderContext,
    html_shortcodes: Vec<Shortcode>,
) -> Result<Rendered> {
    let path = context
        .tera_context
        .get("page")
        .or_else(|| context.tera_context.get("section"))
        .map(|x| x.as_object().unwrap().get("relative_path").unwrap().as_str().unwrap());
    // the rendered html
    let mut html = String::with_capacity(content.len());
    let mut summary = None;
    // Set while parsing
    let mut error = None;

    let mut code_block: Option<ParsedFence> = None;
    let mut code_block_content = String::new();

    // Indicates whether we're in the middle of parsing a text node which will be placed in an HTML
    // attribute, and which hence has to be escaped using escape_html rather than push_html's
    // default HTML body escaping for text nodes.
    let mut inside_attribute = false;

    let mut headings: Vec<Heading> = vec![];
    let mut internal_links = Vec::new();
    let mut external_links = Vec::new();

    let mut stop_next_end_p = false;

    let lazy_async_image = context.config.markdown.lazy_async_image;

    let mut opts = Options::empty();
    let mut has_summary = false;
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_FOOTNOTES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);
    opts.insert(Options::ENABLE_HEADING_ATTRIBUTES);

    if context.config.markdown.smart_punctuation {
        opts.insert(Options::ENABLE_SMART_PUNCTUATION);
    }
    if context.config.markdown.definition_list {
        opts.insert(Options::ENABLE_DEFINITION_LIST);
    }
    if context.config.markdown.github_alerts {
        opts.insert(Options::ENABLE_GFM);
    }

    // we reverse their order so we can pop them easily in order
    let mut html_shortcodes: Vec<_> = html_shortcodes.into_iter().rev().collect();
    let mut next_shortcode = html_shortcodes.pop();
    let contains_shortcode = |txt: &str| -> bool { txt.contains(SHORTCODE_PLACEHOLDER) };

    {
        let mut events = Vec::new();
        macro_rules! render_shortcodes {
            ($is_text:expr, $text:expr, $range:expr) => {
                let orig_range_start = $range.start;
                loop {
                    if let Some(ref shortcode) = next_shortcode {
                        if !$range.contains(&shortcode.span.start) {
                            break;
                        }
                        let sc_span = shortcode.span.clone();

                        // we have some text before the shortcode, push that first
                        if $range.start != sc_span.start {
                            let content: cmark::CowStr<'_> =
                                $text[($range.start - orig_range_start)
                                    ..(sc_span.start - orig_range_start)]
                                    .to_string()
                                    .into();
                            events.push(if $is_text {
                                if inside_attribute {
                                    let mut buffer = "".to_string();
                                    escape_html(&mut buffer, content.as_ref()).unwrap();
                                    Event::Html(buffer.into())
                                } else {
                                    Event::Text(content)
                                }
                            } else {
                                Event::Html(content)
                            });
                            $range.start = sc_span.start;
                        }

                        // Now we should be at the same idx as the shortcode
                        let shortcode = next_shortcode.take().unwrap();
                        match shortcode.render(&context.tera, &context.tera_context) {
                            Ok(s) => {
                                events.push(Event::Html(s.into()));
                                $range.start += SHORTCODE_PLACEHOLDER.len();
                            }
                            Err(e) => {
                                error = Some(e);
                                break;
                            }
                        }
                        next_shortcode = html_shortcodes.pop();
                        continue;
                    }

                    break;
                }

                if !$range.is_empty() {
                    // The $range value is for the whole document, not for this slice of text
                    let content = $text[($range.start - orig_range_start)..].to_string().into();
                    events.push(if $is_text { Event::Text(content) } else { Event::Html(content) });
                }
            };
        }

        for (event, mut range) in Parser::new_ext(content, opts).into_offset_iter() {
            match event {
                Event::Text(text) => {
                    if code_block.is_some() {
                        if contains_shortcode(text.as_ref()) {
                            // mark the start of the code block events
                            let stack_start = events.len();
                            render_shortcodes!(true, text, range);
                            // after rendering the shortcodes we will collect all the text events
                            // and re-render them as code blocks
                            for event in events[stack_start..].iter() {
                                match event {
                                    Event::Html(t) | Event::Text(t) => code_block_content += t,
                                    _ => {
                                        error = Some(Error::msg(format!(
                                            "Unexpected event while expanding the code block: {:?}",
                                            event
                                        )));
                                        break;
                                    }
                                }
                            }

                            // remove all the original events from shortcode rendering
                            events.truncate(stack_start);
                        } else {
                            code_block_content += &text;
                        }
                    } else {
                        let text = if context.config.markdown.render_emoji {
                            EMOJI_REPLACER.replace_all(&text).to_string().into()
                        } else {
                            text
                        };

                        if !contains_shortcode(text.as_ref()) {
                            if inside_attribute {
                                let mut buffer = "".to_string();
                                escape_html(&mut buffer, text.as_ref()).unwrap();
                                events.push(Event::Html(buffer.into()));
                            } else {
                                events.push(Event::Text(text));
                            }
                            continue;
                        }

                        render_shortcodes!(true, text, range);
                    }
                }
                Event::Start(Tag::CodeBlock(ref kind)) => {
                    let fence = match kind {
                        cmark::CodeBlockKind::Fenced(fence_info) => {
                            parse_markdown_fence(fence_info)
                        }
                        _ => ParsedFence::default(),
                    };
                    code_block = Some(fence);
                }
                Event::End(TagEnd::CodeBlock) => {
                    let html = if let Some(code) = code_block.take() {
                        if let Some(hl) = &context.config.markdown.highlighting {
                            if !hl.registry.contains_grammar(&code.lang) {
                                let location = if let Some(p) = path {
                                    format!(" in {p:?}")
                                } else {
                                    String::new()
                                };
                                let warning = format!(
                                    "Language `{}` not found for syntax highlighting{location}.`",
                                    code.lang
                                );
                                if hl.error_on_missing_language {
                                    bail!(warning);
                                } else {
                                    log::warn!("{warning}");
                                }
                            }
                            let highlighted = hl.registry.highlight(
                                &code_block_content,
                                &hl.highlight_options(&code.lang),
                            )?;
                            let renderer = HtmlRenderer {
                                other_metadata: code.rest,
                                css_class_prefix: if hl.uses_classes() {
                                    Some("z-".to_string())
                                } else {
                                    None
                                },
                            };
                            renderer.render(&highlighted, &code.options)
                        } else {
                            let lang = if code.lang != giallo::PLAIN_GRAMMAR_NAME {
                                format!(r#" data-lang="{}""#, code.lang)
                            } else {
                                String::new()
                            };
                            let mut escaped = String::new();
                            escape_html(&mut escaped, &code_block_content)?;
                            format!("<pre><code{lang}>{escaped}</code></pre>\n")
                        }
                    } else {
                        unreachable!(
                            "can we get into a TagEnd::CodeBlock without having seen TagStart?"
                        )
                    };
                    events.push(Event::Html(html.into()));
                    code_block = None;
                    code_block_content.clear();
                }
                Event::Start(Tag::Image { link_type, dest_url, title, id }) => {
                    let link = if is_colocated_asset_link(&dest_url) {
                        let link = format!("{}{}", context.current_page_permalink, &*dest_url);
                        link.into()
                    } else {
                        dest_url
                    };

                    events.push(if lazy_async_image {
                        let mut img_before_alt: String = "<img src=\"".to_string();
                        cmark_escape::escape_href(&mut img_before_alt, &link)
                            .expect("Could not write to buffer");
                        if !title.is_empty() {
                            img_before_alt
                                .write_str("\" title=\"")
                                .expect("Could not write to buffer");
                            cmark_escape::escape_href(&mut img_before_alt, &title)
                                .expect("Could not write to buffer");
                        }
                        img_before_alt.write_str("\" alt=\"").expect("Could not write to buffer");
                        inside_attribute = true;
                        Event::Html(img_before_alt.into())
                    } else {
                        inside_attribute = false;
                        Event::Start(Tag::Image { link_type, dest_url: link, title, id })
                    });
                }
                Event::End(TagEnd::Image) => events.push(if lazy_async_image {
                    Event::Html("\" loading=\"lazy\" decoding=\"async\" />".into())
                } else {
                    event
                }),
                Event::Start(Tag::Link { link_type, dest_url, title, id })
                    if dest_url.is_empty() =>
                {
                    error = Some(Error::msg("There is a link that is missing a URL"));
                    events.push(Event::Start(Tag::Link {
                        link_type,
                        dest_url: "#".into(),
                        title,
                        id,
                    }));
                }
                Event::Start(Tag::Link { link_type, dest_url, title, id }) => {
                    let fixed_link = match fix_link(
                        link_type,
                        &dest_url,
                        context,
                        &mut internal_links,
                        &mut external_links,
                    ) {
                        Ok(fixed_link) => fixed_link,
                        Err(err) => {
                            error = Some(err);
                            events.push(Event::Html("".into()));
                            continue;
                        }
                    };

                    events.push(
                        if is_external_link(&dest_url)
                            && context.config.markdown.has_external_link_tweaks()
                        {
                            let mut escaped = String::new();
                            // write_str can fail but here there are no reasons it should (afaik?)
                            cmark_escape::escape_href(&mut escaped, &dest_url)
                                .expect("Could not write to buffer");
                            Event::Html(
                                context
                                    .config
                                    .markdown
                                    .construct_external_link_tag(&escaped, &title)
                                    .into(),
                            )
                        } else {
                            Event::Start(Tag::Link {
                                link_type,
                                dest_url: fixed_link.into(),
                                title,
                                id,
                            })
                        },
                    )
                }
                Event::Start(Tag::Paragraph) => {
                    // We have to compare the start and the trimmed length because the content
                    // will sometimes contain '\n' at the end which we want to avoid.
                    //
                    // NOTE: It could be more efficient to remove this search and just keep
                    // track of the shortcodes to come and compare it to that.
                    if let Some(ref next_shortcode) = next_shortcode
                        && next_shortcode.span.start == range.start
                        && next_shortcode.span.len() == content[range].trim().len()
                    {
                        stop_next_end_p = true;
                        events.push(Event::Html("".into()));
                        continue;
                    }

                    events.push(event);
                }
                Event::End(TagEnd::Paragraph) => {
                    events.push(if stop_next_end_p {
                        stop_next_end_p = false;
                        Event::Html("".into())
                    } else {
                        event
                    });
                }
                Event::Html(text) | Event::InlineHtml(text)
                    if !has_summary && MORE_DIVIDER_RE.is_match(text.as_ref()) =>
                {
                    has_summary = true;
                    events.push(Event::Html(CONTINUE_READING.into()));
                }
                Event::Html(text) | Event::InlineHtml(text)
                    if contains_shortcode(text.as_ref()) =>
                {
                    render_shortcodes!(false, text, range);
                }
                _ => events.push(event),
            }
        }

        // We remove all the empty things we might have pushed before so we don't get some random \n
        events.retain(|e| match e {
            Event::Text(text) | Event::Html(text) => !text.is_empty(),
            _ => true,
        });

        let heading_refs = get_heading_refs(&events);

        let mut anchors_to_insert = vec![];
        let mut inserted_anchors = vec![];
        for heading in &heading_refs {
            if let Some(s) = &heading.id {
                inserted_anchors.push(s.to_owned());
            }
        }

        // Second heading pass: auto-generate remaining IDs, and emit HTML
        for mut heading_ref in heading_refs {
            let start_idx = heading_ref.start_idx;
            let end_idx = heading_ref.end_idx;
            let title = get_text(&events[start_idx + 1..end_idx]);

            if heading_ref.id.is_none() {
                heading_ref.id = Some(find_anchor(
                    &inserted_anchors,
                    slugify_anchors(&title, context.config.slugify.anchors),
                    0,
                ));
            }

            inserted_anchors.push(heading_ref.id.clone().unwrap());
            let id = inserted_anchors.last().unwrap();

            let html = heading_ref.to_html(id);
            events[start_idx] = Event::Html(html.into());

            // generate anchors and places to insert them
            if context.insert_anchor != InsertAnchor::None {
                let anchor_idx = match context.insert_anchor {
                    InsertAnchor::Left => start_idx + 1,
                    InsertAnchor::Right => end_idx,
                    InsertAnchor::Heading => 0, // modified later to the correct value
                    InsertAnchor::None => unreachable!(),
                };
                let mut c = tera::Context::new();
                c.insert("id", &id);
                c.insert("level", &heading_ref.level);
                c.insert("lang", &context.lang);

                let anchor_link = utils::templates::render_template(
                    ANCHOR_LINK_TEMPLATE,
                    &context.tera,
                    c,
                    &None,
                )
                .context("Failed to render anchor link template")?;
                if context.insert_anchor != InsertAnchor::Heading {
                    anchors_to_insert.push((anchor_idx, Event::Html(anchor_link.into())));
                } else if let Some(captures) = A_HTML_TAG.captures(&anchor_link) {
                    let opening_tag = captures.get(1).map_or("", |m| m.as_str()).to_string();
                    anchors_to_insert.push((start_idx + 1, Event::Html(opening_tag.into())));
                    anchors_to_insert.push((end_idx, Event::Html("</a>".into())));
                }
            }

            // record heading to make table of contents
            let permalink = format!("{}#{}", context.current_page_permalink, id);
            let h = Heading {
                level: heading_ref.level,
                id: id.to_owned(),
                permalink,
                title,
                children: Vec::new(),
            };
            headings.push(h);
        }

        if context.insert_anchor != InsertAnchor::None {
            insert_many(&mut events, anchors_to_insert);
        }

        if context.config.markdown.bottom_footnotes {
            convert_footnotes_to_github_style(&mut events);
        }

        // A no-op unless a table carries a `<!-- reflow: ... -->` directive.
        transform_tables(&mut events);

        let continue_reading = events
            .iter()
            .position(|e| matches!(e, Event::Html(CowStr::Borrowed(CONTINUE_READING))))
            .unwrap_or(events.len());

        // determine closing tags missing from summary
        let mut tags = Vec::new();
        for event in &events[..continue_reading] {
            match event {
                Event::Start(Tag::HtmlBlock) | Event::End(TagEnd::HtmlBlock) => (),
                Event::Start(tag) => tags.push(tag.to_end()),
                Event::End(tag) => {
                    tags.truncate(tags.iter().rposition(|t| *t == *tag).unwrap_or(0));
                }
                _ => (),
            }
        }

        if has_summary {
            // Note: we render the summary separately, restarting from
            // the beginning for the actual body of the page, as cmark's html
            // renderer has internal state, such as the footnote counter,
            // that it does not expose.
            let mut summary_html = String::new();
            cmark::html::push_html(
                &mut summary_html,
                events.iter().take(continue_reading).cloned(),
            );
            // remove footnotes
            let mut summary_html = FOOTNOTES_RE.replace_all(&summary_html, "").into_owned();

            // truncate trailing whitespace
            summary_html.truncate(summary_html.trim_end().len());

            // add cutoff template
            if !tags.is_empty() {
                let mut c = tera::Context::new();
                c.insert("summary", &summary_html);
                c.insert("lang", &context.lang);
                let summary_cutoff = utils::templates::render_template(
                    SUMMARY_CUTOFF_TEMPLATE,
                    &context.tera,
                    c,
                    &None,
                )
                .context("Failed to render summary cutoff template")?;
                summary_html.push_str(&summary_cutoff);
            }

            // close remaining tags
            cmark::html::push_html(&mut summary_html, tags.into_iter().rev().map(Event::End));

            summary = Some(summary_html);
        }

        // emit everything after summary
        cmark::html::push_html(&mut html, events.into_iter());
    }

    if let Some(e) = error {
        Err(e)
    } else {
        Ok(Rendered {
            summary,
            body: html,
            toc: make_table_of_contents(headings),
            internal_links,
            external_links,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use config::Config;
    use insta::assert_snapshot;
    use templates::ZOLA_TERA;

    #[test]
    fn insert_many_works() {
        let mut v = vec![1, 2, 3, 4, 5];
        insert_many(&mut v, vec![(0, 0), (2, -1), (5, 6)]);
        assert_eq!(v, &[0, 1, 2, -1, 3, 4, 5, 6]);

        let mut v2 = vec![1, 2, 3, 4, 5];
        insert_many(&mut v2, vec![(0, 0), (2, -1)]);
        assert_eq!(v2, &[0, 1, 2, -1, 3, 4, 5]);
    }

    #[test]
    fn test_is_external_link() {
        assert!(is_external_link("http://example.com/"));
        assert!(is_external_link("https://example.com/"));
        assert!(is_external_link("https://example.com/index.html#introduction"));

        assert!(!is_external_link("mailto:user@example.com"));
        assert!(!is_external_link("tel:18008675309"));

        assert!(!is_external_link("#introduction"));

        assert!(!is_external_link("http.jpg"))
    }

    #[test]
    // Tests for link  that points to files in the same directory
    fn test_is_colocated_asset_link_true() {
        let links: [&str; 3] = ["./same-dir.md", "file.md", "qwe.js"];
        for link in links {
            assert!(is_colocated_asset_link(link));
        }
    }

    #[test]
    // Tests for files where the link points to a different directory
    fn test_is_colocated_asset_link_false() {
        let links: [&str; 2] = ["/other-dir/file.md", "../sub-dir/file.md"];
        for link in links {
            assert!(!is_colocated_asset_link(link));
        }
    }

    #[test]
    // Tests for summary being split out
    fn test_summary_split() {
        let top = "Here's a compelling summary.";
        let top_rendered = format!("<p>{top}</p>");
        let bottom = "Here's the compelling conclusion.";
        let bottom_rendered = format!("<p>{bottom}</p>");
        // FIXME: would add a test that includes newlines, but due to the way pulldown-cmark parses HTML nodes, these are passed as separate HTML events. see: https://github.com/raphlinus/pulldown-cmark/issues/803
        let mores =
            ["<!-- more -->", "<!--more-->", "<!-- MORE -->", "<!--MORE-->", "<!--\t MoRe \t-->"];
        let config = Config::default();
        let mut context = RenderContext::from_config(&config);
        context.tera.to_mut().extend(&ZOLA_TERA).unwrap();
        for more in mores {
            let content = format!("{top}\n\n{more}\n\n{bottom}");
            let rendered = markdown_to_html(&content, &context, vec![]).unwrap();
            assert!(rendered.summary.is_some(), "no summary when splitting on {more}");
            let summary = rendered.summary.unwrap();
            let summary = summary.trim();
            let body = rendered.body[summary.len()..].trim();
            let continue_reading = &body[..CONTINUE_READING.len()];
            let body = &body[CONTINUE_READING.len()..].trim();
            assert_eq!(summary, &top_rendered);
            assert_eq!(continue_reading, CONTINUE_READING);
            assert_eq!(body, &bottom_rendered);
        }
    }

    #[test]
    fn no_footnotes() {
        let mut opts = Options::empty();
        opts.insert(Options::ENABLE_TABLES);
        opts.insert(Options::ENABLE_FOOTNOTES);
        opts.insert(Options::ENABLE_STRIKETHROUGH);
        opts.insert(Options::ENABLE_TASKLISTS);
        opts.insert(Options::ENABLE_HEADING_ATTRIBUTES);

        let content = "Some text *without* footnotes.\n\nOnly ~~fancy~~ formatting.";
        let mut events: Vec<_> = Parser::new_ext(&content, opts).collect();
        convert_footnotes_to_github_style(&mut events);
        let mut html = String::new();
        cmark::html::push_html(&mut html, events.into_iter());
        assert_snapshot!(html);
    }

    #[test]
    fn single_footnote() {
        let mut opts = Options::empty();
        opts.insert(Options::ENABLE_TABLES);
        opts.insert(Options::ENABLE_FOOTNOTES);
        opts.insert(Options::ENABLE_STRIKETHROUGH);
        opts.insert(Options::ENABLE_TASKLISTS);
        opts.insert(Options::ENABLE_HEADING_ATTRIBUTES);

        let content = "This text has a footnote[^1]\n [^1]:But it is meaningless.";
        let mut events: Vec<_> = Parser::new_ext(&content, opts).collect();
        convert_footnotes_to_github_style(&mut events);
        let mut html = String::new();
        cmark::html::push_html(&mut html, events.into_iter());
        assert_snapshot!(html);
    }

    #[test]
    fn reordered_footnotes() {
        let mut opts = Options::empty();
        opts.insert(Options::ENABLE_TABLES);
        opts.insert(Options::ENABLE_FOOTNOTES);
        opts.insert(Options::ENABLE_STRIKETHROUGH);
        opts.insert(Options::ENABLE_TASKLISTS);
        opts.insert(Options::ENABLE_HEADING_ATTRIBUTES);

        let content = "This text has two[^2] footnotes[^1]\n[^1]: not sorted.\n[^2]: But they are";
        let mut events: Vec<_> = Parser::new_ext(&content, opts).collect();
        convert_footnotes_to_github_style(&mut events);
        let mut html = String::new();
        cmark::html::push_html(&mut html, events.into_iter());
        assert_snapshot!(html);
    }

    #[test]
    fn def_before_use() {
        let mut opts = Options::empty();
        opts.insert(Options::ENABLE_TABLES);
        opts.insert(Options::ENABLE_FOOTNOTES);
        opts.insert(Options::ENABLE_STRIKETHROUGH);
        opts.insert(Options::ENABLE_TASKLISTS);
        opts.insert(Options::ENABLE_HEADING_ATTRIBUTES);

        let content = "[^1]:It's before the reference.\n\n There is footnote definition?[^1]";
        let mut events: Vec<_> = Parser::new_ext(&content, opts).collect();
        convert_footnotes_to_github_style(&mut events);
        let mut html = String::new();
        cmark::html::push_html(&mut html, events.into_iter());
        assert_snapshot!(html);
    }

    #[test]
    fn multiple_refs() {
        let mut opts = Options::empty();
        opts.insert(Options::ENABLE_TABLES);
        opts.insert(Options::ENABLE_FOOTNOTES);
        opts.insert(Options::ENABLE_STRIKETHROUGH);
        opts.insert(Options::ENABLE_TASKLISTS);
        opts.insert(Options::ENABLE_HEADING_ATTRIBUTES);

        let content = "This text has two[^1] identical footnotes[^1]\n[^1]: So one is present.\n[^2]: But another in not.";
        let mut events: Vec<_> = Parser::new_ext(&content, opts).collect();
        convert_footnotes_to_github_style(&mut events);
        let mut html = String::new();
        cmark::html::push_html(&mut html, events.into_iter());
        assert_snapshot!(html);
    }

    #[test]
    fn footnote_inside_footnote() {
        let mut opts = Options::empty();
        opts.insert(Options::ENABLE_TABLES);
        opts.insert(Options::ENABLE_FOOTNOTES);
        opts.insert(Options::ENABLE_STRIKETHROUGH);
        opts.insert(Options::ENABLE_TASKLISTS);
        opts.insert(Options::ENABLE_HEADING_ATTRIBUTES);

        let content = "This text has a footnote[^1]\n[^1]: But the footnote has another footnote[^2].\n[^2]: That's it.";
        let mut events: Vec<_> = Parser::new_ext(&content, opts).collect();
        convert_footnotes_to_github_style(&mut events);
        let mut html = String::new();
        cmark::html::push_html(&mut html, events.into_iter());
        assert_snapshot!(html);
    }

    fn render_reflow(content: &str) -> String {
        let mut opts = Options::empty();
        opts.insert(Options::ENABLE_TABLES);
        opts.insert(Options::ENABLE_STRIKETHROUGH);
        opts.insert(Options::ENABLE_TASKLISTS);
        opts.insert(Options::ENABLE_HEADING_ATTRIBUTES);
        let mut events: Vec<_> = Parser::new_ext(content, opts).collect();
        transform_tables(&mut events);
        let mut html = String::new();
        cmark::html::push_html(&mut html, events.into_iter());
        html
    }

    #[test]
    fn reflow_basic() {
        // Annotated table: scoped <style> at the chosen breakpoint, the wrapper, and
        // a data-label of its column header on every body cell.
        let content =
            "<!-- reflow: 40rem -->\n\n| Name | Department |\n| ---- | ---------- |\n| Ada | Platform |\n| Linus | Kernel |";
        assert_snapshot!(render_reflow(content));
    }

    #[test]
    fn reflow_alignment() {
        // Column alignment is replicated onto the raw <th>/<td>.
        let content = "<!-- reflow: 30rem -->\n\n| L | C | R |\n|:--|:-:|--:|\n| a | b | c |";
        assert_snapshot!(render_reflow(content));
    }

    #[test]
    fn reflow_inline_markup() {
        // Cells keep inline markup; data-label uses the header's plain text only.
        let content =
            "<!-- reflow: 30rem -->\n\n| **Bold head** | `code` |\n| --- | --- |\n| _em_ | [x](https://example.com) |";
        assert_snapshot!(render_reflow(content));
    }

    #[test]
    fn reflow_unannotated_table_is_plain() {
        // No directive => the table is left completely untouched.
        let content = "| A | B |\n| - | - |\n| 1 | 2 |";
        let html = render_reflow(content);
        assert!(
            !html.contains("class=\"reflow") && !html.contains("data-label") && !html.contains("<style"),
            "unannotated table must stay a plain table, got:\n{html}"
        );
    }

    #[test]
    fn reflow_no_table_is_noop() {
        let content = "Just a paragraph, no table here.";
        assert_snapshot!(render_reflow(content));
    }

    #[test]
    fn reflow_directive_does_not_leak_past_content() {
        // A directive applies only to the table that immediately follows it. With
        // unrelated content in between, the directive is dropped, so the later table
        // stays plain rather than picking up a breakpoint meant for nothing.
        let content =
            "<!-- reflow: 40rem -->\n\nA paragraph in between.\n\n| A | B |\n| - | - |\n| 1 | 2 |";
        let html = render_reflow(content);
        assert!(
            !html.contains("class=\"reflow"),
            "table after intervening content must stay plain, got:\n{html}"
        );
    }

    #[test]
    fn reflow_invalid_length_is_plain() {
        // A non-length value is rejected: the table renders plain rather than
        // emitting a broken container query.
        let content = "<!-- reflow: huge -->\n\n| A | B |\n| - | - |\n| 1 | 2 |";
        let html = render_reflow(content);
        assert!(
            !html.contains("class=\"reflow") && !html.contains("<style"),
            "invalid breakpoint must leave the table plain, got:\n{html}"
        );
    }

    #[test]
    fn reflow_escapes_header_in_data_label() {
        // Header text flows into a double-quoted `data-label`; quotes, ampersands and
        // angle brackets must be escaped or a header could break out of the attribute.
        let content = "<!-- reflow: 30rem -->\n\n| Tom & \"Jerry\" |\n| --- |\n| x |";
        let html = render_reflow(content);
        assert!(
            html.contains(r#"data-label="Tom &amp; &quot;Jerry&quot;""#),
            "header must be attribute-escaped in data-label, got:\n{html}"
        );
    }

    #[test]
    fn reflow_class_encodes_breakpoint_value() {
        // The breakpoint becomes the wrapper class; a decimal point is sanitized to
        // `_` so the class stays valid (`37.5rem` -> `reflow-bp-37_5rem`). No CSS is
        // inlined — it ships in the generated reflow.css.
        let content = "<!-- reflow: 37.5rem -->\n\n| A | B |\n| - | - |\n| 1 | 2 |";
        let html = render_reflow(content);
        assert!(
            html.contains(r#"<div class="reflow reflow-bp-37_5rem">"#),
            "breakpoint must drive the wrapper class, got:\n{html}"
        );
        assert!(!html.contains("<style"), "no CSS should be inlined, got:\n{html}");
    }

    #[test]
    fn reflow_css_emits_one_container_query_per_breakpoint() {
        let mut set = std::collections::BTreeSet::new();
        set.insert("40rem".to_string());
        set.insert("37.5rem".to_string());
        let css = reflow_css(&set);
        // The selector token sanitizes the dot; the query keeps the real length.
        assert!(css.contains(".reflow-bp-40rem { container-type: inline-size; }"), "got:\n{css}");
        assert!(css.contains("@container (max-width: 40rem) {"), "got:\n{css}");
        assert!(
            css.contains(".reflow-bp-37_5rem td::before { content: attr(data-label); }"),
            "got:\n{css}"
        );
        assert!(css.contains("@container (max-width: 37.5rem) {"), "got:\n{css}");
    }

    fn render_transpose(content: &str) -> String {
        // Same pipeline as `render_reflow`; `transform_tables` handles both modes.
        let mut opts = Options::empty();
        opts.insert(Options::ENABLE_TABLES);
        opts.insert(Options::ENABLE_STRIKETHROUGH);
        opts.insert(Options::ENABLE_TASKLISTS);
        opts.insert(Options::ENABLE_HEADING_ATTRIBUTES);
        let mut events: Vec<_> = Parser::new_ext(content, opts).collect();
        transform_tables(&mut events);
        let mut html = String::new();
        cmark::html::push_html(&mut html, events.into_iter());
        html
    }

    #[test]
    fn transpose_basic() {
        // Annotated table: the wrapper carries the breakpoint and column-count
        // classes; the table itself is re-emitted unchanged (no per-cell rewriting).
        let content =
            "<!-- transpose: 40rem -->\n\n| Name | Department |\n| ---- | ---------- |\n| Ada | Platform |\n| Linus | Kernel |";
        assert_snapshot!(render_transpose(content));
    }

    #[test]
    fn transpose_alignment() {
        // Column alignment is left to cmark's native cell rendering — transpose does
        // not touch cells — so the alignment styles pass straight through.
        let content = "<!-- transpose: 30rem -->\n\n| L | C | R |\n|:--|:-:|--:|\n| a | b | c |";
        assert_snapshot!(render_transpose(content));
    }

    #[test]
    fn transpose_inline_markup() {
        // Cells keep their inline markup; transpose adds no data-label.
        let content =
            "<!-- transpose: 30rem -->\n\n| **Bold head** | `code` |\n| --- | --- |\n| _em_ | [x](https://example.com) |";
        assert_snapshot!(render_transpose(content));
    }

    #[test]
    fn transpose_unannotated_table_is_plain() {
        // No directive => the table is left completely untouched.
        let content = "| A | B |\n| - | - |\n| 1 | 2 |";
        let html = render_transpose(content);
        assert!(
            !html.contains("class=\"transpose") && !html.contains("<style"),
            "unannotated table must stay a plain table, got:\n{html}"
        );
    }

    #[test]
    fn transpose_no_table_is_noop() {
        let content = "Just a paragraph, no table here.";
        assert_snapshot!(render_transpose(content));
    }

    #[test]
    fn transpose_directive_does_not_leak_past_content() {
        // A directive applies only to the table that immediately follows it.
        let content =
            "<!-- transpose: 40rem -->\n\nA paragraph in between.\n\n| A | B |\n| - | - |\n| 1 | 2 |";
        let html = render_transpose(content);
        assert!(
            !html.contains("class=\"transpose"),
            "table after intervening content must stay plain, got:\n{html}"
        );
    }

    #[test]
    fn transpose_invalid_length_is_plain() {
        // A non-length value is rejected: the table renders plain.
        let content = "<!-- transpose: huge -->\n\n| A | B |\n| - | - |\n| 1 | 2 |";
        let html = render_transpose(content);
        assert!(
            !html.contains("class=\"transpose") && !html.contains("<style"),
            "invalid breakpoint must leave the table plain, got:\n{html}"
        );
    }

    #[test]
    fn transpose_class_encodes_breakpoint_and_column_count() {
        // The breakpoint and column count both become wrapper classes; a decimal
        // point is sanitized to `_` (`37.5rem` -> `transpose-bp-37_5rem`). No CSS or
        // custom property is inlined — the column count drives a literal `repeat()`
        // in the generated stylesheet instead.
        let content = "<!-- transpose: 37.5rem -->\n\n| A | B |\n| - | - |\n| 1 | 2 |";
        let html = render_transpose(content);
        assert!(
            html.contains(r#"<div class="transpose transpose-bp-37_5rem transpose-cols-2">"#),
            "breakpoint and column count must drive the wrapper classes, got:\n{html}"
        );
        assert!(
            !html.contains("<style") && !html.contains("--transpose"),
            "no CSS or custom property should be inlined, got:\n{html}"
        );
    }

    #[test]
    fn transpose_single_column_table() {
        // A one-column table records `transpose-cols-1`.
        let content = "<!-- transpose: 30rem -->\n\n| Only |\n| ---- |\n| a |";
        let html = render_transpose(content);
        assert!(
            html.contains(r#"transpose-cols-1">"#),
            "single-column table must record cols-1, got:\n{html}"
        );
    }

    #[test]
    fn transpose_and_reflow_are_mutually_exclusive() {
        // When both directives precede one table, the last one wins (here transpose);
        // the table is never wrapped as both.
        let content =
            "<!-- reflow: 40rem -->\n\n<!-- transpose: 30rem -->\n\n| A | B |\n| - | - |\n| 1 | 2 |";
        let html = render_transpose(content);
        assert!(
            html.contains("class=\"transpose") && !html.contains("class=\"reflow"),
            "the last directive must win, got:\n{html}"
        );
    }

    #[test]
    fn transpose_css_emits_one_container_query_per_spec() {
        let mut set = std::collections::BTreeSet::new();
        set.insert(("40rem".to_string(), 2usize));
        set.insert(("30rem".to_string(), 3usize));
        let css = transpose_css(&set);
        assert!(css.contains(".transpose-bp-40rem { container-type: inline-size; }"), "got:\n{css}");
        assert!(css.contains("@container (max-width: 40rem) {"), "got:\n{css}");
        // The column count is a literal in `repeat()`, never a `var()`.
        assert!(
            css.contains("grid-template-rows: repeat(2, auto);")
                && css.contains("grid-template-rows: repeat(3, auto);"),
            "got:\n{css}"
        );
        assert!(!css.contains("var("), "repeat() must use a literal count, got:\n{css}");
        assert!(css.contains(".transpose-bp-30rem tr { display: contents; }"), "got:\n{css}");
    }
}
