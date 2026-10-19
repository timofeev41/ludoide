use super::{Diagnostic, Severity};
use sqlparser::dialect::Dialect;
use sqlparser::parser::Parser;

/// Parse each `;`-separated statement on its own so one error does not hide the rest.
pub fn lint(text: &str, dialect: &dyn Dialect) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for (start_line, stmt) in split_statements(text) {
        if stmt.trim().is_empty() || is_comment_only(&stmt) {
            continue;
        }
        if let Err(e) = Parser::parse_sql(dialect, &stmt) {
            let msg = e.to_string();
            let (line, col) = location(&msg).unwrap_or((1, 1));
            let clean =
                msg.split(" at Line:").next().unwrap_or(&msg).trim_start_matches("sql parser error: ").to_owned();
            out.push(Diagnostic {
                line: start_line + line - 1,
                col,
                // The parser does not know every ClickHouse extension, so this is advisory.
                severity: Severity::Warning,
                message: clean,
            });
        }
    }
    out
}

fn is_comment_only(stmt: &str) -> bool {
    stmt.lines().all(|l| {
        let t = l.trim();
        t.is_empty() || t.starts_with("--")
    })
}

fn location(msg: &str) -> Option<(usize, usize)> {
    let rest = &msg[msg.find("Line: ")? + 6..];
    let (line, rest) = rest.split_once(',')?;
    let col = rest.trim().strip_prefix("Column: ")?;
    let col: String = col.chars().take_while(|c| c.is_ascii_digit()).collect();
    Some((line.trim().parse().ok()?, col.parse().ok()?))
}

/// Returns `(1-based start line, statement text)` pairs.
fn split_statements(text: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut line = 1;
    let mut start_line = 1;
    let mut quote: Option<char> = None;
    let mut chars = text.chars().peekable();
    let mut in_comment = false;
    while let Some(c) = chars.next() {
        if cur.trim().is_empty() && !c.is_whitespace() {
            start_line = line;
            // Leading blank lines are dropped so line numbers stay aligned.
            cur.clear();
        }
        if c == '\n' {
            line += 1;
            in_comment = false;
        }
        cur.push(c);
        if in_comment {
            continue;
        }
        match quote {
            Some(q) => {
                if c == '\\' {
                    if let Some(n) = chars.next() {
                        if n == '\n' {
                            line += 1;
                        }
                        cur.push(n);
                    }
                } else if c == q {
                    quote = None;
                }
            }
            None => match c {
                '\'' | '"' | '`' => quote = Some(c),
                '-' if chars.peek() == Some(&'-') => in_comment = true,
                ';' => {
                    out.push((start_line, std::mem::take(&mut cur)));
                }
                _ => {}
            },
        }
    }
    if !cur.trim().is_empty() {
        out.push((start_line, cur));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlparser::dialect::ClickHouseDialect;

    #[test]
    fn reports_line_of_broken_statement() {
        let d = lint("select 1;\n\nselect from where;\n", &ClickHouseDialect {});
        assert_eq!(d.len(), 1, "{d:?}");
        assert_eq!(d[0].line, 3);
        assert!(lint("select a, b from t where a = ';'", &ClickHouseDialect {}).is_empty());
        assert!(lint("-- only a comment", &ClickHouseDialect {}).is_empty());
    }
}
