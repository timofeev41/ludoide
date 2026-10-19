//! dbt support: project detection, Jinja-aware linting and a small "compile"
//! that turns a model into plain SQL you can run against a connection.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct DbtProject {
    /// Directory containing `dbt_project.yml`.
    pub root: PathBuf,
    /// `name:` from `dbt_project.yml` (falls back to the directory name).
    pub name: String,
}

impl DbtProject {
    /// The dbt project a `.sql` file belongs to, if any.
    pub fn for_file(path: &Path) -> Option<Self> {
        if path.extension().and_then(|e| e.to_str()) != Some("sql") {
            return None;
        }
        let mut dir = path.parent();
        while let Some(d) = dir {
            let manifest = d.join("dbt_project.yml");
            if manifest.is_file() {
                let yml = std::fs::read_to_string(&manifest).unwrap_or_default();
                let name = yml
                    .lines()
                    .find_map(|l| l.strip_prefix("name:"))
                    .map(|n| n.trim().trim_matches(['"', '\'']).to_owned())
                    .filter(|n| !n.is_empty())
                    .unwrap_or_else(|| d.file_name().unwrap_or_default().to_string_lossy().into_owned());
                return Some(Self { root: d.to_path_buf(), name });
            }
            dir = d.parent();
        }
        None
    }

    pub fn default_schema(&self) -> String {
        format!("{}_marts", self.name)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum TagKind {
    Expr,      // {{ ... }}
    Statement, // {% ... %}
    Comment,   // {# ... #}
}

#[derive(Debug)]
struct Tag<'a> {
    start: usize,
    end: usize,
    kind: TagKind,
    inner: &'a str,
}

/// All Jinja tags in `text`, as byte ranges.
fn tags(text: &str) -> Vec<Tag<'_>> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 1 < b.len() {
        let kind = match (b[i], b[i + 1]) {
            (b'{', b'{') => TagKind::Expr,
            (b'{', b'%') => TagKind::Statement,
            (b'{', b'#') => TagKind::Comment,
            _ => {
                i += 1;
                continue;
            }
        };
        let close: &[u8] = match kind {
            TagKind::Expr => b"}}",
            TagKind::Statement => b"%}",
            TagKind::Comment => b"#}",
        };
        // Find the closing delimiter outside string literals.
        let mut j = i + 2;
        let mut quote: Option<u8> = None;
        let mut end = None;
        while j < b.len() {
            let c = b[j];
            match quote {
                Some(q) if c == q => quote = None,
                Some(_) => {}
                None if (c == b'\'' || c == b'"') && kind != TagKind::Comment => quote = Some(c),
                None if j + 1 < b.len() && b[j] == close[0] && b[j + 1] == close[1] => {
                    end = Some(j + 2);
                    break;
                }
                None => {}
            }
            j += 1;
        }
        let Some(end) = end else { break };
        let inner = text[i + 2..end - 2].trim_matches(|c: char| c == '-' || c.is_whitespace());
        out.push(Tag { start: i, end, kind, inner });
        i = end;
    }
    out
}

/// A parsed `name(arg, ...)` call; string arguments are unquoted.
struct Call {
    name: String,
    args: Vec<String>,
}

fn parse_call(expr: &str) -> Option<Call> {
    // Drop Jinja filters (`env_var('X') | int`).
    let expr = split_top_level(expr, '|').into_iter().next()?.trim().to_owned();
    let open = expr.find('(')?;
    if !expr.ends_with(')') {
        return None;
    }
    let name = expr[..open].trim().to_owned();
    if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '.') {
        return None;
    }
    let args = split_top_level(&expr[open + 1..expr.len() - 1], ',')
        .into_iter()
        .map(|a| {
            let a = a.trim();
            // keyword args: keep the value
            let a = match a.split_once('=') {
                Some((k, v)) if k.trim().chars().all(|c| c.is_alphanumeric() || c == '_') => v.trim(),
                _ => a,
            };
            a.trim_matches(['"', '\'']).to_owned()
        })
        .filter(|a| !a.is_empty())
        .collect();
    Some(Call { name, args })
}

fn split_top_level(s: &str, sep: char) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let mut last = 0;
    for (i, c) in s.char_indices() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None => match c {
                '\'' | '"' => quote = Some(c),
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => depth -= 1,
                c if c == sep && depth == 0 => {
                    out.push(&s[last..i]);
                    last = i + c.len_utf8();
                }
                _ => {}
            },
        }
    }
    out.push(&s[last..]);
    out
}

/// Replace Jinja with SQL-neutral text of the same shape so the SQL linter
/// only sees SQL: expressions become identifiers, everything else blanks.
/// Line structure is preserved so diagnostics keep their line numbers.
pub fn strip_for_lint(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for t in tags(text) {
        out.push_str(&text[last..t.start]);
        let raw = &text[t.start..t.end];
        let is_config = t.kind == TagKind::Expr && parse_call(t.inner).is_some_and(|c| c.name == "config");
        let identifier = t.kind == TagKind::Expr && !is_config;
        for (n, line) in raw.split('\n').enumerate() {
            if n > 0 {
                out.push('\n');
            }
            let fill = if identifier && n == 0 { '_' } else { ' ' };
            out.extend(std::iter::repeat_n(fill, line.chars().count()));
        }
        last = t.end;
    }
    out.push_str(&text[last..]);
    out
}

#[derive(Clone, Debug, PartialEq)]
pub struct Variable {
    pub name: String,
    pub default: Option<String>,
}

/// Unique `env_var(...)` / `var(...)` names, in order of first appearance.
pub fn variables(text: &str) -> Vec<Variable> {
    let mut out: Vec<Variable> = Vec::new();
    for t in tags(text).into_iter().filter(|t| t.kind == TagKind::Expr) {
        if let Some(c) = parse_call(t.inner) {
            if matches!(c.name.as_str(), "env_var" | "var") {
                if let Some(name) = c.args.first() {
                    if !out.iter().any(|v| &v.name == name) {
                        out.push(Variable { name: name.clone(), default: c.args.get(1).cloned() });
                    }
                }
            }
        }
    }
    out
}

pub struct Compiled {
    pub sql: String,
    /// Tags left as-is because they can't be resolved without dbt.
    pub unresolved: Vec<String>,
}

/// Resolve the common dbt constructs:
/// - `{{ config(...) }}` and `{# comments #}` are removed
/// - `{{ env_var('X') }}` / `{{ var('X') }}` → `values["X"]` (or the default)
/// - `{{ source('s', 't') }}` → `s.t`
/// - `{{ ref('m') }}` → `<schema>.m`, `{{ this }}` → `<schema>.<model>`
/// - `{% if is_incremental() %}…{% endif %}` → dropped (else-branch kept), i.e. full-refresh SQL
pub fn compile(text: &str, values: &HashMap<String, String>, schema: &str, model: &str) -> Compiled {
    let tags = tags(text);
    let mut out = String::with_capacity(text.len());
    let mut unresolved = Vec::new();
    let mut last = 0;
    let mut i = 0;
    while i < tags.len() {
        let t = &tags[i];
        out.push_str(&text[last..t.start]);
        last = t.end;
        let raw = &text[t.start..t.end];
        match t.kind {
            TagKind::Comment => {}
            TagKind::Statement => {
                let inner = t.inner.split_whitespace().collect::<Vec<_>>().join(" ");
                if inner == "if is_incremental()" {
                    // Skip to the matching endif, keeping an else-branch if there is one.
                    let mut depth = 0;
                    let mut j = i + 1;
                    let mut else_at = None;
                    while j < tags.len() {
                        let s = tags[j].inner.split_whitespace().next().unwrap_or_default();
                        if tags[j].kind == TagKind::Statement {
                            match s {
                                "if" => depth += 1,
                                "endif" if depth == 0 => break,
                                "endif" => depth -= 1,
                                "else" if depth == 0 => else_at = Some(j),
                                _ => {}
                            }
                        }
                        j += 1;
                    }
                    if j < tags.len() {
                        if let Some(e) = else_at {
                            out.push_str(&compile_range(
                                &text[tags[e].end..tags[j].start],
                                values,
                                schema,
                                model,
                                &mut unresolved,
                            ));
                        }
                        last = tags[j].end;
                        i = j + 1;
                        continue;
                    }
                }
                unresolved.push(raw.to_owned());
                out.push_str(raw);
            }
            TagKind::Expr => match resolve(t.inner, values, schema, model) {
                Some(s) => out.push_str(&s),
                None => {
                    unresolved.push(raw.to_owned());
                    out.push_str(raw);
                }
            },
        }
        i += 1;
    }
    out.push_str(&text[last..]);
    // Removing `config(...)` usually leaves leading blank lines.
    let sql = out.trim_start_matches(['\n', '\r', ' ']).to_owned();
    Compiled { sql, unresolved }
}

fn compile_range(
    text: &str,
    values: &HashMap<String, String>,
    schema: &str,
    model: &str,
    unresolved: &mut Vec<String>,
) -> String {
    let c = compile(text, values, schema, model);
    unresolved.extend(c.unresolved);
    c.sql
}

fn resolve(inner: &str, values: &HashMap<String, String>, schema: &str, model: &str) -> Option<String> {
    if inner.trim() == "this" {
        return Some(format!("{schema}.{model}"));
    }
    let c = parse_call(inner)?;
    match c.name.as_str() {
        "config" => Some(String::new()),
        "env_var" | "var" => {
            let name = c.args.first()?;
            values.get(name).filter(|v| !v.is_empty()).cloned().or_else(|| c.args.get(1).cloned())
        }
        "source" if c.args.len() >= 2 => Some(format!("{}.{}", c.args[0], c.args[1])),
        "ref" => c.args.last().map(|m| format!("{schema}.{m}")),
        _ => None,
    }
}

/// Where a compiled model is written: `<scratches>/compiled/<project>/<stem>_cmp_<timestamp>.sql`.
pub fn output_path(scratches: &Path, project: &str, model_path: &Path) -> PathBuf {
    let stem = model_path.file_stem().unwrap_or_default().to_string_lossy();
    let stamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
    scratches.join("compiled").join(project).join(format!("{stem}_cmp_{stamp}.sql"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODEL: &str = r#"{{
    config(
        materialized='table',
        order_by='(d)'
    )
}}

-- {# a comment #}
select toDate(time) d, count() c
FROM {{ source('userway', 'enriched') }} e
left join {{ ref('media_source_rules') }} r on r.id = e.id
WHERE toDate(time) < toDate('{{ env_var("END_DATE") }}')
    and toDate(time) >= toDate('{{ env_var("START_DATE") }}')
    and toDate(time) != toDate('{{ env_var("END_DATE") }}')
{% if is_incremental() %}
    and d > (select max(d) from {{ this }})
{% endif %}
group by d
"#;

    #[test]
    fn finds_variables_once() {
        let v = variables(MODEL);
        let names: Vec<&str> = v.iter().map(|v| v.name.as_str()).collect();
        assert_eq!(names, ["END_DATE", "START_DATE"]);
        assert_eq!(variables("{{ var('x', 'def') | int }}")[0].default.as_deref(), Some("def"));
    }

    #[test]
    fn compiles_model() {
        let vals = HashMap::from([
            ("END_DATE".to_owned(), "2026-10-01".to_owned()),
            ("START_DATE".to_owned(), "2026-09-01".to_owned()),
        ]);
        let c = compile(MODEL, &vals, "attribution_marts", "daily");
        assert!(c.unresolved.is_empty(), "{:?}", c.unresolved);
        assert!(c.sql.starts_with("-- "), "{}", c.sql);
        assert!(c.sql.contains("FROM userway.enriched e"));
        assert!(c.sql.contains("left join attribution_marts.media_source_rules r"));
        assert_eq!(c.sql.matches("'2026-10-01'").count(), 2);
        assert!(c.sql.contains("toDate('2026-09-01')"));
        assert!(!c.sql.contains("is_incremental") && !c.sql.contains("max(d)") && !c.sql.contains("{{"));
        assert!(c.sql.contains("group by d"));
    }

    #[test]
    fn keeps_unknown_jinja() {
        let c = compile("select {{ my_macro(1) }} {% for x in y %}", &HashMap::new(), "s", "m");
        assert_eq!(c.unresolved.len(), 2);
        assert!(c.sql.contains("{{ my_macro(1) }}"));
    }

    #[test]
    fn strips_for_lint_keeping_lines() {
        let s = strip_for_lint(MODEL);
        assert_eq!(s.lines().count(), MODEL.lines().count());
        assert!(!s.contains('{'));
        assert!(s.contains("FROM ______"));
        let d = crate::lint::sql::lint(&s, &sqlparser::dialect::ClickHouseDialect {});
        assert!(d.iter().all(|d| !d.message.contains("{")), "{d:?}");
    }

    #[test]
    fn detects_project() {
        let dir = std::env::temp_dir().join(format!("ludo-dbt-{}", crate::db::new_id()));
        std::fs::create_dir_all(dir.join("models/marts")).unwrap();
        std::fs::write(dir.join("dbt_project.yml"), "name: 'attribution'\nversion: '1.0'\n").unwrap();
        let p = DbtProject::for_file(&dir.join("models/marts/x.sql")).unwrap();
        assert_eq!(p.name, "attribution");
        assert_eq!(p.default_schema(), "attribution_marts");
        assert!(DbtProject::for_file(&dir.join("models/marts/x.py")).is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
