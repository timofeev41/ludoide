//! Small, fast, regex-free syntax highlighter driven by the language registry.

use crate::languages::{LangId, Language};
use crate::theme::Palette;
use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tok {
    Plain,
    Keyword,
    Builtin,
    Str,
    Num,
    Comment,
    Key,
    Punct,
    Heading,
}

fn color(t: Tok, p: &Palette) -> Color32 {
    match t {
        Tok::Plain => p.text,
        Tok::Keyword => p.syn_keyword,
        Tok::Builtin => p.syn_builtin,
        Tok::Str => p.syn_string,
        Tok::Num => p.syn_number,
        Tok::Comment => p.syn_comment,
        Tok::Key => p.syn_key,
        Tok::Punct => p.syn_punct,
        Tok::Heading => p.syn_builtin,
    }
}

pub fn highlight(text: &str, lang: &Language, font: FontId, p: &Palette) -> LayoutJob {
    let mut job = LayoutJob::default();
    let push = |job: &mut LayoutJob, s: &str, t: Tok| {
        if s.is_empty() {
            return;
        }
        let mut fmt = TextFormat::simple(font.clone(), color(t, p));
        if t == Tok::Comment {
            fmt.italics = true;
        }
        job.append(s, 0.0, fmt);
    };
    match lang.id {
        LangId::Markdown => {
            highlight_markdown(text, |s, t| push(&mut job, s, t));
            return job;
        }
        LangId::Plain => {
            push(&mut job, text, Tok::Plain);
            return job;
        }
        _ => {}
    }

    let case_insensitive = lang.id == LangId::Sql;
    let comment = lang.line_comment.unwrap_or("\u{0}");
    let mut i = 0;
    let mut line_start = true;
    while i < text.len() {
        let rest = &text[i..];
        let c = rest.chars().next().unwrap();
        if c == '\n' {
            push(&mut job, "\n", Tok::Plain);
            i += 1;
            line_start = true;
            continue;
        }
        if c.is_whitespace() {
            let n = rest.find(|ch: char| !ch.is_whitespace() || ch == '\n').unwrap_or(rest.len());
            push(&mut job, &rest[..n], Tok::Plain);
            i += n;
            continue;
        }
        // Jinja (dbt): {{ expr }}, {% stmt %}, {# comment #}
        if lang.id == LangId::Sql && (rest.starts_with("{{") || rest.starts_with("{%") || rest.starts_with("{#")) {
            let close = match &rest[..2] {
                "{{" => "}}",
                "{%" => "%}",
                _ => "#}",
            };
            let n = rest[2..].find(close).map_or(rest.len(), |e| e + 4);
            let tok = if close == "#}" { Tok::Comment } else { Tok::Key };
            push(&mut job, &rest[..n], tok);
            i += n;
            line_start = false;
            continue;
        }
        // Comments
        if rest.starts_with(comment) || (lang.id == LangId::Sql && rest.starts_with("/*")) {
            let n = if rest.starts_with("/*") {
                rest.find("*/").map_or(rest.len(), |e| e + 2)
            } else {
                rest.find('\n').unwrap_or(rest.len())
            };
            push(&mut job, &rest[..n], Tok::Comment);
            i += n;
            line_start = false;
            continue;
        }
        // Strings (incl. Python triple quotes)
        if c == '"' || c == '\'' || (c == '`' && lang.id != LangId::Yaml) {
            let n = if lang.id == LangId::Python && (rest.starts_with("\"\"\"") || rest.starts_with("'''")) {
                let q = &rest[..3];
                rest[3..].find(q).map_or(rest.len(), |e| e + 6)
            } else {
                string_len(rest, c)
            };
            let tok =
                if lang.id == LangId::Json && rest[n..].trim_start().starts_with(':') { Tok::Key } else { Tok::Str };
            push(&mut job, &rest[..n], tok);
            i += n;
            line_start = false;
            continue;
        }
        if c.is_ascii_digit() {
            let n = rest.find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '.' || ch == '_')).unwrap_or(rest.len());
            push(&mut job, &rest[..n], Tok::Num);
            i += n;
            line_start = false;
            continue;
        }
        if c.is_alphanumeric() || c == '_' || c == '$' {
            let n = rest
                .find(|ch: char| {
                    !(ch.is_alphanumeric() || ch == '_' || (lang.id == LangId::Yaml && (ch == '-' || ch == '.')))
                })
                .unwrap_or(rest.len())
                .max(c.len_utf8());
            let word = &rest[..n];
            let after = rest[n..].trim_start_matches([' ', '\t']);
            let tok = if (lang.id == LangId::Yaml || lang.id == LangId::Toml)
                && (after.starts_with(':') || after.starts_with('=') && lang.id == LangId::Toml)
                && line_start
            {
                Tok::Key
            } else if matches_word(lang.keywords, word, case_insensitive) {
                Tok::Keyword
            } else if matches_word(lang.builtins, word, case_insensitive) {
                Tok::Builtin
            } else if lang.id == LangId::Shell && c == '$' {
                Tok::Key
            } else {
                Tok::Plain
            };
            push(&mut job, word, tok);
            i += n;
            line_start = false;
            continue;
        }
        // YAML list dashes keep "line start" so `- key:` still colors the key.
        let keep_line_start = lang.id == LangId::Yaml && c == '-' && line_start;
        let n = c.len_utf8();
        push(&mut job, &text[i..i + n], Tok::Punct);
        i += n;
        line_start = keep_line_start;
    }
    job
}

fn matches_word(list: &[&str], word: &str, ci: bool) -> bool {
    if ci { list.iter().any(|k| k.eq_ignore_ascii_case(word)) } else { list.contains(&word) }
}

fn string_len(s: &str, q: char) -> usize {
    let mut escaped = false;
    for (i, c) in s.char_indices().skip(1) {
        if c == '\n' {
            return i;
        }
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == q {
            return i + c.len_utf8();
        }
    }
    s.len()
}

fn highlight_markdown(text: &str, mut push: impl FnMut(&str, Tok)) {
    let mut in_fence = false;
    for line in text.split_inclusive('\n') {
        let t = line.trim_start();
        if t.starts_with("```") {
            in_fence = !in_fence;
            push(line, Tok::Comment);
        } else if in_fence {
            push(line, Tok::Str);
        } else if t.starts_with('#') {
            push(line, Tok::Heading);
        } else if t.starts_with('>') {
            push(line, Tok::Comment);
        } else if t.starts_with("- ") || t.starts_with("* ") || t.starts_with("+ ") {
            let lead = line.len() - t.len();
            push(&line[..lead + 2], Tok::Keyword);
            push_inline(&line[lead + 2..], &mut push);
        } else {
            push_inline(line, &mut push);
        }
    }
}

fn push_inline(line: &str, push: &mut impl FnMut(&str, Tok)) {
    let mut rest = line;
    while let Some(start) = rest.find('`') {
        push(&rest[..start], Tok::Plain);
        let after = &rest[start + 1..];
        match after.find('`') {
            Some(end) => {
                push(&rest[start..start + end + 2], Tok::Str);
                rest = &after[end + 1..];
            }
            None => {
                push(&rest[start..], Tok::Plain);
                return;
            }
        }
    }
    push(rest, Tok::Plain);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::languages::{LANGUAGES, get};

    #[test]
    fn preserves_text_for_every_language() {
        let sample =
            "select 'a''b', \"x\" -- c\n# h\nkey: [1, 2.5] # é ünï\n  - name: `t` $VAR\n/* x */ def f(): '''doc'''\n";
        let p = crate::theme::palette("dark");
        for l in LANGUAGES {
            let job = highlight(sample, get(l.id), FontId::monospace(12.0), &p);
            assert_eq!(job.text, sample, "{}", l.name);
        }
    }
}
