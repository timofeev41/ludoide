//! Pure text operations behind the editor shortcuts.
//!
//! Every operation works on char indices (the unit egui's `CCursor` uses) and
//! returns the new text plus the new selection, so they are trivially testable.

/// A selection expressed as `(start, end)` char indices, `start <= end`.
pub type Sel = (usize, usize);

pub struct Edit {
    pub text: String,
    pub sel: Sel,
}

/// Closing counterpart for characters that wrap a selection.
pub fn closing_pair(open: char) -> Option<char> {
    Some(match open {
        '(' => ')',
        '[' => ']',
        '{' => '}',
        '<' => '>',
        '"' => '"',
        '\'' => '\'',
        '`' => '`',
        _ => return None,
    })
}

/// Surround the selection with `open` and its closing pair, keeping the inner text selected.
pub fn wrap_selection(text: &str, sel: Sel, open: char) -> Option<Edit> {
    let close = closing_pair(open)?;
    if sel.0 == sel.1 {
        return None;
    }
    let chars: Vec<char> = text.chars().collect();
    let mut out: String = chars[..sel.0].iter().collect();
    out.push(open);
    out.extend(&chars[sel.0..sel.1]);
    out.push(close);
    out.extend(&chars[sel.1..]);
    Some(Edit { text: out, sel: (sel.0 + 1, sel.1 + 1) })
}

/// Char index range `[first_line_start, last_line_end)` covering the selection,
/// where `last_line_end` excludes the trailing newline.
fn line_block(chars: &[char], sel: Sel) -> (usize, usize) {
    let mut start = sel.0.min(chars.len());
    while start > 0 && chars[start - 1] != '\n' {
        start -= 1;
    }
    // A selection ending right after a newline does not include the next line.
    let mut end_anchor = sel.1.min(chars.len());
    if end_anchor > sel.0 && end_anchor > 0 && chars[end_anchor - 1] == '\n' {
        end_anchor -= 1;
    }
    let mut end = end_anchor;
    while end < chars.len() && chars[end] != '\n' {
        end += 1;
    }
    (start, end)
}

fn split_lines(chars: &[char]) -> Vec<String> {
    chars.iter().collect::<String>().split('\n').map(str::to_owned).collect()
}

/// Move the lines touched by the selection one line up or down.
pub fn move_lines(text: &str, sel: Sel, up: bool) -> Option<Edit> {
    let chars: Vec<char> = text.chars().collect();
    let (bs, be) = line_block(&chars, sel);
    if up {
        if bs == 0 {
            return None;
        }
        // previous line: [ps, bs-1)
        let mut ps = bs - 1;
        while ps > 0 && chars[ps - 1] != '\n' {
            ps -= 1;
        }
        let prev: String = chars[ps..bs - 1].iter().collect();
        let block: String = chars[bs..be].iter().collect();
        let mut out: String = chars[..ps].iter().collect();
        out.push_str(&block);
        out.push('\n');
        out.push_str(&prev);
        out.extend(&chars[be..]);
        let shift = bs - ps;
        Some(Edit { text: out, sel: (sel.0 - shift, sel.1 - shift) })
    } else {
        if be >= chars.len() {
            return None;
        }
        // next line: [be+1, ne)
        let mut ne = be + 1;
        while ne < chars.len() && chars[ne] != '\n' {
            ne += 1;
        }
        let next: String = chars[be + 1..ne].iter().collect();
        let block: String = chars[bs..be].iter().collect();
        let mut out: String = chars[..bs].iter().collect();
        out.push_str(&next);
        out.push('\n');
        out.push_str(&block);
        out.extend(&chars[ne..]);
        let shift = ne - be;
        Some(Edit { text: out, sel: (sel.0 + shift, sel.1 + shift) })
    }
}

/// Apply `f` to each line of the selected block, then re-select the whole block.
fn map_block(text: &str, sel: Sel, mut f: impl FnMut(&str) -> String) -> Edit {
    let chars: Vec<char> = text.chars().collect();
    let (bs, be) = line_block(&chars, sel);
    let lines = split_lines(&chars[bs..be]);
    let mapped: Vec<String> = lines.iter().map(|l| f(l)).collect();
    let block = mapped.join("\n");
    let block_len = block.chars().count();
    let mut out: String = chars[..bs].iter().collect();
    out.push_str(&block);
    out.extend(&chars[be..]);
    Edit { text: out, sel: (bs, bs + block_len) }
}

pub fn indent_lines(text: &str, sel: Sel, indent: usize) -> Edit {
    let pad = " ".repeat(indent);
    map_block(text, sel, |l| if l.is_empty() { String::new() } else { format!("{pad}{l}") })
}

pub fn outdent_lines(text: &str, sel: Sel, indent: usize) -> Edit {
    map_block(text, sel, |l| {
        if let Some(rest) = l.strip_prefix('\t') {
            return rest.to_owned();
        }
        let n = l.chars().take(indent).take_while(|c| *c == ' ').count();
        l[n..].to_owned()
    })
}

/// Comment the block with `prefix` unless every non-blank line already is commented.
pub fn toggle_comment(text: &str, sel: Sel, prefix: &str) -> Edit {
    let chars: Vec<char> = text.chars().collect();
    let (bs, be) = line_block(&chars, sel);
    let lines = split_lines(&chars[bs..be]);
    let all_commented = lines.iter().filter(|l| !l.trim().is_empty()).all(|l| l.trim_start().starts_with(prefix));
    let min_indent =
        lines.iter().filter(|l| !l.trim().is_empty()).map(|l| l.len() - l.trim_start().len()).min().unwrap_or(0);
    map_block(text, sel, |l| {
        if l.trim().is_empty() {
            return l.to_owned();
        }
        if all_commented {
            let ws = l.len() - l.trim_start().len();
            let rest = &l[ws + prefix.len()..];
            let rest = rest.strip_prefix(' ').unwrap_or(rest);
            format!("{}{}", &l[..ws], rest)
        } else {
            format!("{}{} {}", &l[..min_indent], prefix, &l[min_indent..])
        }
    })
}

/// Insert a newline keeping the current line's indentation; indent one more
/// level after an opening bracket or a trailing `:` (Python/YAML blocks).
pub fn newline_with_indent(text: &str, sel: Sel, indent: usize) -> Edit {
    let chars: Vec<char> = text.chars().collect();
    let mut ls = sel.0;
    while ls > 0 && chars[ls - 1] != '\n' {
        ls -= 1;
    }
    let lead: String = chars[ls..sel.0].iter().take_while(|c| **c == ' ' || **c == '\t').collect();
    let before = chars[ls..sel.0].iter().collect::<String>();
    let extra = match before.trim_end().chars().last() {
        Some('(' | '[' | '{' | ':') => " ".repeat(indent),
        _ => String::new(),
    };
    let ins = format!("\n{lead}{extra}");
    let n = ins.chars().count();
    let mut out: String = chars[..sel.0].iter().collect();
    out.push_str(&ins);
    out.extend(&chars[sel.1..]);
    Edit { text: out, sel: (sel.0 + n, sel.0 + n) }
}

/// Insert spaces up to the next indentation stop.
pub fn insert_soft_tab(text: &str, sel: Sel, indent: usize) -> Edit {
    let chars: Vec<char> = text.chars().collect();
    let mut ls = sel.0;
    while ls > 0 && chars[ls - 1] != '\n' {
        ls -= 1;
    }
    let col = sel.0 - ls;
    let n = indent - (col % indent.max(1));
    let mut out: String = chars[..sel.0].iter().collect();
    out.push_str(&" ".repeat(n));
    out.extend(&chars[sel.1..]);
    Edit { text: out, sel: (sel.0 + n, sel.0 + n) }
}

/// Whether the selection spans more than one line.
pub fn is_multiline(text: &str, sel: Sel) -> bool {
    text.chars().skip(sel.0).take(sel.1 - sel.0).any(|c| c == '\n')
}

/// 1-based (line, column) for a char index.
pub fn line_col(text: &str, idx: usize) -> (usize, usize) {
    let mut line = 1;
    let mut col = 1;
    for c in text.chars().take(idx) {
        if c == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}

/// The SQL statement around the cursor: text between the surrounding `;`
/// separators (string literals and comments are respected).
pub fn statement_at(text: &str, idx: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut bounds = vec![0usize];
    let mut i = 0;
    let mut quote: Option<char> = None;
    while i < chars.len() {
        let c = chars[i];
        match quote {
            Some(q) => {
                if c == '\\' {
                    i += 1;
                } else if c == q {
                    quote = None;
                }
            }
            None => match c {
                '\'' | '"' | '`' => quote = Some(c),
                '-' if chars.get(i + 1) == Some(&'-') => {
                    while i < chars.len() && chars[i] != '\n' {
                        i += 1;
                    }
                }
                ';' => bounds.push(i + 1),
                _ => {}
            },
        }
        i += 1;
    }
    bounds.push(chars.len() + 1);
    for w in bounds.windows(2) {
        let (s, e) = (w[0], w[1].min(chars.len() + 1));
        if idx >= s && idx < e {
            let e = (e).min(chars.len());
            let stmt: String = chars[s..e].iter().collect();
            let trimmed = stmt.trim().trim_end_matches(';').trim();
            if trimmed.is_empty() && s > 0 {
                // Cursor right after a `;`: run the previous statement.
                return statement_at(text, s.saturating_sub(2));
            }
            return trimmed.to_owned();
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps() {
        let e = wrap_selection("select a from t", (7, 8), '(').unwrap();
        assert_eq!(e.text, "select (a) from t");
        assert_eq!(e.sel, (8, 9));
        assert!(wrap_selection("x", (0, 0), '(').is_none());
    }

    #[test]
    fn moves_lines() {
        let e = move_lines("a\nb\nc", (2, 2), true).unwrap();
        assert_eq!(e.text, "b\na\nc");
        assert_eq!(e.sel, (0, 0));
        let e = move_lines("a\nb\nc", (2, 3), false).unwrap();
        assert_eq!(e.text, "a\nc\nb");
        assert_eq!(e.sel, (4, 5));
        assert!(move_lines("a\nb", (0, 0), true).is_none());
        assert!(move_lines("a\nb", (2, 2), false).is_none());
    }

    #[test]
    fn indents() {
        let e = indent_lines("a\nb\n\nc", (0, 3), 4);
        assert_eq!(e.text, "    a\n    b\n\nc");
        let e = outdent_lines("    a\n  b\nc", (0, 10), 4);
        assert_eq!(e.text, "a\nb\nc");
    }

    #[test]
    fn comments() {
        let e = toggle_comment("  select 1\n  from t", (0, 5), "--");
        assert_eq!(e.text, "  -- select 1\n  from t");
        let e = toggle_comment(&e.text, (0, 0), "--");
        assert_eq!(e.text, "  select 1\n  from t");
        let e = toggle_comment("a\n\nb", (0, 4), "#");
        assert_eq!(e.text, "# a\n\n# b");
    }

    #[test]
    fn newline_indent() {
        let e = newline_with_indent("def f():", (8, 8), 4);
        assert_eq!(e.text, "def f():\n    ");
        let e = newline_with_indent("  x = 1", (7, 7), 4);
        assert_eq!(e.text, "  x = 1\n  ");
    }

    #[test]
    fn statements() {
        let sql = "select 1;\nselect ';' as x;\n-- c; \nselect 3";
        assert_eq!(statement_at(sql, 3), "select 1");
        assert_eq!(statement_at(sql, 12), "select ';' as x");
        assert_eq!(statement_at(sql, sql.chars().count()), "-- c; \nselect 3");
        assert_eq!(statement_at("select 1;", 9), "select 1");
    }
}
