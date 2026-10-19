//! Right-hand Databases panel: connections, schema browser, connection editor.

use super::{dim, panel_frame, section_header, small_icon_button};
use crate::app::{App, Confirm};
use crate::db::search::{ObjectFilter, Search};
use crate::db::worker::{Payload, Request, Response, Worker};
use crate::db::{self, Column, ConnectionConfig, DriverKind, ObjectKind, QueryResult, SchemaObject};
use crate::editor::Document;
use crate::fonts::icon;
use crate::languages::LangId;
use crate::secrets;
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
    /// All objects of all databases loaded at once (for search).
    pub index: Option<Loadable<()>>,
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
    /// Newly typed password; empty keeps the one in the Keychain.
    pub password: String,
    /// Remove the saved password on save.
    pub forget: bool,
    /// Filter for the visible-databases checklist.
    pub db_search: String,
    /// "Only selected" databases mode (vs all).
    pub pick_databases: bool,
}

impl ConnEditor {
    pub fn new(mut cfg: ConnectionConfig, is_new: bool) -> Self {
        // A plain-text password (legacy settings) is moved to the Keychain on save.
        let password = std::mem::take(&mut cfg.password);
        let pick_databases = !cfg.visible_databases.is_empty();
        Self { cfg, is_new, error: None, test: None, password, forget: false, db_search: String::new(), pick_databases }
    }

    /// Config to test with: typed password, else the saved one (resolved off-thread).
    fn test_config(&self) -> ConnectionConfig {
        let mut cfg = self.cfg.clone();
        cfg.password = self.password.clone();
        if self.forget {
            cfg.keychain_password = false;
        }
        cfg
    }
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
            Request::AllObjects => tree.index = Some(Loadable::Loading),
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
    SetVisible(String, String, bool),
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
                            self.db.editing = Some(ConnEditor::new(ConnectionConfig::default(), true));
                        }
                    });
                });
                ui.add(
                    egui::TextEdit::singleline(&mut self.db.filter)
                        .hint_text(format!("{} Search: ad|timofeev, db.table", icon::MAGNIFYING_GLASS))
                        .desired_width(f32::INFINITY),
                )
                .on_hover_text(
                    "Case-insensitive search across databases and tables.\n\
                     • ad|timofeev — databases containing either word (a space works too)\n\
                     • events — if no database contains it, tables containing \"events\"\n\
                     • ad.events — tables with \"events\" in databases with \"ad\"\n\
                     • ad. — databases only   • .events — tables only\n\
                     Hidden databases are searched too and shown dimmed.",
                );
                ui.add_space(4.0);
                let search = Search::parse(&self.db.filter);
                let active = self.settings.coding.active_connection.clone();
                let conns = self.settings.coding.connections.clone();
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    if conns.is_empty() {
                        ui.label(dim("No connections yet.", &p));
                        if ui.button(format!("{} Add ClickHouse connection", icon::PLUS)).clicked() {
                            self.db.editing = Some(ConnEditor::new(ConnectionConfig::default(), true));
                        }
                    }
                    for cfg in &conns {
                        self.connection_node(ui, cfg, active.as_deref() == Some(&cfg.id), &search, &mut actions);
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
        search: &Search,
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
        let header = CollapsingHeader::new(title)
            .id_salt(("conn", &cfg.id, !search.is_empty()))
            .open(if search.is_empty() { None } else { Some(true) })
            .show(ui, |ui| {
                enum Snap {
                    Load,
                    Loading,
                    Failed(String),
                    Ready(Vec<String>),
                }
                let tree = self.db.trees.entry(id.clone()).or_default();
                let snap = match &tree.databases {
                    None => Snap::Load,
                    Some(Loadable::Loading) => Snap::Loading,
                    Some(Loadable::Failed(e)) => Snap::Failed(e.clone()),
                    Some(Loadable::Ready(dbs)) => Snap::Ready(dbs.clone()),
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
                        let resolved = search.resolve(&dbs);
                        let search = &resolved;
                        // Searching for tables: index every object once, in one query.
                        if !search.is_empty() && search.wants_objects() && tree.index.is_none() {
                            actions.push(DbAction::Load(id.clone(), Request::AllObjects));
                        }
                        if matches!(tree.index, Some(Loadable::Loading)) {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(dim("indexing tables…", &p).small());
                            });
                        }
                        let visible: Vec<&String> = dbs
                            .iter()
                            .filter(|d| cfg.visible_databases.is_empty() || cfg.visible_databases.contains(d))
                            .collect();
                        if !cfg.visible_databases.is_empty() && search.is_empty() {
                            let r = ui.add(
                                egui::Label::new(
                                    dim(format!("showing {} of {} databases", visible.len(), dbs.len()), &p).small(),
                                )
                                .sense(egui::Sense::click()),
                            );
                            if r.on_hover_text("Choose databases in Edit…").clicked() {
                                actions.push(DbAction::Edit(id.clone()));
                            }
                        }
                        let mut shown = 0;
                        for dbname in &visible {
                            if self.database_row(ui, cfg, dbname, &current_db, search, false, actions) {
                                shown += 1;
                            }
                        }
                        if !search.is_empty() {
                            // Hidden databases are still searchable, shown dimmed below.
                            let hidden: Vec<&String> = dbs.iter().filter(|d| !visible.contains(d)).collect();
                            let mut first = true;
                            for dbname in hidden {
                                if self.would_show(cfg, dbname, search) {
                                    if first {
                                        ui.label(dim(format!("{} hidden", icon::EYE_SLASH), &p).small());
                                        first = false;
                                    }
                                    self.database_row(ui, cfg, dbname, &current_db, search, true, actions);
                                    shown += 1;
                                }
                            }
                            if shown == 0 {
                                ui.label(dim("no matches", &p).small());
                            }
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
                if ui.button("Edit… (connection, visible databases)").clicked() {
                    actions.push(DbAction::Edit(cfg.id.clone()));
                }
                ui.separator();
                if ui.button(RichText::new("Remove").color(p.error)).clicked() {
                    actions.push(DbAction::Delete(cfg.id.clone()));
                }
            },
        );
    }

    /// Whether `db` has something to show for the current search.
    fn would_show(&self, cfg: &ConnectionConfig, db: &str, search: &Search) -> bool {
        if search.is_empty() || search.db_matches(db) {
            return true;
        }
        let f = search.objects(db);
        match self.db.trees.get(&cfg.id).and_then(|t| t.objects.get(db)) {
            Some(Loadable::Ready(objs)) => objs.iter().any(|o| f.matches(&o.name)),
            _ => false,
        }
    }

    /// One database row; returns whether it was shown.
    #[allow(clippy::too_many_arguments)]
    fn database_row(
        &mut self,
        ui: &mut egui::Ui,
        cfg: &ConnectionConfig,
        dbname: &str,
        current_db: &str,
        search: &Search,
        hidden: bool,
        actions: &mut Vec<DbAction>,
    ) -> bool {
        let p = self.palette;
        if !self.would_show(cfg, dbname, search) {
            return false;
        }
        let id = &cfg.id;
        let filter = if search.db_matches(dbname) { ObjectFilter::All } else { search.objects(dbname) };
        let mut label = RichText::new(format!("{} {dbname}", icon::STACK));
        if dbname == current_db {
            label = label.strong();
        }
        if hidden {
            label = label.color(p.text_dim);
        }
        let narrowing = filter != ObjectFilter::All;
        let r = CollapsingHeader::new(label)
            .id_salt(("db", id, dbname, !search.is_empty()))
            .open(if narrowing { Some(true) } else { None })
            .show(ui, |ui| self.database_node(ui, id, dbname, &filter, actions));
        r.header_response.context_menu(|ui| {
            if ui.button("Use for queries").clicked() {
                actions.push(DbAction::UseDatabase(id.clone(), dbname.to_owned()));
            }
            if ui.button("Show database DDL").clicked() {
                actions.push(DbAction::DatabaseDdl(id.clone(), dbname.to_owned()));
            }
            if ui.button("Refresh").clicked() {
                actions.push(DbAction::Load(id.clone(), Request::Objects { db: dbname.to_owned() }));
            }
            if ui.button("Copy name").clicked() {
                actions.push(DbAction::Copy(dbname.to_owned()));
            }
            ui.separator();
            if hidden {
                if ui.button("Show in tree").clicked() {
                    actions.push(DbAction::SetVisible(id.clone(), dbname.to_owned(), true));
                }
            } else if ui.button("Hide from tree").clicked() {
                actions.push(DbAction::SetVisible(id.clone(), dbname.to_owned(), false));
            }
        });
        true
    }

    fn database_node(
        &mut self,
        ui: &mut egui::Ui,
        conn: &str,
        dbname: &str,
        filter: &ObjectFilter,
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
        let narrowing = *filter != ObjectFilter::All;
        for kind in ObjectKind::GROUPS {
            let group: Vec<&SchemaObject> =
                objs.iter().filter(|o| o.kind == *kind && filter.matches(&o.name)).collect();
            if group.is_empty() {
                continue;
            }
            CollapsingHeader::new(dim(format!("{} ({})", kind.group_name(), group.len()), &p))
                .id_salt(("grp", conn, dbname, kind.group_name(), narrowing))
                .open(if narrowing { Some(true) } else { None })
                .default_open(*kind == ObjectKind::Table)
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
                        self.db.editing = Some(ConnEditor::new(cfg, false));
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
                DbAction::SetVisible(id, dbname, show) => {
                    let all = match self.db.trees.get(&id).and_then(|t| t.databases.as_ref()) {
                        Some(Loadable::Ready(d)) => d.clone(),
                        _ => Vec::new(),
                    };
                    if let Some(c) = self.settings.coding.connections.iter_mut().find(|c| c.id == id) {
                        if c.visible_databases.is_empty() {
                            c.visible_databases = all;
                        }
                        c.visible_databases.retain(|d| d != &dbname);
                        if show {
                            c.visible_databases.push(dbname);
                            c.visible_databases.sort();
                        }
                    }
                }
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
            (Request::AllObjects, r) => match r {
                Ok(Payload::AllObjects(all)) => {
                    let mut by_db: HashMap<String, Vec<SchemaObject>> = HashMap::new();
                    for (d, o) in all {
                        by_db.entry(d).or_default().push(o);
                    }
                    for (d, objs) in by_db {
                        tree.objects.insert(d, Loadable::Ready(objs));
                    }
                    tree.index = Some(Loadable::Ready(()));
                }
                Ok(_) => tree.index = Some(Loadable::Failed("unexpected".into())),
                Err(e) => tree.index = Some(Loadable::Failed(e)),
            },
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
        let mut load_dbs = false;
        let DbState { editing, trees, .. } = &mut self.db;
        let Some(ed) = editing else { return };
        let known_dbs: Option<Vec<String>> = match trees.get(&ed.cfg.id).and_then(|t| t.databases.as_ref()) {
            Some(Loadable::Ready(d)) => Some(d.clone()),
            _ => None,
        };
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
                let saved = ed.cfg.keychain_password && !ed.forget;
                ui.horizontal(|ui| {
                    let hint = if saved { "saved in Keychain — type to replace" } else { "" };
                    let w = if saved { 228.0 } else { 300.0 };
                    ui.add(
                        egui::TextEdit::singleline(&mut ed.password).password(true).hint_text(hint).desired_width(w),
                    );
                    if saved
                        && ui
                            .small_button("Forget")
                            .on_hover_text("Remove the saved password from the Keychain")
                            .clicked()
                    {
                        ed.forget = true;
                    }
                });
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
            ui.add_space(6.0);
            visible_databases_ui(ui, ed, known_dbs.as_deref(), &mut load_dbs, &p);
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
        if load_dbs && !ed.is_new {
            let id = ed.cfg.id.clone();
            self.db_request(&id, Request::Databases);
            return;
        }
        let Some(ed) = &mut self.db.editing else { return };
        if test {
            let cfg = ed.test_config();
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
            // Store the secret first so a Keychain failure leaves the dialog open.
            let id = ed.cfg.id.clone();
            if !ed.password.is_empty() {
                if let Err(e) = secrets::set(&id, &ed.password) {
                    ed.error = Some(format!("Couldn't save the password to the Keychain: {e}"));
                    return;
                }
                ed.cfg.keychain_password = true;
            } else if ed.forget {
                if let Err(e) = secrets::delete(&id) {
                    ed.error = Some(format!("Couldn't remove the password from the Keychain: {e}"));
                    return;
                }
                ed.cfg.keychain_password = false;
            }
            let ed = self.db.editing.take().unwrap();
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
            let r = cfg.with_secret().and_then(|cfg| cfg.kind.connect(&cfg, timeout).ping(&mut log));
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

/// "Databases in tree": all, or a hand-picked list with a searchable checklist.
fn visible_databases_ui(
    ui: &mut egui::Ui,
    ed: &mut ConnEditor,
    known: Option<&[String]>,
    load: &mut bool,
    p: &crate::theme::Palette,
) {
    ui.label(RichText::new("Databases in tree").strong());
    ui.horizontal(|ui| {
        if ui.radio(!ed.pick_databases, "All").clicked() {
            ed.cfg.visible_databases.clear();
            ed.pick_databases = false;
        }
        if ui.radio(ed.pick_databases, "Only selected").clicked() {
            ed.pick_databases = true;
        }
        if ed.pick_databases {
            let n = ed.cfg.visible_databases.len();
            let text = if n == 0 { "none selected = all".to_owned() } else { format!("{n} selected") };
            ui.label(dim(text, p).small());
        }
    });
    if !ed.pick_databases {
        return;
    }
    let Some(known) = known else {
        if ed.is_new {
            ui.label(dim("Save the connection first, then pick databases here.", p).small());
        } else if ui.button("Load database list").clicked() {
            *load = true;
        }
        return;
    };
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut ed.db_search).hint_text("filter: ad|timofeev").desired_width(220.0));
        let search = Search::parse(&ed.db_search);
        let matching: Vec<&String> = known.iter().filter(|d| search.is_empty() || search.db_matches(d)).collect();
        if ui.small_button(format!("Select {}", matching.len())).on_hover_text("Select all matching").clicked() {
            for d in &matching {
                if !ed.cfg.visible_databases.contains(d) {
                    ed.cfg.visible_databases.push((*d).clone());
                }
            }
            ed.cfg.visible_databases.sort();
        }
        if ui.small_button("Clear").clicked() {
            ed.cfg.visible_databases.clear();
        }
    });
    let search = Search::parse(&ed.db_search);
    egui::ScrollArea::vertical().max_height(180.0).auto_shrink([false, true]).show(ui, |ui| {
        for d in known.iter().filter(|d| search.is_empty() || search.db_matches(d)) {
            let mut on = ed.cfg.visible_databases.contains(d);
            if ui.checkbox(&mut on, d.as_str()).changed() {
                if on {
                    ed.cfg.visible_databases.push(d.clone());
                    ed.cfg.visible_databases.sort();
                } else {
                    ed.cfg.visible_databases.retain(|x| x != d);
                }
            }
        }
    });
}
