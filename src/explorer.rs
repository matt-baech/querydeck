//! Object Explorer: a lazily loaded tree of databases and their objects.

use std::collections::{HashMap, HashSet};

use iced::widget::{Column, Text, button, column, container, row, space, text};
use iced::{Alignment, Color, Element, Length, Theme, color};
use iced_aw::ContextMenu;

use crate::app::Message;
use crate::db::catalog::{ColumnInfo, DbObject, ObjectKind};
use crate::style::{self, icon};

#[derive(Debug, Clone)]
pub enum Load<T> {
    Loading,
    Loaded(T),
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NodeKey {
    Server,
    Databases,
    /// The folder holding master, model, msdb and tempdb.
    SystemDatabases,
    Database(String),
    Folder(String, ObjectKind),
    /// Database name and object id.
    Object(String, String),
}

#[derive(Debug, Clone)]
pub enum ExplorerMsg {
    Toggle(NodeKey),
    DatabasesLoaded(Result<Vec<(String, bool)>, String>),
    ObjectsLoaded(String, Result<Vec<DbObject>, String>),
    ColumnsLoaded(String, String, Result<Vec<ColumnInfo>, String>),
}

/// Something the explorer needs fetched from the server.
pub enum Request {
    Databases,
    Objects(String),
    Columns(String, DbObject),
}

pub struct Explorer {
    server_label: String,
    databases: Load<Vec<String>>,
    /// Which of `databases` are system databases.
    system: HashSet<String>,
    objects: HashMap<String, Load<Vec<DbObject>>>,
    columns: HashMap<(String, String), Load<Vec<ColumnInfo>>>,
    expanded: HashSet<NodeKey>,
    selected: Option<NodeKey>,
}

const INDENT: f32 = 12.0;
const CHEVRON: f32 = 14.0;

impl Explorer {
    pub fn new(server_label: String) -> Self {
        Self {
            server_label,
            databases: Load::Loading,
            system: HashSet::new(),
            objects: HashMap::new(),
            columns: HashMap::new(),
            expanded: [NodeKey::Server, NodeKey::Databases].into_iter().collect(),
            selected: None,
        }
    }

    /// Drops everything cached and returns what to reload, keeping the
    /// expanded nodes so the tree looks the same afterwards.
    pub fn refresh(&mut self) -> Vec<Request> {
        self.databases = Load::Loading;
        self.objects.clear();
        self.columns.clear();
        let mut requests = vec![Request::Databases];
        for key in &self.expanded {
            if let NodeKey::Database(db) = key {
                self.objects.insert(db.clone(), Load::Loading);
                requests.push(Request::Objects(db.clone()));
            }
        }
        // Columns reload when their table is expanded again.
        self.expanded.retain(|k| !matches!(k, NodeKey::Object(..)));
        requests
    }

    /// The object behind a node key, if it is loaded.
    pub fn object(&self, db: &str, id: &str) -> Option<&DbObject> {
        match self.objects.get(db) {
            Some(Load::Loaded(objs)) => objs.iter().find(|o| o.id == id),
            _ => None,
        }
    }

    pub fn update(&mut self, msg: ExplorerMsg) -> Option<Request> {
        match msg {
            ExplorerMsg::Toggle(key) => {
                self.selected = Some(key.clone());
                if self.expanded.remove(&key) {
                    return None;
                }
                self.expanded.insert(key.clone());
                match key {
                    NodeKey::Database(db) if !self.objects.contains_key(&db) => {
                        self.objects.insert(db.clone(), Load::Loading);
                        Some(Request::Objects(db))
                    }
                    NodeKey::Object(db, id) => {
                        let obj = self.object(&db, &id)?.clone();
                        let cache_key = (db.clone(), id);
                        if !obj.kind.is_selectable() || self.columns.contains_key(&cache_key) {
                            return None;
                        }
                        self.columns.insert(cache_key, Load::Loading);
                        Some(Request::Columns(db, obj))
                    }
                    _ => None,
                }
            }
            ExplorerMsg::DatabasesLoaded(r) => {
                let r = r.map(|dbs| {
                    self.system = dbs.iter().filter(|(_, system)| *system).map(|(name, _)| name.clone()).collect();
                    dbs.into_iter().map(|(name, _)| name).collect()
                });
                self.databases = to_load(r);
                None
            }
            ExplorerMsg::ObjectsLoaded(db, r) => {
                self.objects.insert(db, to_load(r));
                None
            }
            ExplorerMsg::ColumnsLoaded(db, id, r) => {
                self.columns.insert((db, id), to_load(r));
                None
            }
        }
    }

    /// The server's databases, once loaded.
    pub fn databases(&self) -> &[String] {
        match &self.databases {
            Load::Loaded(dbs) => dbs,
            _ => &[],
        }
    }

    pub fn view(&self) -> Element<'_, Message> {
        let mut tree = column![].spacing(1);

        let server_menu = vec![
            ("New Query", Message::NewQuery(None)),
            ("Refresh", Message::RefreshExplorer),
            ("Disconnect", Message::Disconnect),
        ];
        tree = tree.push(self.node(0, NodeKey::Server, &self.server_label, icon::server(), true, server_menu));
        if !self.is_open(&NodeKey::Server) {
            return tree.into();
        }
        tree = tree.push(self.folder_node(1, NodeKey::Databases, "Databases", true));
        if !self.is_open(&NodeKey::Databases) {
            return tree.into();
        }

        let dbs = match &self.databases {
            Load::Loaded(dbs) => dbs,
            other => return push_status(tree, 2, other).into(),
        };
        // Your own databases first; the built-in ones in a folder at the
        // bottom, like SSMS, since they're rarely what you're after.
        for db in dbs.iter().filter(|db| !self.system.contains(*db)) {
            tree = self.database(tree, 2, db);
        }
        if !self.system.is_empty() {
            tree = tree.push(self.folder_node(2, NodeKey::SystemDatabases, "System Databases", true));
            if self.is_open(&NodeKey::SystemDatabases) {
                for db in dbs.iter().filter(|db| self.system.contains(*db)) {
                    tree = self.database(tree, 3, db);
                }
            }
        }
        tree.into()
    }

    /// A database node at `level`, with its object folders when expanded.
    fn database<'a>(&'a self, mut tree: Column<'a, Message>, level: u16, db: &'a str) -> Column<'a, Message> {
        let db_key = NodeKey::Database(db.to_string());
        let db_menu =
            vec![("New Query", Message::NewQuery(Some(db.to_string()))), ("Refresh", Message::RefreshExplorer)];
        let glyph = style::icon_colored(icon::database(), Some(style::accent_color));
        tree = tree.push(self.node(level, db_key.clone(), db, glyph, true, db_menu));
        if !self.is_open(&db_key) {
            return tree;
        }
        let objects = match self.objects.get(db) {
            Some(Load::Loaded(objs)) => objs,
            Some(other) => return push_status(tree, level + 1, other),
            None => return tree,
        };
        for kind in ObjectKind::ALL {
            tree = self.folder(tree, level + 1, db, kind, objects);
        }
        tree
    }

    fn folder<'a>(
        &'a self,
        mut tree: Column<'a, Message>,
        level: u16,
        db: &'a str,
        kind: ObjectKind,
        objects: &'a [DbObject],
    ) -> Column<'a, Message> {
        let key = NodeKey::Folder(db.to_string(), kind);
        let count = objects.iter().filter(|o| o.kind == kind).count();
        tree = tree.push(self.folder_node(level, key.clone(), &format!("{}  {count}", kind.folder_name()), count > 0));
        if !self.is_open(&key) {
            return tree;
        }
        for obj in objects.iter().filter(|o| o.kind == kind) {
            let obj_key = NodeKey::Object(db.to_string(), obj.id.clone());
            let label = match &obj.parent {
                Some(table) => format!("{} (on {table})", obj.qualified_name()),
                None => obj.qualified_name(),
            };
            let mut menu = Vec::new();
            if kind.is_selectable() {
                menu.push(("Select Top 1000 Rows", Message::SelectTop(db.to_string(), obj.clone())));
            }
            if kind.has_definition() {
                menu.push(("Script", Message::Script(db.to_string(), obj.clone())));
            }
            menu.push(("New Query", Message::NewQuery(Some(db.to_string()))));
            let (glyph, color) = object_icon(kind);
            let glyph = style::icon_colored(glyph, Some(color));
            tree = tree.push(self.node(level + 1, obj_key.clone(), &label, glyph, kind.is_selectable(), menu));

            if kind.is_selectable() && self.is_open(&obj_key) {
                match self.columns.get(&(db.to_string(), obj.id.clone())) {
                    Some(Load::Loaded(cols)) => {
                        for c in cols {
                            tree = tree.push(column_leaf(level + 2, c));
                        }
                    }
                    Some(other) => tree = push_status(tree, level + 2, other),
                    None => {}
                }
            }
        }
        tree
    }

    fn is_open(&self, key: &NodeKey) -> bool {
        self.expanded.contains(key)
    }

    /// A folder row; right-click offers Refresh.
    fn folder_node<'a>(&self, level: u16, key: NodeKey, label: &str, expandable: bool) -> Element<'a, Message> {
        let glyph = if self.is_open(&key) { icon::folder_opened() } else { icon::folder() };
        self.node(level, key, label, glyph, expandable, vec![("Refresh", Message::RefreshExplorer)])
    }

    /// One tree row: chevron, icon, label. Left-click selects and expands
    /// it; right-click opens `menu`.
    fn node<'a>(
        &self,
        level: u16,
        key: NodeKey,
        label: &str,
        glyph: Text<'a>,
        expandable: bool,
        menu: Vec<(&'static str, Message)>,
    ) -> Element<'a, Message> {
        let chevron: Element<'a, Message> = match (expandable, self.is_open(&key)) {
            (false, _) => space().width(CHEVRON).into(),
            (true, open) => style::dim(if open { icon::chevron_down() } else { icon::chevron_right() })
                .size(12)
                .width(CHEVRON)
                .into(),
        };
        let selected = self.selected.as_ref() == Some(&key);
        let content = row![chevron, glyph.size(style::ICON), text(label.to_string()).size(style::BODY)]
            .spacing(style::S1 + 2.0)
            .align_y(Alignment::Center);
        let node = button(content)
            .padding([3.0, style::S1])
            .width(Length::Fill)
            .style(if selected { style::row_selected } else { style::flat })
            .on_press(Message::Explorer(ExplorerMsg::Toggle(key)));
        row![space().width(INDENT * f32::from(level)), ContextMenu::new(node, move || context_menu(&menu))].into()
    }
}

/// Icon and colour for each kind of database object.
fn object_icon<'a>(kind: ObjectKind) -> (Text<'a>, fn(&Theme) -> Color) {
    match kind {
        ObjectKind::Table => (icon::table(), style::accent_color),
        ObjectKind::View => (icon::eye(), style::accent_color),
        ObjectKind::Procedure => (icon::symbol_method(), purple),
        ObjectKind::Function => (icon::symbol_operator(), purple),
        ObjectKind::Trigger => (icon::symbol_event(), style::warning_color),
    }
}

pub fn purple(t: &Theme) -> Color {
    if t.extended_palette().is_dark { color!(0xa48cb8) } else { color!(0x7a5f91) }
}

/// A column under an expanded table: key or field icon, name, dimmed type.
fn column_leaf<'a>(level: u16, c: &ColumnInfo) -> Element<'a, Message> {
    let glyph = if c.primary_key {
        style::icon_colored(icon::key(), Some(style::warning_color))
    } else {
        style::dim(icon::symbol_field()).size(style::ICON)
    };
    let detail = format!("{}{}", c.data_type, if c.nullable { ", null" } else { "" });
    row![
        space().width(INDENT * f32::from(level) + CHEVRON + style::S1 + 6.0),
        glyph,
        text(c.name.clone()).size(style::BODY),
        style::dim(text(detail).size(style::SMALL)),
    ]
    .spacing(style::S1 + 2.0)
    .padding([2, 0])
    .align_y(Alignment::Center)
    .into()
}

pub fn context_menu<'a>(items: &[(&'static str, Message)]) -> Element<'a, Message> {
    let mut list = column![].spacing(1).width(200);
    for (label, msg) in items {
        list = list.push(
            button(text(*label).size(style::BODY))
                .width(Length::Fill)
                .padding([5.0, style::S3])
                .style(style::flat)
                .on_press(msg.clone()),
        );
    }
    container(list).padding(style::S1).style(style::menu).into()
}

fn push_status<'a, T>(tree: Column<'a, Message>, level: u16, load: &Load<T>) -> Column<'a, Message> {
    let indent = space().width(INDENT * f32::from(level) + CHEVRON + style::S1 + 6.0);
    match load {
        Load::Loading => tree.push(row![indent, style::dim(text("Loading…").size(style::SMALL))].padding([2, 0])),
        Load::Failed(e) => tree.push(
            row![
                indent,
                text(e.clone())
                    .size(style::SMALL)
                    .style(|t: &Theme| text::Style { color: Some(style::danger_color(t)) })
            ]
            .padding([2, 0]),
        ),
        Load::Loaded(_) => tree,
    }
}

fn to_load<T>(r: Result<T, String>) -> Load<T> {
    match r {
        Ok(v) => Load::Loaded(v),
        Err(e) => Load::Failed(e),
    }
}
