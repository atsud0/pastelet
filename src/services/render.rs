use std::{borrow::Cow, collections::HashSet, sync::LazyLock};

use ammonia::Builder;
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd, html};
use syntect::{
    highlighting::ThemeSet,
    html::{ClassStyle, ClassedHTMLGenerator, css_for_theme_with_class_style},
    util::LinesWithEndings,
};

use super::language::{self, SYNTAXES};
use crate::error::AppError;

const CLASS_STYLE: ClassStyle = ClassStyle::SpacedPrefixed { prefix: "syn-" };
static THEMES: LazyLock<ThemeSet> = LazyLock::new(ThemeSet::load_defaults);

fn escape_html(input: &str) -> String {
    let mut escaped = String::with_capacity(input.len());
    for character in input.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            _ => escaped.push(character),
        }
    }
    escaped
}

pub fn highlight(content: &str, language: &str) -> Result<String, AppError> {
    if content.lines().any(|line| line.len() > 8192) {
        return Ok(escape_html(content));
    }
    let syntax = language::syntax(language).unwrap_or_else(|| SYNTAXES.find_syntax_plain_text());
    let mut generator = ClassedHTMLGenerator::new_with_class_style(syntax, &SYNTAXES, CLASS_STYLE);
    for line in LinesWithEndings::from(content) {
        generator.parse_html_for_line_which_includes_newline(line)?;
    }
    Ok(generator.finalize())
}

pub fn source(content: &str, language: &str) -> Result<String, AppError> {
    let content = content.replace("\r\n", "\n").replace('\r', "\n");
    let line_count = content.split_terminator('\n').count().max(1);
    let highlighted = highlight(&content, language)?;
    let wrap_class = if language::syntax(language).is_some_and(|syntax| syntax.name == "Plain Text")
    {
        " is-wrapped"
    } else {
        ""
    };
    let mut output = format!("<div class=\"source-view{wrap_class}\"><div class=\"source-lines\">");
    let mut scopes: Vec<String> = Vec::new();
    for (index, line) in highlighted.split('\n').take(line_count).enumerate() {
        let number = index + 1;
        output.push_str(&format!(
            "<a id=\"L{number}\" href=\"#L{number}\" class=\"line-numbers\" aria-label=\"Line {number}\">{number}</a><pre class=\"source-code syn-code\"><code>"
        ));
        for scope in &scopes {
            output.push_str(scope);
        }
        output.push_str(line);
        for tag in line.split('<').skip(1) {
            if tag.starts_with("/span>") {
                scopes.pop();
            } else if tag.starts_with("span ") {
                let end = tag
                    .find('>')
                    .ok_or(AppError::Internal("invalid highlighted span"))?;
                scopes.push(format!("<{}", &tag[..=end]));
            }
        }
        for _ in &scopes {
            output.push_str("</span>");
        }
        output.push_str("</code></pre>");
    }
    output.push_str("</div></div>");
    Ok(output)
}

pub fn markdown(content: &str) -> Result<String, AppError> {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_GFM;
    let parser = Parser::new_ext(content, options);
    let mut events = Vec::new();
    let mut code_block: Option<(String, String)> = None;
    for event in parser {
        match event {
            Event::Start(Tag::CodeBlock(kind)) => {
                let language = match kind {
                    CodeBlockKind::Fenced(info) => {
                        info.split_whitespace().next().unwrap_or("text").to_owned()
                    }
                    CodeBlockKind::Indented => "text".into(),
                };
                code_block = Some((language, String::new()));
            }
            Event::Text(text) if code_block.is_some() => {
                if let Some((_, code)) = &mut code_block {
                    code.push_str(&text);
                }
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some((language, code)) = code_block.take() {
                    let highlighted = highlight(&code, &language)?;
                    events.push(Event::Html(
                        format!("<pre class=\"syn-code\"><code>{highlighted}</code></pre>").into(),
                    ));
                }
            }
            event => events.push(event),
        }
    }
    let mut output = String::new();
    html::push_html(&mut output, events.into_iter());
    Ok(sanitize(&output))
}

pub fn sanitize(html: &str) -> String {
    Builder::default()
        .add_tags(["input"])
        .add_tag_attributes("input", ["checked"])
        .set_tag_attribute_value("input", "type", "checkbox")
        .set_tag_attribute_value("input", "disabled", "")
        .add_generic_attributes(["id"])
        .add_tag_attributes("span", ["class"])
        .add_tag_attributes("pre", ["class"])
        .add_tag_attributes("sup", ["class"])
        .add_tag_attributes("div", ["class"])
        .id_prefix(Some("md-"))
        .url_schemes(HashSet::from(["http", "https", "mailto"]))
        .link_rel(Some("noopener noreferrer nofollow"))
        .attribute_filter(|_, attribute, value| {
            if attribute == "class" {
                let classes = value
                    .split_whitespace()
                    .filter(|class| {
                        class.starts_with("syn-")
                            || matches!(
                                *class,
                                "footnote-definition"
                                    | "footnote-definition-label"
                                    | "footnote-reference"
                            )
                    })
                    .collect::<Vec<_>>()
                    .join(" ");
                return (!classes.is_empty()).then_some(Cow::Owned(classes));
            }
            if attribute == "href"
                && let Some(fragment) = value.strip_prefix('#')
            {
                return Some(Cow::Owned(format!("#md-{fragment}")));
            }
            Some(Cow::Borrowed(value))
        })
        .clean(html)
        .to_string()
}

pub fn syntax_css() -> Result<String, AppError> {
    let light = THEMES
        .themes
        .get("InspiredGitHub")
        .ok_or(AppError::Internal("missing light theme"))?;
    let dark = THEMES
        .themes
        .get("base16-ocean.dark")
        .ok_or(AppError::Internal("missing dark theme"))?;
    let light_css = css_for_theme_with_class_style(light, CLASS_STYLE)?;
    let dark_css = css_for_theme_with_class_style(dark, CLASS_STYLE)?;
    Ok(format!(
        "{light_css}\n:root[data-theme=dark] {{ {dark_css} }}\n@media (prefers-color-scheme: dark) {{ :root:not([data-theme=light]) {{ {dark_css} }} }}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_is_escaped_and_has_numbered_anchors() {
        let rendered = source("<script>alert('x')</script>\r\nline two\n", "text").expect("source");
        assert!(!rendered.contains("<script>"));
        assert!(rendered.contains("&lt;script&gt;"));
        assert!(rendered.contains("id=\"L1\""));
        assert!(rendered.contains("id=\"L2\""));
        assert!(!rendered.contains("id=\"L3\""));
        assert!(
            source("\nhello", "text")
                .expect("source")
                .contains("id=\"L2\"")
        );
    }

    #[test]
    fn plain_text_wraps_with_numbers_and_safe_line_anchors() {
        let rendered = source("first\n\n<script>third</script>\n", "text").expect("text");
        assert!(rendered.contains("source-view is-wrapped"));
        assert_eq!(rendered.matches("class=\"line-numbers\"").count(), 3);
        assert!(rendered.contains("id=\"L2\" href=\"#L2\""));
        assert!(rendered.contains("&lt;script&gt;third&lt;/script&gt;"));
        assert!(!rendered.contains("<script>"));
        let code = source("let value = 1;", "Rust").expect("Rust");
        assert!(!code.contains("is-wrapped"));
        assert!(code.contains("line-numbers"));
    }

    #[test]
    fn reader_rows_keep_multiline_syntax_scopes_balanced() {
        for (content, language) in [
            (
                "/* comment\ncontinued <script>\n*/\nlet value = 1;\n",
                "Rust",
            ),
            ("value = \"\"\"first\nsecond\n\"\"\"\n", "Python"),
            ("\n\nlast", "text"),
            ("", "text"),
        ] {
            let rendered = source(content, language).expect("rendered");
            let rows: Vec<_> = rendered
                .split("<code>")
                .skip(1)
                .map(|part| part.split("</code>").next().expect("row"))
                .collect();
            assert_eq!(rows.len(), content.split_terminator('\n').count().max(1));
            for row in &rows {
                let mut depth = 0;
                for tag in row.split('<').skip(1) {
                    if tag.starts_with("span ") {
                        depth += 1;
                    } else if tag.starts_with("/span>") {
                        depth -= 1;
                    }
                    assert!(depth >= 0, "unbalanced row: {row}");
                }
                assert_eq!(depth, 0, "unclosed row: {row}");
            }
            if language == "Rust" {
                assert!(rows[1].contains("syn-comment"));
                assert!(rows[1].contains("&lt;script&gt;"));
                assert!(rows[3].contains("syn-storage"));
            }
        }
    }

    #[test]
    fn gfm_and_highlighted_fences_survive_sanitization() {
        let rendered = markdown("# Title\n\n| A | B |\n| - | - |\n| 1 | 2 |\n\n- [x] done\n- [ ] todo\n\n~~gone~~\n\n> quote\n\ntext[^note]\n\n[^note]: footnote\n\n```rust\nfn main() {}\n```\n").expect("markdown");
        for expected in [
            "<h1>",
            "<table>",
            "<blockquote>",
            "<del>",
            "type=\"checkbox\"",
            "disabled=\"\"",
            "checked",
            "href=\"#md-note\"",
            "id=\"md-note\"",
            "syn-storage",
            "syn-code",
        ] {
            assert!(
                rendered.contains(expected),
                "missing {expected}: {rendered}"
            );
        }
        assert!(!rendered.contains("style="));
    }

    #[test]
    fn malicious_markdown_and_html_are_cleaned() {
        let rendered = markdown("<script>alert(1)</script>\n\n<img src=x onerror=alert(1)>\n\n[bad](javascript:alert%281%29)\n\n<svg onload=alert(2)></svg><iframe srcdoc=x></iframe>\n\n<a href=\"jav&#x61;script:alert(1)\" onclick=\"alert(1)\">bad</a>\n\n<div id=\"rpaste-source\" class=\"source-view\" style=\"position:fixed\">safe</div>\n\n<input type=\"password\" name=\"password\" autofocus onfocus=\"alert(1)\">\n").expect("markdown");
        for forbidden in [
            "<script",
            "<svg",
            "<iframe",
            "javascript:",
            "onclick",
            "onerror",
            "onload",
            "onfocus",
            "style=",
            "autofocus",
            "type=\"password\"",
            "name=",
            "class=\"source-view\"",
        ] {
            assert!(
                !rendered.contains(forbidden),
                "unsafe {forbidden}: {rendered}"
            );
        }
        assert!(rendered.contains("id=\"md-rpaste-source\""));
    }

    #[test]
    fn unknown_fence_is_plain_text_and_info_cannot_inject_html() {
        let rendered =
            markdown("```unknown\" onclick=\"alert(1)\n<script>alert(1)</script>\n```\n")
                .expect("markdown");
        assert!(rendered.contains("&lt;script&gt;"));
        assert!(!rendered.contains("<script>"));
        assert!(!rendered.contains("onclick="));
    }

    #[test]
    fn markdown_cannot_activate_htmx_or_theme_controls() {
        let rendered = markdown("<div hx-get=\"/raw/abcdefgh\" hx-trigger=\"load\" data-hx-post=\"/paste\" data-theme=\"dark\">safe</div>").expect("markdown");
        assert!(rendered.contains("safe"));
        for attribute in ["hx-get", "hx-trigger", "data-hx", "data-theme"] {
            assert!(!rendered.contains(attribute));
        }
    }

    #[test]
    fn long_lines_fall_back_without_truncation_and_themes_exist() {
        let content = format!("{}<script>", "x".repeat(9000));
        let rendered = highlight(&content, "Rust").expect("fallback");
        assert!(rendered.starts_with(&"x".repeat(9000)));
        assert!(rendered.ends_with("&lt;script&gt;"));
        let css = syntax_css().expect("themes");
        assert!(css.contains(".syn-code"));
        assert!(css.contains("prefers-color-scheme: dark"));
        assert!(css.contains(":root[data-theme=dark]"));
        assert!(css.contains(":root:not([data-theme=light])"));
    }
}
