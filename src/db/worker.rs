//! Runs driver calls off the UI thread and hands results back via a channel.

use super::{Column, ConnectionConfig, DebugLog, QueryResult, SchemaObject};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

#[derive(Clone, Debug)]
pub enum Request {
    Databases,
    Objects { db: String },
    AllObjects,
    Columns { db: String, object: String },
    ObjectDdl { db: String, object: SchemaObject },
    DatabaseDdl { db: String },
    Query { sql: String, database: Option<String>, query_id: String, max_rows: usize },
    Cancel { query_id: String },
}

impl Request {
    pub fn describe(&self) -> String {
        match self {
            Request::Databases => "list databases".into(),
            Request::Objects { db } => format!("list objects in {db}"),
            Request::AllObjects => "index all objects".into(),
            Request::Columns { db, object } => format!("columns of {db}.{object}"),
            Request::ObjectDdl { db, object } => format!("DDL of {db}.{}", object.name),
            Request::DatabaseDdl { db } => format!("DDL of database {db}"),
            Request::Query { sql, .. } => {
                let one_line: String = sql.split_whitespace().collect::<Vec<_>>().join(" ");
                if one_line.chars().count() > 120 {
                    format!("query: {}…", one_line.chars().take(120).collect::<String>())
                } else {
                    format!("query: {one_line}")
                }
            }
            Request::Cancel { query_id } => format!("cancel {query_id}"),
        }
    }
}

#[derive(Debug)]
pub enum Payload {
    Databases(Vec<String>),
    Objects(Vec<SchemaObject>),
    AllObjects(Vec<(String, SchemaObject)>),
    Columns(Vec<Column>),
    Ddl(String),
    Query(QueryResult),
    Cancelled,
}

#[derive(Debug)]
pub struct Response {
    pub conn_id: String,
    pub conn_name: String,
    pub request: Request,
    pub result: Result<Payload, String>,
    pub log: DebugLog,
    pub elapsed: Duration,
}

pub struct Worker {
    tx: Sender<Response>,
    pub rx: Receiver<Response>,
    repaint: egui::Context,
}

impl Worker {
    pub fn new(repaint: egui::Context) -> Self {
        let (tx, rx) = channel();
        Self { tx, rx, repaint }
    }

    pub fn submit(&self, cfg: &ConnectionConfig, timeout: Duration, request: Request) {
        let tx = self.tx.clone();
        let ctx = self.repaint.clone();
        let cfg = cfg.clone();
        std::thread::spawn(move || {
            let started = std::time::Instant::now();
            let mut log = DebugLog::new();
            let cfg = cfg.with_secret().unwrap_or_else(|e| {
                log.push(e);
                cfg.clone()
            });
            let driver = cfg.kind.connect(&cfg, timeout);
            let result = match &request {
                Request::Databases => driver.databases(&mut log).map(Payload::Databases),
                Request::Objects { db } => driver.objects(db, &mut log).map(Payload::Objects),
                Request::AllObjects => driver.all_objects(&mut log).map(Payload::AllObjects),
                Request::Columns { db, object } => driver.columns(db, object, &mut log).map(Payload::Columns),
                Request::ObjectDdl { db, object } => driver.object_ddl(db, object, &mut log).map(Payload::Ddl),
                Request::DatabaseDdl { db } => driver.database_ddl(db, &mut log).map(Payload::Ddl),
                Request::Query { sql, database, query_id, max_rows } => {
                    driver.query(sql, database.as_deref(), query_id, *max_rows, &mut log).map(Payload::Query)
                }
                Request::Cancel { query_id } => driver.cancel(query_id, &mut log).map(|_| Payload::Cancelled),
            };
            let _ = tx.send(Response {
                conn_id: cfg.id.clone(),
                conn_name: cfg.display_name(),
                request,
                result,
                log,
                elapsed: started.elapsed(),
            });
            ctx.request_repaint();
        });
    }
}
