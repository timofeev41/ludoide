use super::{Diagnostic, Severity};
use serde::Deserialize;

pub fn lint(text: &str) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let indent: String = line.chars().take_while(|c| c.is_whitespace()).collect();
        if indent.contains('\t') {
            out.push(Diagnostic {
                line: i + 1,
                col: 1,
                severity: Severity::Error,
                message: "tab characters are not allowed in YAML indentation".into(),
            });
        }
        if line.ends_with(' ') || line.ends_with('\t') {
            out.push(Diagnostic {
                line: i + 1,
                col: line.trim_end().chars().count() + 1,
                severity: Severity::Warning,
                message: "trailing whitespace".into(),
            });
        }
        if !indent.contains('\t') && indent.len() % 2 == 1 && !line.trim_start().starts_with('#') {
            out.push(Diagnostic {
                line: i + 1,
                col: 1,
                severity: Severity::Warning,
                message: format!("odd indentation ({} spaces)", indent.len()),
            });
        }
    }
    let dups = duplicate_keys(text);
    let found_dups = !dups.is_empty();
    out.extend(dups);
    for doc in serde_yaml::Deserializer::from_str(text) {
        if let Err(e) = serde_yaml::Value::deserialize(doc) {
            // Already reported with a precise location above.
            if found_dups && e.to_string().contains("duplicate entry") {
                break;
            }
            let (line, col) = e.location().map_or((1, 1), |l| (l.line(), l.column()));
            out.push(Diagnostic { line, col, severity: Severity::Error, message: e.to_string() });
            break;
        }
    }
    out
}

/// Line-based duplicate key detection (serde_yaml only reports the mapping start).
fn duplicate_keys(text: &str) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    // (indent of the mapping's keys, key -> first line)
    let mut stack: Vec<(usize, std::collections::HashMap<String, usize>)> = Vec::new();
    let mut block_scalar_indent: Option<usize> = None;
    for (i, raw) in text.lines().enumerate() {
        let trimmed = raw.trim_start();
        let indent = raw.len() - trimmed.len();
        if let Some(bi) = block_scalar_indent {
            if trimmed.is_empty() || indent > bi {
                continue;
            }
            block_scalar_indent = None;
        }
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if trimmed.starts_with("---") || trimmed.starts_with("...") {
            stack.clear();
            continue;
        }
        let (key_indent, content, new_item) = match trimmed.strip_prefix("- ") {
            Some(rest) => {
                let rest_trim = rest.trim_start();
                (indent + 2 + (rest.len() - rest_trim.len()), rest_trim, true)
            }
            None => (indent, trimmed, false),
        };
        let Some(key) = mapping_key(content) else { continue };
        while stack.last().is_some_and(|(ind, _)| *ind > key_indent) {
            stack.pop();
        }
        let need_new = match stack.last() {
            Some((ind, _)) => *ind < key_indent || new_item,
            None => true,
        };
        if need_new {
            if stack.last().is_some_and(|(ind, _)| *ind == key_indent) {
                stack.pop();
            }
            stack.push((key_indent, Default::default()));
        }
        let map = &mut stack.last_mut().unwrap().1;
        if let Some(first) = map.get(&key) {
            out.push(Diagnostic {
                line: i + 1,
                col: key_indent + 1,
                severity: Severity::Error,
                message: format!("duplicate key '{key}' (first defined on line {first})"),
            });
        } else {
            map.insert(key, i + 1);
        }
        let value = content.split_once(':').map_or("", |(_, v)| v.trim());
        if value.starts_with('|') || value.starts_with('>') {
            block_scalar_indent = Some(key_indent);
        }
    }
    out
}

fn mapping_key(content: &str) -> Option<String> {
    let (key, rest) = if let Some(q @ ('"' | '\'')) = content.chars().next() {
        let end = content[1..].find(q)? + 1;
        (&content[1..end], &content[end + 1..])
    } else {
        let colon = content.find(": ").or_else(|| content.strip_suffix(':').map(|s| s.len()))?;
        let key = &content[..colon];
        if key.contains(['{', '[', '#']) || key.trim().is_empty() {
            return None;
        }
        (key, &content[colon..])
    };
    let rest = rest.trim_start();
    (rest == ":" || rest.starts_with(": ")).then(|| key.trim().to_owned())
}

#[cfg(test)]
mod tests {
    #[test]
    fn finds_problems() {
        assert!(super::lint("a: 1\nb:\n  - x\n---\nc: 2\n").is_empty());
        let d = super::lint("a: 1\na: 2\n");
        assert!(d.iter().any(|d| d.message.contains("duplicate")), "{d:?}");
        let d =
            super::lint("x:\n  a: 1\n  b: |\n    a: in block\n  a: 2\nitems:\n  - name: q\n    name: r\n  - name: s\n");
        let lines: Vec<usize> = d.iter().map(|d| d.line).collect();
        assert_eq!(lines, vec![5, 8], "{d:?}");
        let d = super::lint("a:\n\tb: 1\n");
        assert!(d.iter().any(|d| d.message.contains("tab")));
        let d = super::lint("a: [1, 2\n");
        assert!(!d.is_empty());
    }
}
