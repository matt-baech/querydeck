//! Object Explorer: a lazily loaded tree of databases and their objects.

use std::collections::{HashMap, HashSet};

use iced::widget::{Column, button, column, container, row, space, text};
use iced::{Alignment, Element, Length, Theme};
use iced_aw::ContextMenu;

use crate::app::Message;
use crate::db::catalog::{ColumnInfo, DbObject, ObjectKind};

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
    Database(String),
    Folder(String, ObjectKind),
    /// Database name and object id.
    Object(String, String),
}

#[derive(Debug, Clone)]
pub enum ExplorerMsg {
    Toggle(NodeKey),
    DatabasesLoaded(Result<Vec<String>, String>),
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
    objects: HashMap<String, Load<Vec<DbObject>>>,
    columns: HashMap<(String, String), Load<Vec<ColumnInfo>>>,
    expanded: HashSet<NodeKey>,
    selected: Option<NodeKey>,
}

const INDENT: f32 = 14.0;

impl Explorer {
    pub fn new(server_label: String) -> Self {
        Self {
            server_label,
            databases: Load::Loading,
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

    pub fn view(&self) -> Element<'_, Message> {
        let mut tree = column![].spacing(1);

        let server_menu = vec![
            ("New Query", Message::NewQuery(None)),
            ("Refresh", Message::RefreshExplorer),
            ("Disconnect", Message::Disconnect),
        ];
        tree = tree.push(self.node(0, NodeKey::Server, &self.server_label, true, server_menu));
        if !self.is_open(&NodeKey::Server) {
            return tree.into();
        }
        tree = tree.push(self.node(1, NodeKey::Databases, "Databases", true, vec![("Refresh", Message::RefreshExplorer)]));
        if !self.is_open(&NodeKey::Databases) {
            return tree.into();
        }

        let dbs = match &self.databases {
            Load::Loaded(dbs) => dbs,
            other => return push_status(tree, 2, other).into(),
        };
        for db in dbs {
            let db_key = NodeKey::Database(db.clone());
            let db_menu = vec![("New Query", Message::NewQuery(Some(db.clone()))), ("Refresh", Message::RefreshExplorer)];
            tree = tree.push(self.node(2, db_key.clone(), db, true, db_menu));
            if !self.is_open(&db_key) {
                continue;
            }
            let objects = match self.objects.get(db) {
                Some(Load::Loaded(objs)) => objs,
                Some(other) => {
                    tree = push_status(tree, 3, other);
                    continue;
                }
                None => continue,
            };
            for kind in ObjectKind::ALL {
                tree = self.folder(tree, db, kind, objects);
            }
        }
        tree.into()
    }

    fn folder<'a>(
        &'a self,
        mut tree: Column<'a, Message>,
        db: &'a str,
        kind: ObjectKind,
        objects: &'a [DbObject],
    ) -> Column<'a, Message> {
        let key = NodeKey::Folder(db.to_string(), kind);
        let count = objects.iter().filter(|o| o.kind == kind).count();
        let label = format!("{} ({count})", kind.folder_name());
        tree = tree.push(self.node(3, key.clone(), &label, count > 0, vec![("Refresh", Message::RefreshExplorer)]));
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
            tree = tree.push(self.node(4, obj_key.clone(), &label, kind.is_selectable(), menu));

            if kind.is_selectable() && self.is_open(&obj_key) {
                match self.columns.get(&(db.to_string(), obj.id.clone())) {
                    Some(Load::Loaded(cols)) => {
                        for c in cols {
                            tree = tree.push(leaf(5, c.to_string()));
                        }
                    }
                    Some(other) => tree = push_status(tree, 5, other),
                    None => {}
                }
            }
        }
        tree
    }

    fn is_open(&self, key: &NodeKey) -> bool {
        self.expanded.contains(key)
    }

    /// One tree row. Left-click selects and expands it; right-click opens `menu`.
    fn node<'a>(
        &self,
        level: u16,
        key: NodeKey,
        label: &str,
        expandable: bool,
        menu: Vec<(&'static str, Message)>,
    ) -> Element<'a, Message> {
        let arrow = match (expandable, self.is_open(&key)) {
            (false, _) => " ",
            (true, true) => "▾",
            (true, false) => "▸",
        };
        let selected = self.selected.as_ref() == Some(&key);
        let content = row![text(arrow).size(13).width(12), text(label.to_string()).size(13)]
            .spacing(4)
            .align_y(Alignment::Center);
        let node = button(content)
            .padding([2, 6])
            .width(Length::Fill)
            .style(if selected { button::secondary } else { button::text })
            .on_press(Message::Explorer(ExplorerMsg::Toggle(key)));
        row![space().width(INDENT * f32::from(level)), ContextMenu::new(node, move || context_menu(&menu))].into()
    }
}

fn leaf<'a>(level: u16, label: String) -> Element<'a, Message> {
    row![space().width(INDENT * f32::from(level) + 22.0), text(label).size(12).style(text::secondary)]
        .padding([1, 0])
        .into()
}

fn context_menu<'a>(items: &[(&'static str, Message)]) -> Element<'a, Message> {
    let mut list = column![].spacing(1).width(190);
    for (label, msg) in items {
        list = list.push(
            button(text(*label).size(13)).width(Length::Fill).padding([4, 10]).style(button::text).on_press(msg.clone()),
        );
    }
    container(list).padding(4).style(menu_style).into()
}

fn menu_style(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(palette.background.weak.color.into()),
        border: iced::Border { color: palette.background.strong.color, width: 1.0, radius: 6.0.into() },
        shadow: iced::Shadow {
            color: iced::Color::from_rgba(0.0, 0.0, 0.0, 0.4),
            offset: iced::Vector::new(0.0, 4.0),
            blur_radius: 12.0,
        },
        ..container::Style::default()
    }
}

fn push_status<'a, T>(tree: Column<'a, Message>, level: u16, load: &Load<T>) -> Column<'a, Message> {
    match load {
        Load::Loading => tree.push(leaf(level, "Loading…".into())),
        Load::Failed(e) => tree.push(
            row![space().width(INDENT * f32::from(level) + 22.0), text(e.clone()).size(12).style(text::danger)]
                .padding([1, 0]),
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
