use std::sync::Arc;

use iced::keyboard::{self, Key, key::Named};
use iced::theme::Mode;
use iced::widget::{button, center, column, container, opaque, pick_list, row, rule, scrollable, space, stack, text};
use iced::{Alignment, Color, Element, Length, Subscription, Task, Theme};

use crate::connect::{ConnectForm, FormMsg};
use crate::db::catalog::{self, DbObject};
use crate::db::{self, Connected, Connection, Profile, QueryOutcome};
use crate::explorer::{Explorer, ExplorerMsg, Request};
use crate::query_tab::{QueryTab, ResultsPane};
use crate::settings::{Settings, ThemeChoice};
use crate::style;

const SAMPLE_SQL: &str = "-- F5 or Ctrl+Enter runs the script (or just the selection).\n\
-- SQL Server scripts can be split into batches with GO.\n\nSELECT 1 AS hello;\n";

#[derive(Debug, Clone)]
pub enum Message {
    // Appearance
    SetTheme(ThemeChoice),
    SystemTheme(Mode),

    // Connect dialog
    Form(FormMsg),
    OpenDialog,
    CloseDialog,
    Connect,
    Connected(Result<Connected, String>),
    Disconnect,

    // Object Explorer
    ToggleSidebar,
    Explorer(ExplorerMsg),
    RefreshExplorer,
    SelectTop(String, DbObject),
    Script(String, DbObject),
    ScriptLoaded(String, String, Result<String, String>),
    NewQuery(Option<String>),

    // Query tabs
    SelectTab(u64),
    CloseTab(u64),
    Edit(iced::widget::text_editor::Action),
    Run,
    Cancel,
    TabConnected(u64, String, Result<Connected, String>),
    QueryFinished(u64, Arc<QueryOutcome>),
    CancelSent(Result<(), String>),
    ShowPane(ResultsPane),
    SelectResult(usize),
    Page(usize),
}

/// A live connection to a server: what we connected with, plus the
/// connection the Object Explorer uses. Query tabs open their own.
struct Session {
    profile: Profile,
    password: String,
    explorer_conn: Connection,
}

pub struct App {
    form: ConnectForm,
    dialog_open: bool,
    session: Option<Session>,
    explorer: Option<Explorer>,
    sidebar_open: bool,

    tabs: Vec<QueryTab>,
    active: u64,
    next_tab_id: u64,

    status: Status,
    settings: Settings,
    /// Light or dark, as the desktop currently prefers.
    system_mode: Mode,
}

enum Status {
    Idle,
    Info(String),
    Error(String),
}

impl App {
    pub fn new() -> (Self, Task<Message>) {
        let mut app = Self {
            form: ConnectForm::new(),
            dialog_open: true,
            session: None,
            explorer: None,
            sidebar_open: true,
            tabs: Vec::new(),
            active: 0,
            next_tab_id: 1,
            status: Status::Idle,
            settings: Settings::load(),
            system_mode: Mode::Dark,
        };
        app.open_tab(None, SAMPLE_SQL, None);
        (app, iced::system::theme().map(Message::SystemTheme))
    }

    pub fn title(&self) -> String {
        match &self.session {
            Some(s) => format!("litedb — {}", s.profile.display_name()),
            None => "litedb".into(),
        }
    }

    pub fn theme(&self) -> Theme {
        self.settings.theme.resolve(self.system_mode)
    }

    pub fn subscription(&self) -> Subscription<Message> {
        // The editor handles its own shortcuts while focused; this covers the
        // case where focus is elsewhere.
        let keys = keyboard::listen().filter_map(|event| match event {
            keyboard::Event::KeyPressed { key, modifiers, .. } => shortcut(&key, modifiers),
            _ => None,
        });
        Subscription::batch([keys, iced::system::theme_changes().map(Message::SystemTheme)])
    }

    fn active_tab(&mut self) -> Option<&mut QueryTab> {
        let id = self.active;
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    fn tab(&mut self, id: u64) -> Option<&mut QueryTab> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    fn open_tab(&mut self, title: Option<String>, sql: &str, database: Option<String>) -> u64 {
        let id = self.next_tab_id;
        self.next_tab_id += 1;
        let title = title.unwrap_or_else(|| format!("SQLQuery{id}"));
        self.tabs.push(QueryTab::new(id, title, sql, database));
        self.active = id;
        id
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::SetTheme(choice) => {
                self.settings.theme = choice;
                if let Err(e) = self.settings.save() {
                    self.status = Status::Error(e);
                }
            }
            Message::SystemTheme(mode) => self.system_mode = mode,
            Message::Form(msg) => self.form.update(msg),
            Message::OpenDialog => self.dialog_open = true,
            Message::CloseDialog => self.dialog_open = false,
            Message::Connect => {
                if self.form.connecting || !self.form.validate() {
                    return Task::none();
                }
                self.form.connecting = true;
                self.form.notice = Some(Ok(format!("Connecting to {}…", self.form.form.display_name())));
                let profile = self.form.form.clone();
                let password = self.form.password.clone();
                return Task::perform(db::connect(profile, password), Message::Connected);
            }
            Message::Connected(result) => {
                self.form.connecting = false;
                match result {
                    Ok(c) => return self.start_session(c),
                    Err(e) => self.form.notice = Some(Err(format!("Connection failed: {e}"))),
                }
            }
            Message::Disconnect => {
                let close = self.end_session();
                self.status = Status::Info("Disconnected.".into());
                self.dialog_open = true;
                return close;
            }

            Message::ToggleSidebar => self.sidebar_open = !self.sidebar_open,
            Message::Explorer(msg) => {
                if let Some(request) = self.explorer.as_mut().and_then(|e| e.update(msg)) {
                    return self.fetch(request);
                }
            }
            Message::RefreshExplorer => {
                if let Some(explorer) = &mut self.explorer {
                    let requests = explorer.refresh();
                    return Task::batch(requests.into_iter().map(|r| self.fetch(r)));
                }
            }
            Message::SelectTop(database, obj) => {
                let Some(session) = &self.session else { return Task::none() };
                let sql = catalog::select_top_script(session.profile.driver, &obj);
                let id = self.open_tab(Some(obj.qualified_name()), &sql, Some(database));
                return self.run_tab(id);
            }
            Message::Script(database, obj) => {
                let Some(session) = &self.session else { return Task::none() };
                let conn = session.explorer_conn.clone();
                let title = obj.qualified_name();
                self.status = Status::Info(format!("Scripting {title}…"));
                return Task::perform(catalog::definition(conn, database.clone(), obj), move |r| {
                    Message::ScriptLoaded(database.clone(), title.clone(), r)
                });
            }
            Message::ScriptLoaded(database, title, result) => match result {
                Ok(sql) => {
                    self.open_tab(Some(title), &sql, Some(database));
                    self.status = Status::Idle;
                }
                Err(e) => self.status = Status::Error(e),
            },
            Message::NewQuery(database) => {
                self.open_tab(None, "", database);
            }

            Message::SelectTab(id) => self.active = id,
            Message::CloseTab(id) => {
                let Some(pos) = self.tabs.iter().position(|t| t.id == id) else { return Task::none() };
                let mut tab = self.tabs.remove(pos);
                let close = close_conn(tab.take_connection());
                if self.active == id && !self.tabs.is_empty() {
                    self.active = self.tabs[pos.min(self.tabs.len() - 1)].id;
                }
                return close;
            }
            Message::Edit(action) => {
                if let Some(tab) = self.active_tab() {
                    tab.editor.perform(action);
                }
            }
            Message::Run => return self.run_tab(self.active),
            Message::Cancel => {
                // Esc closes the dialog when it is open.
                if self.dialog_open {
                    self.dialog_open = false;
                    return Task::none();
                }
                let Some(tab) = self.active_tab() else { return Task::none() };
                let Some(handle) = tab.running.take() else { return Task::none() };
                handle.abort();
                let conn = tab.conn.clone();
                self.status = Status::Info("Cancelling…".into());
                if let Some(conn) = conn {
                    return Task::perform(conn.cancel(), Message::CancelSent);
                }
            }
            Message::CancelSent(result) => {
                self.status = match result {
                    Ok(()) => Status::Info("Query cancelled.".into()),
                    Err(e) => Status::Error(format!("Cancel failed: {e}")),
                };
            }
            Message::TabConnected(id, sql, result) => {
                let still_waiting = self.tab(id).is_some_and(|t| t.running.is_some());
                match result {
                    Ok(c) if still_waiting && self.session.is_some() => {
                        let tab = self.tab(id).expect("checked above");
                        tab.current_db = c.database;
                        tab.conn = Some(c.conn);
                        return self.execute(id, sql);
                    }
                    // The tab was closed or cancelled while connecting.
                    Ok(c) => return close_conn(Some(c.conn)),
                    Err(e) => {
                        if let Some(tab) = self.tab(id) {
                            tab.running = None;
                        }
                        self.status = Status::Error(format!("Could not open a connection for the query: {e}"));
                    }
                }
            }
            Message::QueryFinished(id, outcome) => {
                let secs = outcome.elapsed.as_secs_f64();
                self.status = if outcome.failed {
                    Status::Error(format!("Query failed after {secs:.3}s."))
                } else {
                    Status::Info(format!("Query completed in {secs:.3}s."))
                };
                if let Some(tab) = self.tab(id) {
                    tab.finish(outcome);
                }
            }
            Message::ShowPane(p) => {
                if let Some(tab) = self.active_tab() {
                    tab.pane = p;
                }
            }
            Message::SelectResult(i) => {
                if let Some(tab) = self.active_tab() {
                    tab.result_index = i;
                    tab.page = 0;
                }
            }
            Message::Page(p) => {
                if let Some(tab) = self.active_tab() {
                    tab.page = p;
                }
            }
        }
        Task::none()
    }

    fn start_session(&mut self, c: Connected) -> Task<Message> {
        let close = self.end_session();
        self.form.remember_password();
        let profile = self.form.form.clone();
        let label = format!("{} ({})", profile.host, profile.driver);
        self.status = Status::Info(format!("Connected. {}", c.server_version));
        self.session = Some(Session { profile, password: self.form.password.clone(), explorer_conn: c.conn });
        self.explorer = Some(Explorer::new(label));
        self.dialog_open = false;
        self.sidebar_open = true;
        close.chain(self.fetch(Request::Databases))
    }

    /// Closes the explorer's and every tab's connection.
    fn end_session(&mut self) -> Task<Message> {
        let mut conns: Vec<Connection> = self.tabs.iter_mut().filter_map(|t| t.take_connection()).collect();
        if let Some(s) = self.session.take() {
            conns.push(s.explorer_conn);
        }
        self.explorer = None;
        Task::batch(conns.into_iter().map(|c| close_conn(Some(c))))
    }

    fn fetch(&self, request: Request) -> Task<Message> {
        let Some(session) = &self.session else { return Task::none() };
        let conn = session.explorer_conn.clone();
        match request {
            Request::Databases => Task::perform(catalog::databases(conn), |r| {
                Message::Explorer(ExplorerMsg::DatabasesLoaded(r))
            }),
            Request::Objects(db) => Task::perform(catalog::objects(conn, db.clone()), move |r| {
                Message::Explorer(ExplorerMsg::ObjectsLoaded(db.clone(), r))
            }),
            Request::Columns(db, obj) => {
                let id = obj.id.clone();
                Task::perform(catalog::columns(conn, db.clone(), obj), move |r| {
                    Message::Explorer(ExplorerMsg::ColumnsLoaded(db.clone(), id.clone(), r))
                })
            }
        }
    }

    /// Runs the tab's script, opening the tab's connection first if needed.
    fn run_tab(&mut self, id: u64) -> Task<Message> {
        let Some(session) = &self.session else {
            self.status = Status::Error("Connect to a server first.".into());
            return Task::none();
        };
        let mut profile = session.profile.clone();
        let password = session.password.clone();
        let Some(tab) = self.tab(id) else { return Task::none() };
        if tab.running.is_some() {
            return Task::none();
        }
        let Some(sql) = tab.sql_to_run() else { return Task::none() };

        if tab.conn.is_some() {
            return self.execute(id, sql);
        }
        if let Some(db) = &tab.database {
            profile.database = db.clone();
        }
        let (task, handle) =
            Task::perform(db::connect(profile, password), move |r| Message::TabConnected(id, sql.clone(), r))
                .abortable();
        tab.running = Some(handle);
        self.status = Status::Info("Connecting…".into());
        task
    }

    fn execute(&mut self, id: u64, sql: String) -> Task<Message> {
        let Some(tab) = self.tab(id) else { return Task::none() };
        let Some(conn) = tab.conn.clone() else { return Task::none() };
        let (task, handle) =
            Task::perform(conn.run(sql), move |outcome| Message::QueryFinished(id, Arc::new(outcome))).abortable();
        tab.running = Some(handle);
        self.status = Status::Info("Executing…".into());
        task
    }

    // ---------------------------------------------------------------- view

    pub fn view(&self) -> Element<'_, Message> {
        let main = column![self.tab_strip(), self.active_tab_view()].width(Length::Fill);
        let body = if self.sidebar_open {
            row![self.sidebar(), rule::vertical(1), main]
        } else {
            row![main]
        };
        let base = column![body.height(Length::Fill), rule::horizontal(1), self.status_bar()];

        if self.dialog_open {
            let live = self.session.as_ref().map(|s| &s.profile);
            stack![base, opaque(center(self.form.view(live)).style(backdrop))].into()
        } else {
            base.into()
        }
    }

    fn sidebar(&self) -> Element<'_, Message> {
        let connected = self.session.is_some();
        let header = row![
            text("Object Explorer").size(15),
            space().width(Length::Fill),
            small_button("Connect…", Some(Message::OpenDialog)),
            small_button("⟳", connected.then_some(Message::RefreshExplorer)),
            small_button("Disconnect", connected.then_some(Message::Disconnect)),
        ]
        .spacing(4)
        .align_y(Alignment::Center);

        let tree: Element<'_, Message> = match &self.explorer {
            Some(explorer) => scrollable(explorer.view()).height(Length::Fill).into(),
            None => container(
                column![
                    text("Not connected.").size(14),
                    button("Connect…").on_press(Message::OpenDialog).style(style::success),
                ]
                .spacing(10)
                .align_x(Alignment::Center),
            )
            .center(Length::Fill)
            .into(),
        };

        container(column![header, rule::horizontal(1), tree].spacing(6))
            .padding(8)
            .width(320)
            .height(Length::Fill)
            .into()
    }

    fn tab_strip(&self) -> Element<'_, Message> {
        let mut strip = row![
            button(text(if self.sidebar_open { "«" } else { "»" }).size(14))
                .style(button::text)
                .on_press(Message::ToggleSidebar)
        ]
        .spacing(2)
        .align_y(Alignment::Center);
        for tab in &self.tabs {
            let active = tab.id == self.active;
            let busy = if tab.running.is_some() { "⏳ " } else { "" };
            strip = strip.push(
                container(
                    row![
                        button(text(format!("{busy}{}", tab.title)).size(13))
                            .style(move |t: &Theme, s| tab_button(t, s, active))
                            .padding([6, 10])
                            .on_press(Message::SelectTab(tab.id)),
                        button(text("×").size(13)).style(move |t: &Theme, s| tab_button(t, s, active)).padding([6, 8]).on_press(Message::CloseTab(tab.id)),
                    ]
                    .align_y(Alignment::Center),
                )
                .style(move |theme: &Theme| style::tab(theme, active)),
            );
        }
        strip = strip.push(
            button(text("+").size(14)).style(button::text).on_press(Message::NewQuery(None)),
        );
        container(scrollable(strip).direction(scrollable::Direction::Horizontal(scrollable::Scrollbar::new().width(4).scroller_width(4))))
            .padding(iced::Padding { top: 4.0, right: 6.0, bottom: 0.0, left: 6.0 })
            .width(Length::Fill)
            .style(style::tab_strip)
            .into()
    }

    fn active_tab_view(&self) -> Element<'_, Message> {
        match self.tabs.iter().find(|t| t.id == self.active) {
            Some(tab) => tab.view(self.session.is_some(), style::highlighter(&self.theme())),
            None => container(
                column![
                    text("No open queries").size(16),
                    button("New Query (Ctrl+N)").on_press(Message::NewQuery(None)).style(style::neutral),
                ]
                .spacing(10)
                .align_x(Alignment::Center),
            )
            .center(Length::Fill)
            .into(),
        }
    }

    fn status_bar(&self) -> Element<'_, Message> {
        let (msg, style): (&str, fn(&Theme) -> text::Style) = match &self.status {
            Status::Idle => ("Ready", text::default),
            Status::Info(s) => (s, text::default),
            Status::Error(s) => (s, text::danger),
        };
        let server = match &self.session {
            Some(s) => format!("{} · {}@{}:{}", s.profile.driver, s.profile.user, s.profile.host, s.profile.port),
            None => "Not connected".into(),
        };
        container(
            row![
                text(msg).size(13).style(style),
                space().width(Length::Fill),
                text(server).size(13).style(text::secondary),
                pick_list(&ThemeChoice::ALL[..], Some(self.settings.theme), Message::SetTheme)
                    .text_size(12)
                    .padding([2, 8]),
            ]
            .spacing(12)
            .align_y(Alignment::Center),
        )
        .padding([4, 10])
        .width(Length::Fill)
        .into()
    }
}

pub fn shortcut(key: &Key, modifiers: keyboard::Modifiers) -> Option<Message> {
    match key {
        Key::Named(Named::F5) => Some(Message::Run),
        Key::Named(Named::Enter) if modifiers.command() => Some(Message::Run),
        Key::Named(Named::Escape) => Some(Message::Cancel),
        Key::Character(c) if modifiers.command() && c.as_str() == "n" => Some(Message::NewQuery(None)),
        _ => None,
    }
}

fn close_conn(conn: Option<Connection>) -> Task<Message> {
    match conn {
        Some(c) => Task::future(c.close()).discard(),
        None => Task::none(),
    }
}

fn small_button(label: &str, msg: Option<Message>) -> iced::widget::Button<'_, Message> {
    button(text(label).size(12)).padding([3, 7]).style(style::neutral).on_press_maybe(msg)
}

fn backdrop(_theme: &Theme) -> container::Style {
    container::Style { background: Some(Color::from_rgba(0.0, 0.0, 0.0, 0.55).into()), ..container::Style::default() }
}

/// Tab labels are plain text; inactive ones are dimmed until hovered. The
/// container behind them shows which tab is active.
fn tab_button(theme: &Theme, status: button::Status, active: bool) -> button::Style {
    let text = theme.extended_palette().background.base.text;
    let text_color = match status {
        _ if active => text,
        button::Status::Hovered | button::Status::Pressed => text,
        _ => text.scale_alpha(0.6),
    };
    button::Style { text_color, ..button::text(theme, status) }
}
