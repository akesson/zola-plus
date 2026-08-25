use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use unicode_segmentation::UnicodeSegmentation;
use utils::templates::ShortcodeInvocationCounter;

use crate::shortcode::{SHORTCODE_PLACEHOLDER, Shortcode, parse_for_shortcodes};

/// Counts the words a reader actually reads in some markdown source.
///
/// Counted: prose, headings, list items, table cells, inline code and the bodies of
/// `{% sc() %}...{% end %}` shortcodes.
///
/// Skipped: code blocks (fenced with any marker, or indented), raw HTML (tags, attributes,
/// comments, `<script>`/`<style>`, inline `<svg>`), link and image destinations, image alt
/// text, footnote markers and the shortcode invocations themselves.
///
/// Whatever a shortcode renders to is never seen here, since the count happens at parse time.
pub fn count_words(content: &str) -> usize {
    // avoid parsing the content if not needed, mirrors `render_content`
    if !content.contains("{{") && !content.contains("{%") {
        return count_markdown_words(content);
    }

    match parse_for_shortcodes(content, &mut ShortcodeInvocationCounter::new()) {
        Ok((stripped, shortcodes)) => {
            count_markdown_words(&stripped)
                + shortcodes.iter().map(count_shortcode_body_words).sum::<usize>()
        }
        // Malformed shortcode syntax: rendering reports the error later, count the source as-is
        Err(_) => count_markdown_words(content),
    }
}

fn count_shortcode_body_words(sc: &Shortcode) -> usize {
    sc.body.as_deref().map_or(0, count_markdown_words)
        + sc.inner.iter().map(count_shortcode_body_words).sum::<usize>()
}

fn count_markdown_words(content: &str) -> usize {
    // Placeholders left behind by the shortcode parser would otherwise count as a word each
    let content = content.replace(SHORTCODE_PLACEHOLDER, " ");

    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_FOOTNOTES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);
    opts.insert(Options::ENABLE_HEADING_ATTRIBUTES);
    opts.insert(Options::ENABLE_GFM);

    // > 0 while inside a code block or an image (alt text)
    let mut skip_depth = 0usize;
    let mut count = 0;
    for event in Parser::new_ext(&content, opts) {
        match event {
            Event::Start(Tag::CodeBlock(_)) | Event::Start(Tag::Image { .. }) => skip_depth += 1,
            Event::End(TagEnd::CodeBlock) | Event::End(TagEnd::Image) => skip_depth -= 1,
            Event::Text(text) | Event::Code(text) if skip_depth == 0 => {
                count += text.unicode_words().count();
            }
            _ => (),
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::count_words;

    #[test]
    fn counts_plain_prose_and_markdown_syntax() {
        assert_eq!(count_words(""), 0);
        assert_eq!(count_words("  "), 0);
        assert_eq!(count_words("Hello World"), 2);
        assert_eq!(
            count_words("# Title {#my-id .cls}\n\n* item\n> quote\n\n---\n\n**bold** _it_"),
            5
        );
        assert_eq!(count_words("| a | b |\n|---|---|\n| 1 | 2 |"), 4);
        assert_eq!(count_words("use `foo_bar()` here"), 3);
    }

    #[test]
    fn skips_code_blocks_of_every_kind() {
        assert_eq!(count_words("before\n```\nlet x = 1;\n```\nafter"), 2);
        assert_eq!(count_words("before\n```rust\nlet x = 1;\n```\nafter"), 2);
        assert_eq!(count_words("before\n~~~\nlet x = 1;\n~~~\nafter"), 2);
        assert_eq!(count_words("before\n\n    let x = 1;\n\nafter"), 2);
        assert_eq!(count_words("before\n````\nlet x = 1;\n````\nafter"), 2);
        // a fence wrapping a fence
        assert_eq!(count_words("before\n````md\n```rust\nlet x = 1;\n```\n````\nafter"), 2);
        // an unterminated fence swallows the rest, as it does when rendering
        assert_eq!(count_words("words here\n```\ncode forever"), 2);
    }

    #[test]
    fn inline_backticks_in_prose_do_not_swallow_the_page() {
        assert_eq!(count_words("type ``` to start, then words continue"), 6);
        assert_eq!(count_words("hello world ``` code goes here ``` goodbye world"), 7);
    }

    #[test]
    fn skips_raw_html() {
        assert_eq!(count_words("text <!-- this is hidden --> end"), 2);
        assert_eq!(count_words("text\n\n<!-- this is hidden -->\n\nend"), 2);
        assert_eq!(count_words("<div class=\"note\">hi</div>"), 0);
        assert_eq!(count_words("a <span class=\"note\">hi</span> b"), 3);
        assert_eq!(
            count_words(
                "text\n\n<svg viewBox=\"0 0 24 24\"><path d=\"M12 2L2 7l10 5 10-5-10-5z\" fill=\"none\"/></svg>\n\nend"
            ),
            2
        );
        assert_eq!(
            count_words(
                "text <svg viewBox=\"0 0 24 24\"><path d=\"M12 2L2 7l10 5 10-5-10-5z\" fill=\"none\"/></svg> end"
            ),
            2
        );
        assert_eq!(count_words("<script>\nconsole.log('x')\n</script>\n\nafter"), 1);
        assert_eq!(count_words("<style>\n.a { color: red }\n</style>\n\nafter"), 1);
    }

    #[test]
    fn skips_urls_alt_text_and_footnote_markers() {
        assert_eq!(count_words("see [the docs](https://example.com/some/path.html)"), 3);
        assert_eq!(
            count_words("see [the docs][ref]\n\n[ref]: https://example.com/some/path.html"),
            3
        );
        assert_eq!(count_words("![alt text](images/my-photo.png)"), 0);
        assert_eq!(count_words("text[^1]\n\n[^1]: the note"), 3);
    }

    #[test]
    fn shortcodes_count_body_but_not_invocation() {
        assert_eq!(count_words("{{ youtube(id=\"dQw4w9WgXcQ\") }}"), 0);
        assert_eq!(count_words("see {{ youtube(id=\"dQw4w9WgXcQ\") }} now"), 2);
        assert_eq!(count_words("{% note(kind=\"warn\") %}\nBe very careful\n{% end %}"), 3);
        assert_eq!(
            count_words(
                "intro\n\n{% outer() %}\nouter body {{ inner(a=1) }} more\n{% end %}\n\nend"
            ),
            5
        );
        // body shortcodes nested in a body shortcode
        assert_eq!(
            count_words("{% outer() %}\none {% inner() %}\ntwo three\n{% end %}\n{% end %}"),
            3
        );
        // escaped shortcodes are rendered literally, so they are read literally
        assert_eq!(count_words("{{/* youtube(id=\"x\") */}}"), 3);
        // malformed shortcode: fall back to counting the source
        assert_eq!(count_words("hello {{ broken( world"), 3);
    }

    #[test]
    fn counts_cjk_per_segment() {
        // unicode-segmentation yields one word per Han character and one per kana run
        assert_eq!(count_words("日本語のテキスト"), 5);
    }
}
