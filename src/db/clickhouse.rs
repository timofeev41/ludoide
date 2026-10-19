//! ClickHouse over its HTTP interface.

use super::{Column, ConnectionConfig, DebugLog, Driver, ObjectKind, QueryResult, SchemaObject};
use serde_json::Value;
use std::time::{Duration, Instant};

pub struct ClickHouse {
    agent: ureq::Agent,
    url: String,
    user: String,
    password: String,
    database: String,
}

/// Quote a string literal.
pub fn lit(s: &str) -> String {
    format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'"))
}

/// Quote an identifier.
pub fn ident(s: &str) -> String {
    format!("`{}`", s.replace('\\', "\\\\").replace('`', "\\`"))
}

fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn cell_to_string(v: &Value) -> String {
    match v {
        Value::Null => "NULL".into(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

struct Raw {
    body: String,
    summary: Option<String>,
}

impl ClickHouse {
    pub fn new(cfg: &ConnectionConfig, timeout: Duration) -> Self {
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(timeout))
            // System TLS (Security.framework on macOS) so Keychain-trusted CAs work.
            .tls_config(ureq::tls::TlsConfig::builder().provider(ureq::tls::TlsProvider::NativeTls).build())
            .build()
            .into();
        Self {
            agent,
            url: cfg.url.trim().trim_end_matches('/').to_owned(),
            user: cfg.username.trim().to_owned(),
            password: cfg.password.clone(),
            database: cfg.database.trim().to_owned(),
        }
    }

    fn post(&self, sql: &str, params: &[(&str, String)], log: &mut DebugLog) -> Result<Raw, String> {
        let mut url = format!("{}/?", self.url);
        let mut all: Vec<(&str, String)> = params.to_vec();
        if !self.database.is_empty() && !all.iter().any(|(k, _)| *k == "database") {
            all.push(("database", self.database.clone()));
        }
        let qs: Vec<String> = all.iter().map(|(k, v)| format!("{k}={}", url_encode(v))).collect();
        url.push_str(&qs.join("&"));
        log.push(format!("POST {url}"));
        let started = Instant::now();
        let mut resp = self
            .agent
            .post(&url)
            .header("X-ClickHouse-User", &self.user)
            .header("X-ClickHouse-Key", &self.password)
            .send(sql)
            .map_err(|e| {
                let msg = format!("request failed: {e}");
                log.push(msg.clone());
                msg
            })?;
        let status = resp.status();
        let header = |name: &str| resp.headers().get(name).and_then(|v| v.to_str().ok()).map(str::to_owned);
        let summary = header("x-clickhouse-summary");
        for h in ["x-clickhouse-query-id", "x-clickhouse-server-display-name", "x-clickhouse-format"] {
            if let Some(v) = header(h) {
                log.push(format!("{h}: {v}"));
            }
        }
        if let Some(s) = &summary {
            log.push(format!("x-clickhouse-summary: {s}"));
        }
        let body = resp
            .body_mut()
            .with_config()
            .limit(1 << 30)
            .read_to_string()
            .map_err(|e| format!("reading response: {e}"))?;
        log.push(format!("HTTP {} in {:.1?}, {} bytes", status.as_u16(), started.elapsed(), body.len()));
        if !status.is_success() {
            let msg = body.trim().to_owned();
            log.push(format!("error: {msg}"));
            return Err(if msg.is_empty() { format!("HTTP {status}") } else { msg });
        }
        Ok(Raw { body, summary })
    }

    /// Run a metadata query and return rows of JSON values.
    fn rows(&self, sql: &str, log: &mut DebugLog) -> Result<Vec<Vec<Value>>, String> {
        let raw = self.post(&format!("{sql} FORMAT JSONCompact"), &[], log)?;
        let v: Value = serde_json::from_str(&raw.body).map_err(|e| format!("bad JSON: {e}"))?;
        Ok(v["data"]
            .as_array()
            .map(|rows| rows.iter().filter_map(|r| r.as_array().cloned()).collect())
            .unwrap_or_default())
    }

    fn text(&self, sql: &str, log: &mut DebugLog) -> Result<String, String> {
        Ok(self.post(&format!("{sql} FORMAT TSVRaw"), &[], log)?.body)
    }
}

/// Whether the statement returns rows we should render as a grid.
fn returns_rows(sql: &str) -> bool {
    let first =
        strip_leading_comments(sql).split(|c: char| !c.is_alphanumeric()).next().unwrap_or_default().to_lowercase();
    matches!(first.as_str(), "select" | "with" | "show" | "describe" | "desc" | "explain" | "exists" | "check")
}

fn strip_leading_comments(sql: &str) -> &str {
    let mut s = sql.trim_start();
    loop {
        if let Some(rest) = s.strip_prefix("--") {
            s = rest.split_once('\n').map_or("", |(_, r)| r).trim_start();
        } else if let Some(rest) = s.strip_prefix("/*") {
            s = rest.split_once("*/").map_or("", |(_, r)| r).trim_start();
        } else {
            return s;
        }
    }
}

/// Whether the user already chose an output format (`... FORMAT CSV`).
fn has_format_clause(sql: &str) -> bool {
    let lower = sql.to_lowercase();
    lower.split_whitespace().rev().nth(1) == Some("format")
}

fn object_kind(engine: &str) -> ObjectKind {
    match engine {
        "View" | "LiveView" | "WindowView" => ObjectKind::View,
        "MaterializedView" => ObjectKind::MaterializedView,
        "Dictionary" => ObjectKind::Dictionary,
        _ => ObjectKind::Table,
    }
}

impl Driver for ClickHouse {
    fn ping(&self, log: &mut DebugLog) -> Result<String, String> {
        let v = self.text("SELECT version()", log)?;
        Ok(format!("ClickHouse {}", v.trim()))
    }

    fn databases(&self, log: &mut DebugLog) -> Result<Vec<String>, String> {
        Ok(self
            .rows("SELECT name FROM system.databases ORDER BY name", log)?
            .iter()
            .map(|r| cell_to_string(&r[0]))
            .collect())
    }

    fn objects(&self, db: &str, log: &mut DebugLog) -> Result<Vec<SchemaObject>, String> {
        let mut out: Vec<SchemaObject> = self
            .rows(
                &format!(
                    "SELECT name, engine, total_rows, total_bytes FROM system.tables \
                     WHERE database = {} ORDER BY name",
                    lit(db)
                ),
                log,
            )?
            .iter()
            .map(|r| {
                let engine = cell_to_string(&r[1]);
                let num = |v: &Value| match v {
                    Value::Number(n) => n.as_u64(),
                    Value::String(s) => s.parse().ok(),
                    _ => None,
                };
                SchemaObject {
                    name: cell_to_string(&r[0]),
                    kind: object_kind(&engine),
                    detail: engine,
                    parent: None,
                    rows: num(&r[2]),
                    bytes: num(&r[3]),
                }
            })
            .collect();
        // Older servers may lack the table; the tree still works without indices.
        if let Ok(rows) = self.rows(
            &format!(
                "SELECT table, name, type, expr FROM system.data_skipping_indices \
                 WHERE database = {} ORDER BY table, name",
                lit(db)
            ),
            log,
        ) {
            out.extend(rows.iter().map(|r| SchemaObject {
                name: cell_to_string(&r[1]),
                kind: ObjectKind::Index,
                detail: format!("{} · {}", cell_to_string(&r[2]), cell_to_string(&r[3])),
                parent: Some(cell_to_string(&r[0])),
                rows: None,
                bytes: None,
            }));
        }
        Ok(out)
    }

    fn columns(&self, db: &str, object: &str, log: &mut DebugLog) -> Result<Vec<Column>, String> {
        Ok(self
            .rows(
                &format!(
                    "SELECT name, type, is_in_primary_key, is_in_sorting_key, default_kind, comment \
                     FROM system.columns WHERE database = {} AND table = {} ORDER BY position",
                    lit(db),
                    lit(object)
                ),
                log,
            )?
            .iter()
            .map(|r| {
                let flag = |v: &Value| matches!(v, Value::Number(n) if n.as_u64() == Some(1));
                let mut detail = Vec::new();
                if flag(&r[2]) {
                    detail.push("PK".to_owned());
                } else if flag(&r[3]) {
                    detail.push("sort".to_owned());
                }
                let dk = cell_to_string(&r[4]);
                if !dk.is_empty() {
                    detail.push(dk);
                }
                let comment = cell_to_string(&r[5]);
                if !comment.is_empty() {
                    detail.push(comment);
                }
                Column { name: cell_to_string(&r[0]), data_type: cell_to_string(&r[1]), detail: detail.join(" · ") }
            })
            .collect())
    }

    fn object_ddl(&self, db: &str, obj: &SchemaObject, log: &mut DebugLog) -> Result<String, String> {
        let sql = match obj.kind {
            ObjectKind::Index => {
                let table = obj.parent.as_deref().unwrap_or_default();
                return Ok(format!(
                    "ALTER TABLE {}.{} ADD INDEX {} {} GRANULARITY ...;\n-- {}",
                    ident(db),
                    ident(table),
                    ident(&obj.name),
                    obj.detail.split(" · ").nth(1).unwrap_or_default(),
                    obj.detail
                ));
            }
            ObjectKind::Dictionary => format!("SHOW CREATE DICTIONARY {}.{}", ident(db), ident(&obj.name)),
            _ => format!("SHOW CREATE TABLE {}.{}", ident(db), ident(&obj.name)),
        };
        Ok(format!("{};\n", self.text(&sql, log)?.trim_end()))
    }

    fn database_ddl(&self, db: &str, log: &mut DebugLog) -> Result<String, String> {
        let mut out = format!("{};\n", self.text(&format!("SHOW CREATE DATABASE {}", ident(db)), log)?.trim_end());
        let rows = self.rows(
            &format!(
                "SELECT create_table_query FROM system.tables WHERE database = {} \
                 ORDER BY dependencies_table = [] DESC, name",
                lit(db)
            ),
            log,
        )?;
        for r in rows {
            let q = cell_to_string(&r[0]);
            if !q.is_empty() {
                out.push_str(&format!("\n{};\n", q.trim_end()));
            }
        }
        Ok(out)
    }

    fn query(
        &self,
        sql: &str,
        database: Option<&str>,
        query_id: &str,
        max_rows: usize,
        log: &mut DebugLog,
    ) -> Result<QueryResult, String> {
        let started = Instant::now();
        let tabular = returns_rows(sql);
        let custom_format = has_format_clause(sql);
        let mut params = vec![("query_id", query_id.to_owned()), ("send_progress_in_http_headers", "0".into())];
        if let Some(db) = database.filter(|d| !d.is_empty()) {
            params.push(("database", db.to_owned()));
        }
        if tabular && !custom_format {
            params.push(("default_format", "JSONCompact".into()));
            params.push(("max_result_rows", max_rows.to_string()));
            params.push(("result_overflow_mode", "break".into()));
            params.push(("output_format_json_quote_64bit_integers", "0".into()));
        }
        let raw = self.post(sql, &params, log)?;
        let mut res = QueryResult { elapsed: started.elapsed(), ..Default::default() };
        if let Some(summary) = raw.summary.as_deref().and_then(|s| serde_json::from_str::<Value>(s).ok()) {
            let n = |k: &str| summary[k].as_str().and_then(|s| s.parse().ok());
            res.rows_read = n("read_rows");
            res.bytes_read = n("read_bytes");
        }
        if tabular && !custom_format {
            let v: Value = serde_json::from_str(&raw.body).map_err(|e| format!("bad JSON: {e}"))?;
            res.columns = v["meta"]
                .as_array()
                .map(|m| m.iter().map(|c| (cell_to_string(&c["name"]), cell_to_string(&c["type"]))).collect())
                .unwrap_or_default();
            let data = v["data"].as_array().cloned().unwrap_or_default();
            res.truncated = data.len() >= max_rows;
            res.rows = data
                .iter()
                .take(max_rows)
                .map(|r| r.as_array().map(|c| c.iter().map(cell_to_string).collect()).unwrap_or_default())
                .collect();
            if let Some(st) = v.get("statistics") {
                res.rows_read = res.rows_read.or(st["rows_read"].as_u64());
                res.bytes_read = res.bytes_read.or(st["bytes_read"].as_u64());
            }
        } else if !raw.body.trim().is_empty() {
            res.columns = vec![("result".into(), "String".into())];
            res.rows = raw.body.lines().take(max_rows).map(|l| vec![l.to_owned()]).collect();
        } else {
            res.message = Some("OK".into());
        }
        Ok(res)
    }

    fn cancel(&self, query_id: &str, log: &mut DebugLog) -> Result<(), String> {
        self.post(&format!("KILL QUERY WHERE query_id = {} ASYNC", lit(query_id)), &[], log)?;
        Ok(())
    }

    fn inspect_sql(&self, db: &str, object: &str, limit: usize) -> String {
        format!("SELECT * FROM {}.{} LIMIT {limit}", ident(db), ident(object))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies() {
        assert!(returns_rows("-- hi\n  SELECT 1"));
        assert!(returns_rows("/* x */ with a as (select 1) select * from a"));
        assert!(!returns_rows("CREATE TABLE t (a Int8) ENGINE = Memory"));
        assert!(has_format_clause("select 1 FORMAT CSV"));
        assert!(!has_format_clause("select format from t"));
        assert_eq!(lit("a'b"), "'a\\'b'");
    }

    /// Runs against a live server: `LUDO_CH_URL=http://localhost:8123 cargo test -- --ignored`.
    /// Expects the `analytics` fixture from README "Development".
    #[test]
    #[ignore]
    fn live_server() {
        let Ok(url) = std::env::var("LUDO_CH_URL") else { return };
        let cfg = ConnectionConfig { url, ..Default::default() };
        let ch = ClickHouse::new(&cfg, Duration::from_secs(30));
        let mut log = Vec::new();
        assert!(ch.ping(&mut log).unwrap().starts_with("ClickHouse"));
        assert!(ch.databases(&mut log).unwrap().contains(&"analytics".to_owned()));
        let objs = ch.objects("analytics", &mut log).unwrap();
        let kind = |n: &str| objs.iter().find(|o| o.name == n).map(|o| o.kind);
        assert_eq!(kind("events"), Some(ObjectKind::Table));
        assert_eq!(kind("v_buys"), Some(ObjectKind::View));
        assert_eq!(kind("mv_daily"), Some(ObjectKind::MaterializedView));
        assert_eq!(kind("idx_event"), Some(ObjectKind::Index));
        let cols = ch.columns("analytics", "events", &mut log).unwrap();
        assert_eq!(cols[0].name, "ts");
        let events = objs.iter().find(|o| o.name == "events").unwrap();
        assert!(ch.object_ddl("analytics", events, &mut log).unwrap().starts_with("CREATE TABLE analytics.events"));
        assert!(ch.database_ddl("analytics", &mut log).unwrap().contains("CREATE MATERIALIZED VIEW"));
        let q = ch.query(&ch.inspect_sql("analytics", "events", 500), None, "t1", 10_000, &mut log).unwrap();
        assert_eq!(q.rows.len(), 500);
        assert_eq!(q.columns.len(), 5);
        assert!(q.rows.iter().any(|r| r[4] == "NULL"));
        let q = ch.query("select number from system.numbers limit 50", None, "t2", 10, &mut log).unwrap();
        assert!(q.truncated && q.rows.len() == 10);
        let q = ch.query("select 1 as a FORMAT CSV", None, "t3", 10, &mut log).unwrap();
        assert_eq!(q.rows, vec![vec!["1".to_owned()]]);
        let q = ch
            .query("create table if not exists default.tmp_ludo (a Int8) engine = Memory", None, "t4", 10, &mut log)
            .unwrap();
        assert_eq!(q.message.as_deref(), Some("OK"));
        let q = ch.query("select count() from events", Some("analytics"), "t5", 10, &mut log).unwrap();
        assert_eq!(q.rows[0][0], "100000");
        let err = ch.query("select * from nope", None, "t6", 10, &mut log).unwrap_err();
        assert!(err.contains("UNKNOWN_TABLE") || err.contains("doesn't exist"), "{err}");
        let bad = ClickHouse::new(&ConnectionConfig { username: "ghost".into(), ..cfg }, Duration::from_secs(5));
        assert!(bad.ping(&mut log).is_err());
    }
}
