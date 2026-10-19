//! Database layer. Every engine implements [`Driver`]; the UI only talks to the
//! trait, so adding PostgreSQL/MySQL/MongoDB means adding a [`DriverKind`]
//! variant and a module implementing [`Driver`].

pub mod clickhouse;
pub mod worker;

use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DriverKind {
    ClickHouse,
}

impl DriverKind {
    pub const ALL: &'static [DriverKind] = &[DriverKind::ClickHouse];

    pub fn name(self) -> &'static str {
        match self {
            DriverKind::ClickHouse => "ClickHouse",
        }
    }

    pub fn default_url(self) -> &'static str {
        match self {
            DriverKind::ClickHouse => "http://localhost:8123",
        }
    }

    pub fn default_user(self) -> &'static str {
        match self {
            DriverKind::ClickHouse => "default",
        }
    }

    /// SQL dialect used to lint scripts run against this kind of connection.
    pub fn sql_dialect(self) -> Box<dyn sqlparser::dialect::Dialect> {
        match self {
            DriverKind::ClickHouse => Box::new(sqlparser::dialect::ClickHouseDialect {}),
        }
    }

    pub fn connect(self, cfg: &ConnectionConfig, timeout: Duration) -> Box<dyn Driver> {
        match self {
            DriverKind::ClickHouse => Box::new(clickhouse::ClickHouse::new(cfg, timeout)),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ConnectionConfig {
    pub id: String,
    pub name: String,
    pub kind: DriverKind,
    pub url: String,
    pub username: String,
    pub password: String,
    /// Default database; empty = server default.
    pub database: String,
}

impl Default for ConnectionConfig {
    fn default() -> Self {
        let kind = DriverKind::ClickHouse;
        Self {
            id: new_id(),
            name: String::new(),
            kind,
            url: kind.default_url().into(),
            username: kind.default_user().into(),
            password: String::new(),
            database: String::new(),
        }
    }
}

impl ConnectionConfig {
    /// Identity used to enforce "one connection per URL + user".
    pub fn identity(&self) -> (String, String) {
        (normalize_url(&self.url), self.username.trim().to_owned())
    }

    pub fn display_name(&self) -> String {
        if self.name.trim().is_empty() { format!("{}@{}", self.username, self.url) } else { self.name.clone() }
    }
}

pub fn normalize_url(url: &str) -> String {
    url.trim().trim_end_matches('/').to_lowercase()
}

pub fn new_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
    format!("{t:x}-{:x}", N.fetch_add(1, Ordering::Relaxed))
}

/// Validate that `cfg` does not duplicate an existing connection.
pub fn check_unique(all: &[ConnectionConfig], cfg: &ConnectionConfig) -> Result<(), String> {
    let id = cfg.identity();
    match all.iter().find(|c| c.id != cfg.id && c.identity() == id) {
        Some(other) => Err(format!(
            "A connection for {} with user '{}' already exists ({})",
            cfg.url.trim(),
            cfg.username.trim(),
            other.display_name()
        )),
        None => Ok(()),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ObjectKind {
    Table,
    View,
    MaterializedView,
    Dictionary,
    Index,
}

impl ObjectKind {
    pub const GROUPS: &'static [ObjectKind] =
        &[ObjectKind::Table, ObjectKind::View, ObjectKind::MaterializedView, ObjectKind::Dictionary, ObjectKind::Index];

    pub fn group_name(self) -> &'static str {
        match self {
            ObjectKind::Table => "tables",
            ObjectKind::View => "views",
            ObjectKind::MaterializedView => "materialized views",
            ObjectKind::Dictionary => "dictionaries",
            ObjectKind::Index => "indices",
        }
    }

    /// Whether `SELECT * FROM` makes sense for this object.
    pub fn queryable(self) -> bool {
        !matches!(self, ObjectKind::Index)
    }
}

#[derive(Clone, Debug)]
pub struct SchemaObject {
    pub name: String,
    pub kind: ObjectKind,
    /// Engine / index type, shown dimmed next to the name.
    pub detail: String,
    /// For indices: the table they belong to.
    pub parent: Option<String>,
    pub rows: Option<u64>,
    pub bytes: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct Column {
    pub name: String,
    pub data_type: String,
    pub detail: String,
}

#[derive(Clone, Debug, Default)]
pub struct QueryResult {
    pub columns: Vec<(String, String)>,
    pub rows: Vec<Vec<String>>,
    pub elapsed: Duration,
    pub rows_read: Option<u64>,
    pub bytes_read: Option<u64>,
    /// Server-side row count when the result was cut by the row limit.
    pub truncated: bool,
    /// Non-tabular outcome (DDL/INSERT), shown instead of the grid when empty.
    pub message: Option<String>,
}

/// Debug lines produced while talking to the server; shown in the DB log.
pub type DebugLog = Vec<String>;

pub trait Driver: Send + Sync {
    /// Returns a short server description (e.g. version).
    fn ping(&self, log: &mut DebugLog) -> Result<String, String>;
    fn databases(&self, log: &mut DebugLog) -> Result<Vec<String>, String>;
    fn objects(&self, db: &str, log: &mut DebugLog) -> Result<Vec<SchemaObject>, String>;
    fn columns(&self, db: &str, object: &str, log: &mut DebugLog) -> Result<Vec<Column>, String>;
    /// `CREATE ...` statement for one object.
    fn object_ddl(&self, db: &str, obj: &SchemaObject, log: &mut DebugLog) -> Result<String, String>;
    /// All `CREATE ...` statements of a database.
    fn database_ddl(&self, db: &str, log: &mut DebugLog) -> Result<String, String>;
    fn query(
        &self,
        sql: &str,
        database: Option<&str>,
        query_id: &str,
        max_rows: usize,
        log: &mut DebugLog,
    ) -> Result<QueryResult, String>;
    fn cancel(&self, query_id: &str, log: &mut DebugLog) -> Result<(), String>;
    fn inspect_sql(&self, db: &str, object: &str, limit: usize) -> String;
}

pub fn human_bytes(b: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut v = b as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 { format!("{b} B") } else { format!("{v:.1} {}", UNITS[u]) }
}

pub fn human_count(n: u64) -> String {
    match n {
        0..1_000 => n.to_string(),
        1_000..1_000_000 => format!("{:.1}K", n as f64 / 1e3),
        1_000_000..1_000_000_000 => format!("{:.1}M", n as f64 / 1e6),
        _ => format!("{:.1}B", n as f64 / 1e9),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_by_url_and_user() {
        let a = ConnectionConfig { url: "http://Host:8123/".into(), ..Default::default() };
        let b = ConnectionConfig { url: "http://host:8123".into(), ..Default::default() };
        assert!(check_unique(std::slice::from_ref(&a), &b).is_err());
        let c = ConnectionConfig { username: "other".into(), ..b.clone() };
        assert!(check_unique(&[a.clone()], &c).is_ok());
        // Editing a connection does not conflict with itself.
        assert!(check_unique(&[a.clone()], &a).is_ok());
    }
}
