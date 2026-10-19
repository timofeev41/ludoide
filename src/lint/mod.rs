//! Linters produce [`Diagnostic`]s; the editor shows them in the gutter and
//! in the Problems panel. Add a linter by adding a module and a match arm in [`lint`].

pub mod sql;
pub mod yaml;

use crate::languages::LangId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Clone, Debug)]
pub struct Diagnostic {
    /// 1-based line.
    pub line: usize,
    pub col: usize,
    pub severity: Severity,
    pub message: String,
}

pub struct LintContext<'a> {
    pub sql_dialect: &'a dyn sqlparser::dialect::Dialect,
    pub sql: bool,
    pub yaml: bool,
}

pub fn lint(lang: LangId, text: &str, ctx: &LintContext) -> Vec<Diagnostic> {
    let mut out = match lang {
        LangId::Sql if ctx.sql => sql::lint(text, ctx.sql_dialect),
        LangId::Yaml if ctx.yaml => yaml::lint(text),
        _ => Vec::new(),
    };
    out.sort_by_key(|d| (d.line, d.col));
    out
}
