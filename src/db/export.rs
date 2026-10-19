//! Copy formats for query results.

use super::QueryResult;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum CopyFormat {
    #[default]
    Tsv,
    Csv,
    Plain,
    SqlInsert,
    Json,
    Markdown,
}

impl CopyFormat {
    pub const ALL: [CopyFormat; 6] = [
        CopyFormat::Tsv,
        CopyFormat::Csv,
        CopyFormat::Plain,
        CopyFormat::SqlInsert,
        CopyFormat::Json,
        CopyFormat::Markdown,
    ];

    pub fn label(self) -> &'static str {
        match self {
            CopyFormat::Tsv => "TSV",
            CopyFormat::Csv => "CSV",
            CopyFormat::Plain => "Plain text",
            CopyFormat::SqlInsert => "SQL INSERTs",
            CopyFormat::Json => "JSON",
            CopyFormat::Markdown => "Markdown",
        }
    }
}

fn is_null(v: &str) -> bool {
    v == "NULL"
}

fn is_numeric_type(t: &str) -> bool {
    let t = t.trim_start_matches("Nullable(").trim_start_matches("LowCardinality(");
    ["Int", "UInt", "Float", "Decimal", "Bool"].iter().any(|p| t.starts_with(p))
}

fn csv_field(v: &str) -> String {
    if v.contains([',', '"', '\n', '\r']) { format!("\"{}\"", v.replace('"', "\"\"")) } else { v.to_owned() }
}

fn sql_literal(v: &str, ty: &str) -> String {
    if is_null(v) {
        "NULL".into()
    } else if is_numeric_type(ty) && !v.is_empty() {
        v.to_owned()
    } else {
        format!("'{}'", v.replace('\\', "\\\\").replace('\'', "\\'"))
    }
}

fn json_value(v: &str, ty: &str) -> serde_json::Value {
    if is_null(v) {
        return serde_json::Value::Null;
    }
    if is_numeric_type(ty) {
        if let Ok(n) = serde_json::from_str::<serde_json::Number>(v) {
            return serde_json::Value::Number(n);
        }
    }
    serde_json::Value::String(v.to_owned())
}

/// Best-effort table name for INSERT statements: the first `FROM x` of the query.
pub fn table_from_sql(sql: &str) -> String {
    let words: Vec<&str> = sql.split_whitespace().collect();
    words
        .windows(2)
        .find(|w| w[0].eq_ignore_ascii_case("from") && !w[1].starts_with('('))
        .map(|w| w[1].trim_end_matches([';', ')']).to_owned())
        .unwrap_or_else(|| "result".into())
}

pub fn export(q: &QueryResult, format: CopyFormat, table: &str) -> String {
    let names: Vec<&str> = q.columns.iter().map(|c| c.0.as_str()).collect();
    let mut out = String::new();
    match format {
        CopyFormat::Tsv => {
            out.push_str(&names.join("\t"));
            for r in &q.rows {
                out.push('\n');
                let cells: Vec<String> = r.iter().map(|c| c.replace(['\t', '\n'], " ")).collect();
                out.push_str(&cells.join("\t"));
            }
        }
        CopyFormat::Csv => {
            out.push_str(&names.iter().map(|n| csv_field(n)).collect::<Vec<_>>().join(","));
            for r in &q.rows {
                out.push('\n');
                out.push_str(&r.iter().map(|c| csv_field(c)).collect::<Vec<_>>().join(","));
            }
        }
        CopyFormat::Plain | CopyFormat::Markdown => {
            let md = format == CopyFormat::Markdown;
            let mut widths: Vec<usize> = names.iter().map(|n| n.chars().count()).collect();
            for r in &q.rows {
                for (i, c) in r.iter().enumerate() {
                    widths[i] = widths[i].max(c.chars().count());
                }
            }
            let line = |cells: Vec<&str>| {
                let padded: Vec<String> =
                    cells.iter().zip(&widths).map(|(c, w)| format!("{c:<w$}", c = c.replace('\n', " "))).collect();
                if md { format!("| {} |", padded.join(" | ")) } else { padded.join("  ").trim_end().to_owned() }
            };
            out.push_str(&line(names.clone()));
            out.push('\n');
            if md {
                let dashes: Vec<String> = widths.iter().map(|w| "-".repeat(*w)).collect();
                out.push_str(&format!("| {} |", dashes.join(" | ")));
            } else {
                out.push_str(&widths.iter().map(|w| "-".repeat(*w)).collect::<Vec<_>>().join("  "));
            }
            for r in &q.rows {
                out.push('\n');
                out.push_str(&line(r.iter().map(String::as_str).collect()));
            }
        }
        CopyFormat::SqlInsert => {
            let cols = names.join(", ");
            for (i, r) in q.rows.iter().enumerate() {
                if i > 0 {
                    out.push('\n');
                }
                let vals: Vec<String> = r.iter().zip(&q.columns).map(|(v, (_, t))| sql_literal(v, t)).collect();
                out.push_str(&format!("INSERT INTO {table} ({cols}) VALUES ({});", vals.join(", ")));
            }
        }
        CopyFormat::Json => {
            let rows: Vec<serde_json::Value> = q
                .rows
                .iter()
                .map(|r| {
                    let obj: serde_json::Map<String, serde_json::Value> =
                        r.iter().zip(&q.columns).map(|(v, (n, t))| (n.clone(), json_value(v, t))).collect();
                    serde_json::Value::Object(obj)
                })
                .collect();
            out = serde_json::to_string_pretty(&rows).unwrap_or_default();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> QueryResult {
        QueryResult {
            columns: vec![("id".into(), "UInt64".into()), ("name".into(), "Nullable(String)".into())],
            rows: vec![
                vec!["1".into(), "a,b".into()],
                vec!["2".into(), "NULL".into()],
                vec!["3".into(), "it's".into()],
            ],
            ..Default::default()
        }
    }

    #[test]
    fn formats() {
        let q = sample();
        assert_eq!(export(&q, CopyFormat::Tsv, "t"), "id\tname\n1\ta,b\n2\tNULL\n3\tit's");
        assert_eq!(export(&q, CopyFormat::Csv, "t"), "id,name\n1,\"a,b\"\n2,NULL\n3,it's");
        assert_eq!(
            export(&q, CopyFormat::SqlInsert, "db.t"),
            "INSERT INTO db.t (id, name) VALUES (1, 'a,b');\nINSERT INTO db.t (id, name) VALUES (2, NULL);\nINSERT INTO db.t (id, name) VALUES (3, 'it\\'s');"
        );
        assert_eq!(export(&q, CopyFormat::Plain, "t").lines().nth(1), Some("--  ----"));
        assert!(export(&q, CopyFormat::Markdown, "t").starts_with("| id | name |\n| -- | ---- |"));
        let json: serde_json::Value = serde_json::from_str(&export(&q, CopyFormat::Json, "t")).unwrap();
        assert_eq!(json[0]["id"], 1);
        assert!(json[1]["name"].is_null());
        assert_eq!(table_from_sql("select * from analytics.events limit 5"), "analytics.events");
        assert_eq!(table_from_sql("select 1"), "result");
    }
}
