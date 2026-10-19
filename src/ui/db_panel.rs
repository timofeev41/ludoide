//! Right-hand Databases panel: connections, schema browser, connection editor.

use super::{dim, panel_frame, section_header, small_icon_button};
use crate::app::{App, Confirm};
use crate::db::worker::{Payload, Request, Response, Worker};
use crate::db::{self, Column, ConnectionConfig, DriverKind, ObjectKind, QueryResult, SchemaObject};
use crate::editor::Document;
use crate::fonts::icon;
use crate::languages::LangId;
use egui::{CollapsingHeader, RichText};
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

pub enum Loadable<T> {
    Loading,
    Ready(T),
    Failed(String),
}

#[derive(Default)]
pub struct ConnTree {
    pub databases: Option<Loadable<Vec<String>>>,
    pub objects: HashMap<String, Loadable<Vec<SchemaObject>>>,
    pub columns: HashMap<(String, String), Loadable<Vec<Column>>>,
}

pub struct LogEntry {
    pub at: chrono::DateTime<chrono::Local>,
    pub conn: String,
    pub what: String,
    pub ok: bool,
    pub elapsed: Duration,
    pub lines: Vec<String>,
}

pub struct Running {
    pub conn_id: String,
    pub query_id: String,
    pub sql: String,
    pub started: Instant,
}

pub struct ResultView {
    pub sql: String,
    pub conn: String,
    pub outcome: Result<QueryResult, String>,
    pub index: usize,
    pub total: usize,
}

pub struct ConnEditor {
    pub cfg: ConnectionConfig,
    pub is_new: bool,
    pub error: Option<String>,
    pub test: Option<Loadable<String>>,
}

pub struct DbState {
    pub worker: Worker,
    pub trees: HashMap<String, ConnTree>,
    pub filter: String,
    pub log: Vec<LogEntry>,
    pub result: Option<ResultView>,
    pub running: Option<Running>,
    queue: VecDeque<String>,
    script_total: usize,
    script_cfg: Option<(ConnectionConfig, Duration, usize)>,
    pub editing: Option<ConnEditor>,
    /// Database used for queries per connection (overrides the connection default).
    pub current_db: HashMap<String, String>,
    test_rx: Option<std::sync::mpsc::Receiver<Result<String, String>>>,
}

impl DbState {
    pub fn new(worker: Worker) -> Self {
        Self {
            worker,
            trees: HashMap::new(),
            filter: String::new(),
            log: Vec::new(),
            result: None,
            running: None,
            queue: VecDeque::new(),
            script_total: 0,
            script_cfg: None,
            editing: None,
            current_db: HashMap::new(),
            test_rx: None,
        }
    }

    pub fn submit(&mut self, cfg: &ConnectionConfig, timeout: Duration, req: Request) {
        let tree = self.trees.entry(cfg.id.clone()).or_default();
        match &req {
            Request::Databases => tree.databases = Some(Loadable::Loading),
            Request::Objects { db } => {
                tree.objects.insert(db.clone(), Loadable::Loading);
            }
            Request::Columns { db, object } => {
                tree.columns.insert((db.clone(), object.clone()), Loadable::Loading);
            }
            _ => {}
        }
        self.worker.submit(cfg, timeout, req);
    }

    pub fn start_script(
        &mut self,
        cfg: &ConnectionConfig,
        timeout: Duration,
        statements: Vec<String>,
        max_rows: usize,
    ) {
        self.script_total = statements.len();
        self.queue = statements.into();
        self.script_cfg = Some((cfg.clone(), timeout, max_rows));
        self.next_statement();
    }

    fn next_statement(&mut self) {
        let Some((cfg, timeout, max_rows)) = self.script_cfg.clone() else { return };
        let Some(sql) = self.queue.pop_front() else {
            self.script_cfg = None;
            return;
        };
        let query_id = format!("ludo-{}", db::new_id());
        let database = self.current_db.get(&cfg.id).cloned();
        self.running = Some(Running {
            conn_id: cfg.id.clone(),
            query_id: query_id.clone(),
            sql: sql.clone(),
            started: Instant::now(),
        });
        self.worker.submit(&cfg, timeout, Request::Query { sql, database, query_id, max_rows });
    }

    pub fn cancel(&mut self, cfg: &ConnectionConfig, timeout: Duration) {
        if let Some(r) = &self.running {
            self.queue.clear();
            self.worker.submit(cfg, timeout, Request::Cancel { query_id: r.query_id.clone() });
        }
    }

    fn database_for(&self, cfg: &ConnectionConfig) -> String {
        self.current_db.get(&cfg.id).cloned().unwrap_or_else(|| cfg.database.clone())
    }
}

enum DbAction {
    Activate(String),
    Edit(String),
    Delete(String),
    Refresh(String),
    Load(String, Request),
    Inspect(String, String, String),
    ObjectDdl(String, String, SchemaObject),
    DatabaseDdl(String, String),
    UseDatabase(String, String),
    NewConsole(String),
    Copy(String),
}

fn matches(filter: &str, s: &str) -> bool {
    filter.is_empty() || s.to_lowercase().contains(filter)
}

fn kind_icon(k: ObjectKind) -> &'static str {
    match k {
        ObjectKind::Table => icon::TABLE,
        ObjectKind::View => icon::EYE,
        ObjectKind::MaterializedView => icon::LIGHTNING,
        ObjectKind::Dictionary => icon::BOOKMARKS,
        ObjectKind::Index => icon::KEY,
    }
}

impl App {
    pub fn db_panel(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        let mut actions = Vec::new();
        egui::Panel::right("db_panel")
            .resizable(true)
            .default_size(300.0)
            .size_range(200.0..=700.0)
            .frame(panel_frame(p.panel, 8))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    section_header(ui, "Databases", &p);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if small_icon_button(ui, icon::PLUS, "Add connection", &p).clicked() {
                            self.db.editing = Some(ConnEditor {
                                cfg: ConnectionConfig::default(),
                                is_new: true,
                                error: None,
                                test: None,
                            });
                        }
                    });
                });
                ui.add(
                    egui::TextEdit::singleline(&mut self.db.filter)
                        .hint_text(format!("{} Filter databases, tables, views…", icon::FUNNEL))
                        .desired_width(f32::INFINITY),
                );
                ui.add_space(4.0);
                let filter = self.db.filter.to_lowercase();
                let active = self.settings.coding.active_connection.clone();
                let conns = self.settings.coding.connections.clone();
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    if conns.is_empty() {
                        ui.label(dim("No connections yet.", &p));
                        if ui.button(format!("{} Add ClickHouse connection", icon::PLUS)).clicked() {
                            self.db.editing = Some(ConnEditor {
                                cfg: ConnectionConfig::default(),
                                is_new: true,
                                error: None,
                                test: None,
                            });
                        }
                    }
                    for cfg in &conns {
                        self.connection_node(ui, cfg, active.as_deref() == Some(&cfg.id), &filter, &mut actions);
                    }
                });
            });
        self.apply_db_actions(actions);
    }

    fn connection_node(
        &mut self,
        ui: &mut egui::Ui,
        cfg: &ConnectionConfig,
        active: bool,
        filter: &str,
        actions: &mut Vec<DbAction>,
    ) {
        let p = self.palette;
        let tree = self.db.trees.entry(cfg.id.clone()).or_default();
        let dot = match &tree.databases {
            Some(Loadable::Ready(_)) => p.success,
            Some(Loadable::Failed(_)) => p.error,
            Some(Loadable::Loading) => p.warning,
            None => p.text_dim,
        };
        let mut title = RichText::new(format!("{} {}", icon::DATABASE, cfg.display_name()));
        if active {
            title = title.strong().color(p.accent);
        }
        let id = cfg.id.clone();
        let current_db = self.db.database_for(cfg);
        let header = CollapsingHeader::new(title).id_salt(("conn", &cfg.id)).show(ui, |ui| {
            enum Snap {
                Load,
                Loading,
                Failed(String),
                Ready(Vec<(String, bool)>),
            }
            let tree = self.db.trees.entry(id.clone()).or_default();
            let snap = match &tree.databases {
                None => Snap::Load,
                Some(Loadable::Loading) => Snap::Loading,
                Some(Loadable::Failed(e)) => Snap::Failed(e.clone()),
                Some(Loadable::Ready(dbs)) => Snap::Ready(
                    dbs.iter()
                        .map(|d| {
                            let m = match tree.objects.get(d) {
                                Some(Loadable::Ready(objs)) => objs.iter().any(|o| matches(filter, &o.name)),
                                _ => false,
                            };
                            (d.clone(), m)
                        })
                        .collect(),
                ),
            };
            match snap {
                Snap::Load => actions.push(DbAction::Load(id.clone(), Request::Databases)),
                Snap::Loading => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(dim("connecting…", &p));
                    });
                }
                Snap::Failed(e) => {
                    ui.label(RichText::new(e).color(p.error).small());
                    if ui.small_button("Retry").clicked() {
                        actions.push(DbAction::Refresh(id.clone()));
                    }
                }
                Snap::Ready(dbs) => {
                    for (dbname, objects_match) in dbs {
                        if !matches(filter, &dbname) && !objects_match {
                            continue;
                        }
                        let filter_objects = if matches(filter, &dbname) { "" } else { filter };
                        let mut label = RichText::new(format!("{} {dbname}", icon::STACK));
                        if dbname == current_db {
                            label = label.strong();
                        }
                        let r = CollapsingHeader::new(label)
                            .id_salt(("db", &id, &dbname))
                            .open(if filter_objects.is_empty() { None } else { Some(true) })
                            .show(ui, |ui| self.database_node(ui, &id, &dbname, filter_objects, actions));
                        r.header_response.context_menu(|ui| {
                            if ui.button("Use for queries").clicked() {
                                actions.push(DbAction::UseDatabase(id.clone(), dbname.clone()));
                            }
                            if ui.button("Show database DDL").clicked() {
                                actions.push(DbAction::DatabaseDdl(id.clone(), dbname.clone()));
                            }
                            if ui.button("Refresh").clicked() {
                                actions.push(DbAction::Load(id.clone(), Request::Objects { db: dbname.clone() }));
                            }
                            if ui.button("Copy name").clicked() {
                                actions.push(DbAction::Copy(dbname.clone()));
                            }
                        });
                    }
                }
            }
        });
        let hr = header.header_response;
        ui.painter().circle_filled(egui::pos2(ui.max_rect().right() - 6.0, hr.rect.center().y), 3.5, dot);
        if hr.double_clicked() {
            actions.push(DbAction::Activate(cfg.id.clone()));
        }
        hr.on_hover_text(format!("{}\nuser: {}\nDouble-click to make active", cfg.url, cfg.username)).context_menu(
            |ui| {
                if ui.button("Make active").clicked() {
                    actions.push(DbAction::Activate(cfg.id.clone()));
                }
                if ui.button("New SQL console").clicked() {
                    actions.push(DbAction::NewConsole(cfg.id.clone()));
                }
                if ui.button("Refresh").clicked() {
                    actions.push(DbAction::Refresh(cfg.id.clone()));
                }
                if ui.button("Edit…").clicked() {
                    actions.push(DbAction::Edit(cfg.id.clone()));
                }
                ui.separator();
                if ui.button(RichText::new("Remove").color(p.error)).clicked() {
                    actions.push(DbAction::Delete(cfg.id.clone()));
                }
            },
        );
    }

    fn database_node(
        &mut self,
        ui: &mut egui::Ui,
        conn: &str,
        dbname: &str,
        filter: &str,
        actions: &mut Vec<DbAction>,
    ) {
        let p = self.palette;
        let tree = self.db.trees.entry(conn.to_owned()).or_default();
        let objs = match tree.objects.get(dbname) {
            None => {
                actions.push(DbAction::Load(conn.to_owned(), Request::Objects { db: dbname.to_owned() }));
                return;
            }
            Some(Loadable::Loading) => {
                ui.spinner();
                return;
            }
            Some(Loadable::Failed(e)) => {
                ui.label(RichText::new(e).color(p.error).small());
                return;
            }
            Some(Loadable::Ready(objs)) => objs.clone(),
        };
        if objs.is_empty() {
            ui.label(dim("no objects", &p).small());
        }
        for kind in ObjectKind::GROUPS {
            let group: Vec<&SchemaObject> =
                objs.iter().filter(|o| o.kind == *kind && matches(filter, &o.name)).collect();
            if group.is_empty() {
                continue;
            }
            CollapsingHeader::new(dim(format!("{} ({})", kind.group_name(), group.len()), &p))
                .id_salt(("grp", conn, dbname, kind.group_name()))
                .default_open(*kind == ObjectKind::Table || !filter.is_empty())
                .show(ui, |ui| {
                    for obj in group {
                        self.object_node(ui, conn, dbname, obj, actions);
                    }
                });
        }
    }

    fn object_node(
        &mut self,
        ui: &mut egui::Ui,
        conn: &str,
        dbname: &str,
        obj: &SchemaObject,
        actions: &mut Vec<DbAction>,
    ) {
        let p = self.palette;
        let mut meta = Vec::new();
        if let Some(r) = obj.rows {
            meta.push(format!("{} rows", db::human_count(r)));
        }
        if let Some(b) = obj.bytes.filter(|b| *b > 0) {
            meta.push(db::human_bytes(b));
        }
        let name = match &obj.parent {
            Some(parent) => format!("{} {}  ({parent})", kind_icon(obj.kind), obj.name),
            None => format!("{} {}", kind_icon(obj.kind), obj.name),
        };
        let tooltip = format!("{}\n{}", obj.detail, meta.join(" · "));
        let response = if obj.kind.queryable() {
            let key = (dbname.to_owned(), obj.name.clone());
            let hr = CollapsingHeader::new(name)
                .id_salt(("obj", conn, dbname, &obj.name))
                .show(ui, |ui| {
                    let tree = self.db.trees.entry(conn.to_owned()).or_default();
                    match tree.columns.get(&key) {
                        None => actions.push(DbAction::Load(
                            conn.to_owned(),
                            Request::Columns { db: dbname.to_owned(), object: obj.name.clone() },
                        )),
                        Some(Loadable::Loading) => {
                            ui.spinner();
                        }
                        Some(Loadable::Failed(e)) => {
                            ui.label(RichText::new(e).color(p.error).small());
                        }
                        Some(Loadable::Ready(cols)) => {
                            for c in cols {
                                let r = ui.horizontal(|ui| {
                                    ui.label(RichText::new(icon::COLUMNS).color(p.text_dim));
                                    ui.label(&c.name);
                                    ui.label(dim(&c.data_type, &p).small());
                                });
                                let r = r.response.interact(egui::Sense::click());
                                let r = if c.detail.is_empty() { r } else { r.on_hover_text(&c.detail) };
                                r.context_menu(|ui| {
                                    if ui.button("Copy name").clicked() {
                                        actions.push(DbAction::Copy(c.name.clone()));
                                    }
                                });
                            }
                        }
                    }
                })
                .header_response;
            hr
        } else {
            ui.add(egui::Label::new(name).sense(egui::Sense::click()))
        };
        if response.double_clicked() && obj.kind.queryable() {
            actions.push(DbAction::Inspect(conn.to_owned(), dbname.to_owned(), obj.name.clone()));
        }
        response.on_hover_text(tooltip).context_menu(|ui| {
            if obj.kind.queryable() {
                let limit = self.settings.coding.inspect_limit;
                if ui.button(format!("{} Quick inspect (LIMIT {limit})", icon::PLAY)).clicked() {
                    actions.push(DbAction::Inspect(conn.to_owned(), dbname.to_owned(), obj.name.clone()));
                }
            }
            if ui.button(format!("{} Show CREATE statement", icon::CODE)).clicked() {
                actions.push(DbAction::ObjectDdl(conn.to_owned(), dbname.to_owned(), obj.clone()));
            }
            if ui.button("Copy qualified name").clicked() {
                actions.push(DbAction::Copy(format!("{dbname}.{}", obj.name)));
            }
        });
    }

    fn apply_db_actions(&mut self, actions: Vec<DbAction>) {
        for a in actions {
            match a {
                DbAction::Activate(id) => self.settings.coding.active_connection = Some(id),
                DbAction::Edit(id) => {
                    if let Some(cfg) = self.connection(&id).cloned() {
                        self.db.editing = Some(ConnEditor { cfg, is_new: false, error: None, test: None });
                    }
                }
                DbAction::Delete(id) => self.confirm = Some(Confirm::DeleteConnection(id)),
                DbAction::Refresh(id) => {
                    self.db.trees.remove(&id);
                    self.db_request(&id, Request::Databases);
                }
                DbAction::Load(id, req) => self.db_request(&id, req),
                DbAction::Inspect(id, dbname, obj) => {
                    let Some(cfg) = self.connection(&id).cloned() else { continue };
                    let sql = cfg.kind.connect(&cfg, self.timeout()).inspect_sql(
                        &dbname,
                        &obj,
                        self.settings.coding.inspect_limit,
                    );
                    self.settings.coding.active_connection = Some(id);
                    self.run_sql(vec![sql]);
                }
                DbAction::ObjectDdl(id, dbname, obj) => {
                    self.db_request(&id, Request::ObjectDdl { db: dbname, object: obj })
                }
                DbAction::DatabaseDdl(id, dbname) => self.db_request(&id, Request::DatabaseDdl { db: dbname }),
                DbAction::UseDatabase(id, dbname) => {
                    self.db.current_db.insert(id.clone(), dbname.clone());
                    self.settings.coding.active_connection = Some(id);
                    self.notify(format!("Queries now run in database '{dbname}'"));
                }
                DbAction::NewConsole(id) => {
                    self.settings.coding.active_connection = Some(id);
                    let dir = self.settings.other.scratches_dir.clone();
                    self.new_scratch(&dir, "sql");
                }
                DbAction::Copy(text) => self.copy_to_clipboard(text),
            }
        }
    }

    pub fn copy_to_clipboard(&mut self, text: String) {
        self.pending_copy = Some(text);
    }

    /// Route finished background requests to the tree, results grid and log.
    pub fn poll_db(&mut self) {
        while let Ok(resp) = self.db.worker.rx.try_recv() {
            self.handle_db_response(resp);
        }
        if let Ok(r) = self.db.test_rx_try() {
            if let Some(ed) = &mut self.db.editing {
                ed.test = Some(match r {
                    Ok(v) => Loadable::Ready(v),
                    Err(e) => Loadable::Failed(e),
                });
            }
        }
    }

    fn handle_db_response(&mut self, resp: Response) {
        let Response { conn_id, conn_name, request, result, log, elapsed } = resp;
        self.db.log.push(LogEntry {
            at: chrono::Local::now(),
            conn: conn_name.clone(),
            what: request.describe(),
            ok: result.is_ok(),
            elapsed,
            lines: log,
        });
        if self.db.log.len() > 500 {
            self.db.log.drain(..100);
        }
        let tree = self.db.trees.entry(conn_id.clone()).or_default();
        match (request, result) {
            (Request::Databases, r) => {
                tree.databases = Some(match r {
                    Ok(Payload::Databases(d)) => Loadable::Ready(d),
                    Ok(_) => Loadable::Failed("unexpected".into()),
                    Err(e) => Loadable::Failed(e),
                })
            }
            (Request::Objects { db }, r) => {
                tree.objects.insert(
                    db,
                    match r {
                        Ok(Payload::Objects(o)) => Loadable::Ready(o),
                        Ok(_) => Loadable::Failed("unexpected".into()),
                        Err(e) => Loadable::Failed(e),
                    },
                );
            }
            (Request::Columns { db, object }, r) => {
                tree.columns.insert(
                    (db, object),
                    match r {
                        Ok(Payload::Columns(c)) => Loadable::Ready(c),
                        Ok(_) => Loadable::Failed("unexpected".into()),
                        Err(e) => Loadable::Failed(e),
                    },
                );
            }
            (Request::ObjectDdl { db, object }, r) => match r {
                Ok(Payload::Ddl(text)) => {
                    let doc = Document::scratch(format!("{db}.{}.sql", object.name), text, LangId::Sql);
                    self.open_document(doc);
                }
                Err(e) => self.error(e),
                _ => {}
            },
            (Request::DatabaseDdl { db }, r) => match r {
                Ok(Payload::Ddl(text)) => {
                    let doc = Document::scratch(format!("{db}.schema.sql"), text, LangId::Sql);
                    self.open_document(doc);
                }
                Err(e) => self.error(e),
                _ => {}
            },
            (Request::Query { sql, query_id, .. }, r) => {
                if self.db.running.as_ref().is_some_and(|run| run.query_id == query_id) {
                    self.db.running = None;
                }
                let outcome = match r {
                    Ok(Payload::Query(q)) => Ok(q),
                    Ok(_) => Err("unexpected response".into()),
                    Err(e) => Err(e),
                };
                let failed = outcome.is_err();
                let index = self.db.script_total - self.db.queue.len();
                self.db.result = Some(ResultView { sql, conn: conn_name, outcome, index, total: self.db.script_total });
                if failed {
                    self.db.queue.clear();
                }
                self.db.next_statement();
            }
            (Request::Cancel { .. }, r) => match r {
                Ok(_) => self.notify("Cancellation requested"),
                Err(e) => self.error(format!("Cancel failed: {e}")),
            },
        }
    }

    pub fn connection_editor(&mut self, ctx: &egui::Context) {
        let Some(ed) = &mut self.db.editing else { return };
        let p = self.palette;
        let mut save = false;
        let mut cancel = false;
        let mut test = false;
        egui::Modal::new(egui::Id::unique("conn_editor")).show(ctx, |ui| {
            ui.set_width(440.0);
            ui.heading(if ed.is_new { "New connection" } else { "Edit connection" });
            ui.add_space(8.0);
            egui::Grid::new("conn_grid").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
                ui.label("Type");
                egui::ComboBox::from_id_salt("conn_kind").selected_text(ed.cfg.kind.name()).show_ui(ui, |ui| {
                    for k in DriverKind::ALL {
                        ui.selectable_value(&mut ed.cfg.kind, *k, k.name());
                    }
                });
                ui.end_row();
                ui.label("Name");
                ui.add(egui::TextEdit::singleline(&mut ed.cfg.name).hint_text("optional").desired_width(300.0));
                ui.end_row();
                ui.label("URL");
                ui.add(
                    egui::TextEdit::singleline(&mut ed.cfg.url)
                        .hint_text(ed.cfg.kind.default_url())
                        .desired_width(300.0),
                );
                ui.end_row();
                ui.label("User");
                ui.add(egui::TextEdit::singleline(&mut ed.cfg.username).desired_width(300.0));
                ui.end_row();
                ui.label("Password");
                ui.add(egui::TextEdit::singleline(&mut ed.cfg.password).password(true).desired_width(300.0));
                ui.end_row();
                ui.label("Database");
                ui.add(egui::TextEdit::singleline(&mut ed.cfg.database).hint_text("default").desired_width(300.0));
                ui.end_row();
            });
            ui.add_space(6.0);
            match &ed.test {
                Some(Loadable::Loading) => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Testing…");
                    });
                }
                Some(Loadable::Ready(v)) => {
                    ui.label(RichText::new(format!("{} Connected: {v}", icon::CHECK_CIRCLE)).color(p.success));
                }
                Some(Loadable::Failed(e)) => {
                    ui.label(RichText::new(format!("{} {e}", icon::X_CIRCLE)).color(p.error));
                }
                None => {}
            }
            if let Some(e) = &ed.error {
                ui.label(RichText::new(e).color(p.error));
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(format!("{} Test", icon::PLUG)).clicked() {
                    test = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        cancel = true;
                    }
                    if ui.button(RichText::new("Save").strong()).clicked() {
                        save = true;
                    }
                });
            });
        });
        if test {
            let cfg = ed.cfg.clone();
            ed.test = Some(Loadable::Loading);
            self.db.test_connection(cfg, Duration::from_secs(10), ctx.clone());
        }
        if cancel {
            self.db.editing = None;
            return;
        }
        if save {
            let ed = self.db.editing.as_mut().unwrap();
            if ed.cfg.url.trim().is_empty() || ed.cfg.username.trim().is_empty() {
                ed.error = Some("URL and user are required".into());
                return;
            }
            if let Err(e) = db::check_unique(&self.settings.coding.connections, &ed.cfg) {
                ed.error = Some(e);
                return;
            }
            let ed = self.db.editing.take().unwrap();
            let id = ed.cfg.id.clone();
            match self.settings.coding.connections.iter_mut().find(|c| c.id == id) {
                Some(existing) => *existing = ed.cfg,
                None => self.settings.coding.connections.push(ed.cfg),
            }
            if self.settings.coding.active_connection.is_none() {
                self.settings.coding.active_connection = Some(id.clone());
            }
            self.db.trees.remove(&id);
        }
    }
}

// Connection tests bypass the worker so they never touch the tree state.
impl DbState {
    fn test_connection(&mut self, cfg: ConnectionConfig, timeout: Duration, ctx: egui::Context) {
        let (tx, rx) = std::sync::mpsc::channel();
        self.test_rx = Some(rx);
        std::thread::spawn(move || {
            let mut log = Vec::new();
            let r = cfg.kind.connect(&cfg, timeout).ping(&mut log);
            let _ = tx.send(r);
            ctx.request_repaint();
        });
    }

    fn test_rx_try(&mut self) -> Result<Result<String, String>, ()> {
        match &self.test_rx {
            Some(rx) => rx.try_recv().map_err(|_| ()),
            None => Err(()),
        }
    }
}
