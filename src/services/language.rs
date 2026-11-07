use std::sync::LazyLock;

use syntect::parsing::{SyntaxReference, SyntaxSet};

use crate::error::AppError;

pub static SYNTAXES: LazyLock<SyntaxSet> = LazyLock::new(two_face::syntax::extra_newlines);

pub struct LanguageOption {
    pub value: String,
    pub label: String,
}

pub fn options() -> Vec<LanguageOption> {
    let mut syntaxes: Vec<_> = SYNTAXES
        .syntaxes()
        .iter()
        .filter(|syntax| {
            !syntax.hidden && !matches!(syntax.name.as_str(), "Plain Text" | "Markdown")
        })
        .map(|syntax| LanguageOption {
            value: syntax.name.clone(),
            label: syntax.name.clone(),
        })
        .collect();
    syntaxes.sort_by_cached_key(|syntax| syntax.label.to_ascii_lowercase());
    let mut result = vec![
        LanguageOption {
            value: "auto".into(),
            label: "Auto".into(),
        },
        LanguageOption {
            value: "text".into(),
            label: "Plain text".into(),
        },
        LanguageOption {
            value: "markdown".into(),
            label: "Markdown".into(),
        },
    ];
    result.extend(syntaxes);
    result
}

pub fn syntax(token: &str) -> Option<&'static SyntaxReference> {
    let token = token.trim();
    let alias = match token.to_ascii_lowercase().as_str() {
        "text" | "txt" | "plain" | "plaintext" => "Plain Text",
        "md" | "markdown" => "Markdown",
        "js" | "javascript" => "JavaScript",
        "ts" | "typescript" => "TypeScript",
        "py" | "python" => "Python",
        "rs" | "rust" => "Rust",
        "sh" | "shell" | "bash" => "Bash",
        "yml" | "yaml" => "YAML",
        "c++" | "cpp" => "C++",
        "csharp" | "cs" | "c#" => "C#",
        _ => token,
    };
    SYNTAXES
        .syntaxes()
        .iter()
        .find(|syntax| syntax.name.eq_ignore_ascii_case(alias))
        .or_else(|| SYNTAXES.find_syntax_by_token(alias))
}

fn canonical(syntax: &SyntaxReference) -> String {
    match syntax.name.as_str() {
        "Plain Text" => "text".into(),
        "Markdown" => "markdown".into(),
        name => name.to_owned(),
    }
}

pub fn download_filename(id: &str, language: &str) -> String {
    let extension = syntax(language).map_or("txt", |syntax| match syntax.name.as_str() {
        "Plain Text" => "txt",
        "Markdown" => "md",
        "HTML" => "html",
        "JavaScript" => "js",
        "TypeScript" => "ts",
        "Python" => "py",
        "Rust" => "rs",
        "Bash" => "sh",
        "C++" => "cpp",
        "C#" => "cs",
        "YAML" => "yaml",
        _ => syntax
            .file_extensions
            .iter()
            .find(|extension| {
                !extension.is_empty()
                    && extension.len() <= 32
                    && extension.bytes().all(|byte| byte.is_ascii_alphanumeric())
            })
            .map_or("txt", String::as_str),
    });
    format!("{id}.{extension}")
}

pub fn resolve(requested: Option<&str>, content: &str) -> Result<String, AppError> {
    match requested.map(str::trim) {
        None | Some("") => Ok(detect(content)),
        Some(token) if token.eq_ignore_ascii_case("auto") => Ok(detect(content)),
        Some(token) => syntax(token)
            .map(canonical)
            .ok_or(AppError::BadRequest("Unknown language")),
    }
}

pub fn detect(content: &str) -> String {
    let mut end = content.len().min(16 * 1024);
    while !content.is_char_boundary(end) {
        end -= 1;
    }
    let sample = content[..end].trim_start();
    let first_line = sample.lines().next().unwrap_or_default();
    if first_line.len() <= 1024
        && (first_line.starts_with("#!") || first_line.starts_with("<?xml"))
        && let Some(syntax) = SYNTAXES.find_syntax_by_first_line(first_line)
    {
        return canonical(syntax);
    }
    let lower = sample.to_ascii_lowercase();
    let token = if sample.lines().any(|line| {
        let line = line.trim_start();
        line.starts_with("```") || line.starts_with("~~~")
    }) {
        "Markdown"
    } else if (sample.starts_with('{') || sample.starts_with('['))
        && serde_json::from_str::<serde_json::Value>(content).is_ok()
    {
        "JSON"
    } else if lower.starts_with("<!doctype html") || lower.starts_with("<html") {
        "HTML"
    } else if sample.contains("fn main(")
        || sample.contains("use std::")
        || sample.contains("let mut ")
    {
        "Rust"
    } else if sample.contains("public class ") || sample.contains("public static void main(") {
        "Java"
    } else if sample.contains("#include") && sample.contains("std::") {
        "C++"
    } else if sample.contains("#include") {
        "C"
    } else if sample.starts_with("package ") && sample.contains("func ") {
        "Go"
    } else if sample.lines().any(|line| {
        let line = line.trim();
        (line.starts_with("def ") || line.starts_with("async def ") || line.starts_with("class "))
            && line.ends_with(':')
    }) || sample.contains("if __name__ ==")
    {
        "Python"
    } else if sample.lines().any(|line| {
        let line = line.trim_start().trim_start_matches("export ");
        line.starts_with("interface ") || (line.starts_with("type ") && line.contains(" = "))
    }) {
        "TypeScript"
    } else if sample.contains("console.log(")
        || sample.contains("function ")
        || sample.contains(" => ")
    {
        "JavaScript"
    } else if [
        "select ",
        "insert into ",
        "create table ",
        "update ",
        "delete from ",
    ]
    .iter()
    .any(|prefix| lower.starts_with(prefix))
    {
        "SQL"
    } else if sample.starts_with("FROM ")
        && (sample.contains("\nRUN ") || sample.contains("\nCOPY "))
    {
        "Dockerfile"
    } else if sample.contains('{')
        && sample.contains('}')
        && (sample.contains("color:") || sample.contains("display:"))
    {
        "CSS"
    } else if sample.lines().any(|line| {
        line.starts_with("# ")
            || line.starts_with("## ")
            || line.starts_with("### ")
            || line.starts_with("```")
            || line.starts_with("~~~")
            || line.starts_with("> ")
            || line.starts_with("- [ ] ")
            || line.starts_with("- [x] ")
    }) {
        "Markdown"
    } else {
        "Plain Text"
    };
    syntax(token)
        .map(canonical)
        .unwrap_or_else(|| "text".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn download_extensions_follow_languages_and_have_a_safe_fallback() {
        for (language, extension) in [
            ("markdown", "md"),
            ("MD", "md"),
            ("HTML", "html"),
            ("js", "js"),
            ("ts", "ts"),
            ("py", "py"),
            ("rs", "rs"),
            ("bash", "sh"),
            ("C++", "cpp"),
            ("C#", "cs"),
            ("JSON", "json"),
            ("CSS", "css"),
            ("YAML", "yaml"),
            ("text", "txt"),
            ("unknown", "txt"),
            ("../bad\r\n", "txt"),
        ] {
            assert_eq!(
                download_filename("Ab12Cd34", language),
                format!("Ab12Cd34.{extension}")
            );
        }
        for option in options() {
            let name = download_filename("Ab12Cd34", &option.value);
            assert!(
                name.strip_prefix("Ab12Cd34.")
                    .expect("prefix")
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric())
            );
        }
    }

    #[test]
    fn more_than_one_hundred_languages_and_aliases() {
        assert!(options().len() > 103);
        for (input, expected) in [
            ("rs", "Rust"),
            ("python", "Python"),
            ("ts", "TypeScript"),
            ("md", "markdown"),
            ("TXT", "text"),
            ("Dockerfile", "Dockerfile"),
        ] {
            assert_eq!(resolve(Some(input), "ignored").expect("language"), expected);
        }
        assert!(resolve(Some("not-a-language"), "hello").is_err());
    }

    #[test]
    fn conservative_detection_and_explicit_override() {
        for (content, expected) in [
            ("fn main() { println!(\"hi\"); }", "Rust"),
            ("def hello():\n    return 1", "Python"),
            ("#!/usr/bin/env python3\nprint('hello')", "Python"),
            ("{\"hello\": true}", "JSON"),
            ("# Heading\n\n**hello**", "markdown"),
            ("# Example\n\n```rust\nfn main() {}\n```", "markdown"),
            ("interface User { name: string }", "TypeScript"),
            ("package main\nfunc main() {}", "Go"),
            ("hello world", "text"),
        ] {
            assert_eq!(resolve(None, content).expect("detected language"), expected);
        }
        assert_eq!(
            resolve(Some("text"), "# Heading").expect("override"),
            "text"
        );
        assert_eq!(detect(&"中".repeat(10_000)), "text");
    }
}
