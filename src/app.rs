use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;

use iced::keyboard::{self, Key, key::Named};
use iced::theme::Mode;
use iced::widget::{
    button, center, checkbox, column, container, opaque, operation, row, scrollable, space, stack, text,
};
use iced::{Alignment, Color, Element, Length, Point, Subscription, Task, Theme, mouse, window};
use iced_aw::ContextMenu;
use iced_code::{Action, Edit};

use crate::complete::{self, Sources};

use crate::connect::{ConnectForm, FormMsg};
use crate::db::catalog::{self, DbObject, SchemaInfo};
use crate::db::{self, Connected, Connection, Profile, QueryOutcome};
use crate::explorer::{Explorer, ExplorerMsg, Load, Request, context_menu};
use crate::files;
use crate::preferences::{self, TrayStatus};
use crate::query_tab::{FindBar, QueryTab, ResultsPane, completion_popup, horizontal_divider, splitter};
use crate::settings::{Settings, ThemeChoice};
use crate::style::{self, icon};
use crate::tray::{self, TrayEvent};

#[derive(Debug, Clone)]
pub enum Message {
    // Appearance
    OpenSettings,
    CloseSettings,
    SetTheme(ThemeChoice),
    SetEditorFontSize(f32),
    SetPageSize(usize),
    SetMinimizeToTray(bool),
    SetCompleteWhileTyping(bool),
    Tray(TrayEvent),
    CloseRequested,
    ModifiersChanged(keyboard::Modifiers),
    Resized(iced::Size),
    Moved(Point),
    /// The window's size or position changed; `bool` is whether it's now
    /// maximized.
    Geometry(Option<iced::Size>, Option<Point>, bool),
    DragStart(Drag),
    DragMove(Point),
    DragEnd,
    SystemTheme(Mode),

    // Connect dialog
    Form(FormMsg),
    OpenDialog,
    CloseDialog,
    Connect,
    Connected(Result<Connected, String>),
    Disconnect,

    // Find and replace
    /// Opens the find bar; `true` also shows the replace row.
    OpenFind(bool),
    CloseFind,
    FindInput(String),
    ReplaceInput(String),
    ToggleMatchCase,
    ToggleWholeWord,
    ToggleReplace,
    /// Enter in the find box: next match, or previous with Shift.
    FindSubmit,
    FindNext,
    FindPrevious,
    Replace,
    ReplaceAll,

    // Object Explorer
    ToggleSidebar,
    Explorer(ExplorerMsg),
    RefreshExplorer,
    SelectTop(String, DbObject),
    Script(String, DbObject),
    ScriptLoaded(String, String, Result<String, String>),
    NewQuery(Option<String>),

    // Files
    OpenFile,
    OpenPaths(Vec<PathBuf>),
    FileLoaded(PathBuf, Result<String, String>),
    Save,
    SaveAs,
    SavePicked(u64, Option<PathBuf>, AfterSave),
    CloseOthers(u64),
    CloseToRight(u64),
    CloseAll,
    CopyPath(String),
    /// Unsaved-changes prompts: per file, save or don't save; for several
    /// files, discard them all or go through them one by one.
    ConfirmSave,
    ConfirmDiscard,
    ConfirmApplyRest(bool),
    ConfirmDiscardAll,
    ConfirmEach,
    ConfirmCancel,

    // Query tabs
    SelectTab(u64),
    CloseTab(u64),
    Edit(Action),
    Run,
    Cancel,
    TabConnected(u64, String, Result<Connected, String>),
    QueryFinished(u64, Arc<QueryOutcome>),
    CancelSent(Result<(), String>),
    ShowPane(ResultsPane),
    SelectResult(usize),
    Page(usize),

    // Autocomplete
    TriggerCompletion,
    CompletionMove(isize),
    /// Insert the given suggestion, or the selected one.
    AcceptCompletion(Option<usize>),
    CloseCompletion,
    SchemaLoaded(String, Result<Arc<SchemaInfo>, String>),
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
    /// Tabs with unsaved changes being closed, one question at a time.
    closing: Option<Closing>,
    /// Autocomplete metadata per database, loaded on first use.
    schemas: HashMap<String, Load<Arc<SchemaInfo>>>,

    status: Status,
    settings: Settings,
    drag: Option<DragState>,
    settings_open: bool,
    tray: TrayStatus,
    /// The main window, or `None` while litedb sits in the tray.
    window: Option<window::Id>,
    window_size: iced::Size,
    /// Held modifier keys, for Shift+Enter in the find box.
    modifiers: keyboard::Modifiers,
    /// Light or dark, as the desktop currently prefers.
    system_mode: Mode,
}

/// Sidebar width limits. The minimum keeps the explorer header (title and
/// three icon buttons) fully visible.
const SIDEBAR_MIN: f32 = 240.0;
const SIDEBAR_MAX: f32 = 700.0;

/// A pane splitter the user can drag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Drag {
    Sidebar,
    Editor,
}

struct DragState {
    drag: Drag,
    anchor: Option<Point>,
    /// Pane size when the drag started.
    start: f32,
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
            schemas: HashMap::new(),
            closing: None,
            status: Status::Idle,
            settings: Settings::load(),
            drag: None,
            settings_open: false,
            tray: TrayStatus::Off,
            window: None,
            window_size: iced::Size::ZERO,
            modifiers: keyboard::Modifiers::default(),
            system_mode: Mode::Dark,
        };
        app.open_tab(None, "", None);
        if app.settings.minimize_to_tray {
            app.tray = TrayStatus::Starting;
        }
        let open = app.open_window();
        // `litedb a.sql b.sql` opens the files.
        let paths: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
        let files = if paths.is_empty() { Task::none() } else { Task::done(Message::OpenPaths(paths)) };
        (app, Task::batch([open, iced::system::theme().map(Message::SystemTheme), files]))
    }

    pub fn title(&self, _window: window::Id) -> String {
        match &self.session {
            Some(s) => format!("litedb — {}", s.profile.display_name()),
            None => "litedb".into(),
        }
    }

    pub fn theme(&self, _window: window::Id) -> Theme {
        self.current_theme()
    }

    fn current_theme(&self) -> Theme {
        self.settings.theme.resolve(self.system_mode)
    }

    pub fn subscription(&self) -> Subscription<Message> {
        // The editor handles its own shortcuts while focused; this covers the
        // case where focus is elsewhere.
        let keys = keyboard::listen().filter_map(|event| match event {
            keyboard::Event::KeyPressed { key, modifiers, .. } => shortcut(&key, modifiers),
            _ => None,
        });
        let mut subs = vec![
            keys,
            iced::system::theme_changes().map(Message::SystemTheme),
            window::close_requests().map(|_| Message::CloseRequested),
            window::resize_events().map(|(_, size)| Message::Resized(size)),
            iced::event::listen_with(|event, _status, _window| match event {
                iced::Event::Window(window::Event::FileDropped(path)) => Some(Message::OpenPaths(vec![path])),
                iced::Event::Window(window::Event::Moved(position)) => Some(Message::Moved(position)),
                iced::Event::Keyboard(keyboard::Event::ModifiersChanged(m)) => Some(Message::ModifiersChanged(m)),
                _ => None,
            }),
        ];
        // The find boxes swallow Esc, so listen for it whatever has focus.
        if self.tabs.iter().any(|t| t.id == self.active && t.find.is_some()) {
            subs.push(iced::event::listen_with(|event, _status, _window| match event {
                iced::Event::Keyboard(keyboard::Event::KeyPressed { key: Key::Named(Named::Escape), .. }) => {
                    Some(Message::CloseFind)
                }
                _ => None,
            }));
        }
        if self.settings.minimize_to_tray {
            subs.push(tray::subscription().map(Message::Tray));
        }
        // Track the mouse only while a splitter is being dragged.
        if self.drag.is_some() {
            subs.push(iced::event::listen_with(|event, _status, _window| match event {
                iced::Event::Mouse(mouse::Event::CursorMoved { position }) => Some(Message::DragMove(position)),
                iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => Some(Message::DragEnd),
                _ => None,
            }));
        }
        Subscription::batch(subs)
    }

    fn open_window(&mut self) -> Task<Message> {
        let geometry = self.settings.window;
        self.window_size = iced::Size::new(geometry.width, geometry.height);
        let position = match (geometry.x, geometry.y) {
            (Some(x), Some(y)) => window::Position::Specific(Point::new(x, y)),
            _ => window::Position::default(),
        };
        let (id, open) = window::open(window::Settings {
            size: self.window_size,
            position,
            maximized: geometry.maximized,
            min_size: Some(iced::Size::new(720.0, 480.0)),
            // Wayland compositors ignore this and use the icon of the
            // litedb.desktop entry that matches `application_id`.
            icon: window::icon::from_rgba(tray::icon::window(64), 64, 64).ok(),
            // Closing is decided in `CloseRequested` (quit or go to the tray).
            exit_on_close_request: false,
            platform_specific: window::settings::PlatformSpecific {
                application_id: "litedb".into(),
                ..Default::default()
            },
            ..window::Settings::default()
        });
        self.window = Some(id);
        open.discard()
    }

    /// Closes the window but keeps running in the tray.
    fn hide_window(&mut self) -> Task<Message> {
        self.drag = None;
        self.save_settings();
        match self.window.take() {
            Some(id) => window::close(id),
            None => Task::none(),
        }
    }

    /// Changes the find bar, then searches again from the cursor so the
    /// selection follows what's typed.
    fn update_find(&mut self, change: impl FnOnce(&mut FindBar)) {
        let Some(tab) = self.active_tab() else {
            return;
        };
        let Some(find) = &mut tab.find else {
            return;
        };
        change(find);
        let search = find.search();
        tab.editor.set_search(Some(search));
        let _ = tab.editor.find_from_cursor();
    }

    /// Asks whether the window is maximized before recording a new size or
    /// position.
    fn check_maximized(&self, size: Option<iced::Size>, position: Option<Point>) -> Task<Message> {
        match self.window {
            Some(id) => window::is_maximized(id).map(move |maximized| Message::Geometry(size, position, maximized)),
            None => Task::none(),
        }
    }

    fn show_window(&mut self) -> Task<Message> {
        match self.window {
            Some(id) => window::gain_focus(id),
            None => self.open_window(),
        }
    }

    /// Quits, unless some tabs have unsaved changes: then asks first.
    fn request_quit(&mut self) -> Task<Message> {
        let ids = self.tabs.iter().map(|t| t.id).collect();
        self.close_tabs(ids, true)
    }

    /// Closes tabs. Those without unsaved changes go at once; for the rest
    /// the user is asked (one prompt, or an overview first if there are
    /// several). With `then_quit`, quits once they're all handled.
    fn close_tabs(&mut self, ids: Vec<u64>, then_quit: bool) -> Task<Message> {
        let (dirty, clean): (Vec<u64>, Vec<u64>) =
            ids.into_iter().partition(|id| self.tabs.iter().any(|t| t.id == *id && t.dirty));
        let mut tasks: Vec<Task<Message>> = clean.into_iter().map(|id| self.close_tab(id)).collect();
        match dirty.first() {
            None if then_quit => tasks.push(self.quit()),
            None => {}
            Some(&first) => {
                self.active = first;
                self.closing = Some(Closing {
                    total: dirty.len(),
                    overview: dirty.len() > 1,
                    queue: dirty.into(),
                    then_quit,
                    apply_rest: false,
                    remembered: None,
                    saving: false,
                });
                if then_quit {
                    tasks.push(self.show_window());
                }
            }
        }
        Task::batch(tasks)
    }

    /// Answers the question about the first tab in the closing queue.
    fn decide(&mut self, choice: Choice) -> Task<Message> {
        let Some(c) = &mut self.closing else {
            return Task::none();
        };
        let Some(&id) = c.queue.front() else {
            return self.next_to_close();
        };
        if c.apply_rest {
            c.remembered = Some(choice);
        }
        match choice {
            Choice::Discard => {
                c.queue.pop_front();
                let close = self.close_tab(id);
                Task::batch([close, self.next_to_close()])
            }
            Choice::Save => {
                c.saving = true;
                self.save(id, false, AfterSave::CloseAndContinue)
            }
        }
    }

    /// Moves on to the next tab to close, or finishes (and quits if asked).
    fn next_to_close(&mut self) -> Task<Message> {
        let Some(c) = &mut self.closing else {
            return Task::none();
        };
        let tabs = &self.tabs;
        c.queue.retain(|id| tabs.iter().any(|t| t.id == *id));
        c.saving = false;
        match c.queue.front() {
            None => {
                let quit = c.then_quit;
                self.closing = None;
                if quit { self.quit() } else { Task::none() }
            }
            Some(&id) => {
                self.active = id;
                match c.remembered {
                    Some(choice) => self.decide(choice),
                    None => Task::none(),
                }
            }
        }
    }

    fn close_tab(&mut self, id: u64) -> Task<Message> {
        let Some(pos) = self.tabs.iter().position(|t| t.id == id) else {
            return Task::none();
        };
        let mut tab = self.tabs.remove(pos);
        let close = close_conn(tab.take_connection());
        if self.active == id && !self.tabs.is_empty() {
            self.active = self.tabs[pos.min(self.tabs.len() - 1)].id;
        }
        close
    }

    /// Where file dialogs start: the active tab's folder.
    fn file_dir(&self) -> Option<PathBuf> {
        let tab = self.tabs.iter().find(|t| t.id == self.active)?;
        tab.path.as_ref()?.parent().map(PathBuf::from)
    }

    /// Saves a tab to its file, asking for one if it has none (or `ask`).
    fn save(&mut self, id: u64, ask: bool, after: AfterSave) -> Task<Message> {
        let dir = self.file_dir();
        let Some(tab) = self.tab(id) else {
            return Task::none();
        };
        match (&tab.path, ask) {
            (Some(path), false) => {
                let path = path.clone();
                self.write_tab(id, path, after)
            }
            _ => {
                let name = match &tab.path {
                    Some(p) => files::display_name(p),
                    None => format!("{}.sql", tab.title),
                };
                Task::perform(files::pick_save(dir, name), move |p| Message::SavePicked(id, p, after))
            }
        }
    }

    fn write_tab(&mut self, id: u64, path: PathBuf, after: AfterSave) -> Task<Message> {
        let Some(tab) = self.tab(id) else {
            return Task::none();
        };
        if let Err(e) = files::write(&path, &tab.editor.text()) {
            self.status = Status::Error(e);
            // Don't close anything else after a failed save.
            self.closing = None;
            return Task::none();
        }
        tab.title = files::display_name(&path);
        tab.dirty = false;
        let saved = format!("Saved {}", path.display());
        tab.path = Some(path);
        self.status = Status::Info(saved);
        match after {
            AfterSave::Nothing => Task::none(),
            AfterSave::CloseAndContinue => {
                if let Some(c) = &mut self.closing {
                    c.queue.retain(|i| *i != id);
                }
                let close = self.close_tab(id);
                Task::batch([close, self.next_to_close()])
            }
        }
    }

    /// Closes every connection, then exits.
    fn quit(&mut self) -> Task<Message> {
        // Saves the window geometry.
        self.save_settings();
        self.end_session().chain(iced::exit())
    }

    fn save_settings(&mut self) {
        if let Err(e) = self.settings.save() {
            self.status = Status::Error(e);
        }
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
            Message::OpenSettings => self.settings_open = true,
            Message::CloseSettings => self.settings_open = false,
            Message::SetTheme(choice) => {
                self.settings.theme = choice;
                self.save_settings();
            }
            Message::SetEditorFontSize(size) => {
                self.settings.editor_font_size = size;
                self.save_settings();
            }
            Message::SetPageSize(size) => {
                self.settings.page_size = size;
                for tab in &mut self.tabs {
                    tab.page = 0;
                }
                self.save_settings();
            }
            Message::SetCompleteWhileTyping(on) => {
                self.settings.complete_while_typing = on;
                self.save_settings();
            }
            Message::SetMinimizeToTray(on) => {
                self.settings.minimize_to_tray = on;
                self.tray = if on { TrayStatus::Starting } else { TrayStatus::Off };
                self.save_settings();
            }
            Message::Tray(event) => match event {
                TrayEvent::Ready => self.tray = TrayStatus::Ready,
                TrayEvent::Unavailable(e) => self.tray = TrayStatus::Unavailable(e),
                TrayEvent::Toggle if self.window.is_none() => return self.show_window(),
                TrayEvent::Toggle => return self.hide_window(),
                TrayEvent::Show => return self.show_window(),
                TrayEvent::Quit => return self.request_quit(),
            },
            Message::CloseRequested => {
                if self.settings.minimize_to_tray && self.tray == TrayStatus::Ready {
                    return self.hide_window();
                }
                return self.request_quit();
            }
            Message::Resized(size) => {
                self.window_size = size;
                return self.check_maximized(Some(size), None);
            }
            Message::Moved(position) => return self.check_maximized(None, Some(position)),
            Message::Geometry(size, position, maximized) => {
                // Keep the normal geometry, so un-maximizing after a restart
                // goes back to it.
                let geometry = &mut self.settings.window;
                geometry.maximized = maximized;
                if !maximized {
                    if let Some(size) = size {
                        (geometry.width, geometry.height) = (size.width, size.height);
                    }
                    if let Some(p) = position {
                        (geometry.x, geometry.y) = (Some(p.x), Some(p.y));
                    }
                }
            }
            Message::DragStart(drag) => {
                let start = match drag {
                    Drag::Sidebar => self.settings.sidebar_width,
                    Drag::Editor => self.settings.editor_height,
                };
                self.drag = Some(DragState { drag, anchor: None, start });
            }
            Message::DragMove(p) => {
                if let Some(d) = &mut self.drag {
                    // The first move after the press becomes the anchor point.
                    let anchor = *d.anchor.get_or_insert(p);
                    match d.drag {
                        Drag::Sidebar => {
                            self.settings.sidebar_width = (d.start + p.x - anchor.x).clamp(SIDEBAR_MIN, SIDEBAR_MAX)
                        }
                        Drag::Editor => self.settings.editor_height = (d.start + p.y - anchor.y).clamp(60.0, 2000.0),
                    }
                }
            }
            Message::DragEnd => {
                if self.drag.take().is_some() {
                    self.save_settings();
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
                self.schemas.clear();
                if let Some(explorer) = &mut self.explorer {
                    let requests = explorer.refresh();
                    return Task::batch(requests.into_iter().map(|r| self.fetch(r)));
                }
            }
            Message::SelectTop(database, obj) => {
                let Some(session) = &self.session else {
                    return Task::none();
                };
                let sql = catalog::select_top_script(session.profile.driver, &obj);
                let id = self.open_tab(Some(obj.qualified_name()), &sql, Some(database));
                return self.run_tab(id);
            }
            Message::Script(database, obj) => {
                let Some(session) = &self.session else {
                    return Task::none();
                };
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
            Message::CloseTab(id) => return self.close_tabs(vec![id], false),
            Message::CloseOthers(keep) => {
                self.active = keep;
                let ids = self.tabs.iter().map(|t| t.id).filter(|id| *id != keep).collect();
                return self.close_tabs(ids, false);
            }
            Message::CloseToRight(id) => {
                let ids = self.tabs.iter().map(|t| t.id).skip_while(|t| *t != id).skip(1).collect();
                return self.close_tabs(ids, false);
            }
            Message::CloseAll => {
                let ids = self.tabs.iter().map(|t| t.id).collect();
                return self.close_tabs(ids, false);
            }
            Message::CopyPath(path) => return iced::clipboard::write(path),
            Message::OpenFile => {
                let dir = self.file_dir();
                return Task::perform(files::pick_open(dir), Message::OpenPaths);
            }
            Message::OpenPaths(paths) => {
                let mut tasks = Vec::new();
                for path in paths {
                    // A file that's already open just gets focus.
                    if let Some(tab) = self.tabs.iter().find(|t| t.path.as_ref() == Some(&path)) {
                        self.active = tab.id;
                        continue;
                    }
                    tasks.push(Task::perform(files::read(path.clone()), move |r| Message::FileLoaded(path.clone(), r)));
                }
                return Task::batch(tasks);
            }
            Message::FileLoaded(path, result) => match result {
                Ok(text) => {
                    // Replace the active tab if it's an untouched scratch tab.
                    let reusable = self.active_tab().filter(|t| {
                        t.path.is_none() && !t.dirty && t.outcome.is_none() && t.running.is_none() && t.conn.is_none()
                    });
                    let id = match reusable {
                        Some(tab) => {
                            tab.editor = iced_code::Content::with_text(&text);
                            tab.id
                        }
                        None => self.open_tab(None, &text, None),
                    };
                    let tab = self.tab(id).expect("just opened");
                    tab.title = files::display_name(&path);
                    tab.path = Some(path);
                    self.status = Status::Idle;
                }
                Err(e) => self.status = Status::Error(e),
            },
            Message::Save => return self.save(self.active, false, AfterSave::Nothing),
            Message::SaveAs => return self.save(self.active, true, AfterSave::Nothing),
            Message::SavePicked(id, path, after) => match path {
                Some(path) => return self.write_tab(id, path, after),
                // The save dialog was cancelled: back to the question.
                None => {
                    if let Some(c) = &mut self.closing {
                        c.saving = false;
                        c.remembered = None;
                    }
                }
            },
            Message::ConfirmSave => return self.decide(Choice::Save),
            Message::ConfirmDiscard => return self.decide(Choice::Discard),
            Message::ConfirmApplyRest(on) => {
                if let Some(c) = &mut self.closing {
                    c.apply_rest = on;
                }
            }
            Message::ConfirmDiscardAll => {
                let Some(c) = self.closing.take() else {
                    return Task::none();
                };
                let mut tasks: Vec<Task<Message>> = c.queue.into_iter().map(|id| self.close_tab(id)).collect();
                if c.then_quit {
                    tasks.push(self.quit());
                }
                return Task::batch(tasks);
            }
            Message::ConfirmEach => {
                if let Some(c) = &mut self.closing {
                    c.overview = false;
                }
            }
            Message::ConfirmCancel => self.closing = None,
            Message::ModifiersChanged(modifiers) => self.modifiers = modifiers,
            Message::OpenFind(replace) => {
                let Some(tab) = self.active_tab() else {
                    return Task::none();
                };
                tab.completion = None;
                let find = tab.find.get_or_insert_with(FindBar::default);
                find.replace = replace;
                // Search for the selected text, like most editors.
                if let Some(selected) = tab.editor.selection().filter(|s| !s.is_empty() && !s.contains('\n')) {
                    find.query = selected;
                }
                let search = find.search();
                tab.editor.set_search(Some(search));
                let id = if replace && !find.query.is_empty() { tab.replace_id.clone() } else { tab.find_id.clone() };
                return Task::batch([operation::focus(id.clone()), operation::select_all(id)]);
            }
            Message::CloseFind => {
                let Some(tab) = self.active_tab() else {
                    return Task::none();
                };
                if tab.find.take().is_none() {
                    return Task::none();
                }
                tab.editor.set_search(None);
                return operation::focus(tab.editor_id.clone());
            }
            Message::FindInput(query) => self.update_find(|f| f.query = query),
            Message::ToggleMatchCase => self.update_find(|f| f.match_case = !f.match_case),
            Message::ToggleWholeWord => self.update_find(|f| f.whole_word = !f.whole_word),
            Message::ReplaceInput(replacement) => {
                if let Some(find) = self.active_tab().and_then(|t| t.find.as_mut()) {
                    find.replacement = replacement;
                }
            }
            Message::ToggleReplace => {
                let Some(tab) = self.active_tab() else {
                    return Task::none();
                };
                let Some(find) = &mut tab.find else {
                    return Task::none();
                };
                find.replace = !find.replace;
                let id = if find.replace { tab.replace_id.clone() } else { tab.find_id.clone() };
                return operation::focus(id);
            }
            Message::FindSubmit => {
                let previous = self.modifiers.shift();
                if let Some(tab) = self.active_tab() {
                    let _ = if previous { tab.editor.find_previous() } else { tab.editor.find_next() };
                }
            }
            Message::FindNext | Message::FindPrevious => {
                let Some(tab) = self.active_tab() else {
                    return Task::none();
                };
                if tab.find.is_none() {
                    return self.update(Message::OpenFind(false));
                }
                let _ = if matches!(message, Message::FindNext) {
                    tab.editor.find_next()
                } else {
                    tab.editor.find_previous()
                };
            }
            Message::Replace => {
                if let Some(tab) = self.active_tab()
                    && let Some(find) = &tab.find
                {
                    let replacement = find.replacement.clone();
                    tab.dirty |= tab.editor.replace(&replacement);
                }
            }
            Message::ReplaceAll => {
                let Some(tab) = self.active_tab() else {
                    return Task::none();
                };
                let Some(find) = &tab.find else {
                    return Task::none();
                };
                let replacement = find.replacement.clone();
                let count = tab.editor.replace_all(&replacement);
                tab.dirty |= count > 0;
                self.status = Status::Info(match count {
                    1 => "Replaced 1 occurrence.".into(),
                    n => format!("Replaced {n} occurrences."),
                });
            }
            Message::Edit(action) => {
                let while_typing = self.settings.complete_while_typing;
                let Some(tab) = self.active_tab() else {
                    return Task::none();
                };
                // Typing a name (or a dot) updates open suggestions, or opens
                // them if "suggest while typing" is on. Anything else that
                // moves the cursor or edits text closes them.
                let open = tab.completion.is_some() || while_typing;
                let typing = match &action {
                    Action::Edit(Edit::Insert(c)) => {
                        open && (complete::is_word_char(*c) || matches!(c, '.' | '[' | '"'))
                    }
                    Action::Edit(
                        Edit::Backspace | Edit::Delete | Edit::DeleteWordBackward | Edit::DeleteWordForward,
                    ) => tab.completion.is_some(),
                    _ => false,
                };
                let keep = matches!(action, Action::Scroll { .. });
                tab.dirty |= action.is_edit();
                tab.editor.perform(action);
                if typing {
                    let forced = tab.completion.as_ref().is_some_and(|c| c.forced);
                    return self.refresh_completion(forced);
                }
                if !keep {
                    tab.completion = None;
                }
            }
            Message::TriggerCompletion => return self.refresh_completion(true),
            Message::CompletionMove(step) => {
                if let Some(c) = self.active_tab().and_then(|t| t.completion.as_mut()) {
                    c.select_next(step);
                }
            }
            Message::AcceptCompletion(index) => {
                let Some(tab) = self.active_tab() else {
                    return Task::none();
                };
                let Some(c) = tab.completion.take() else {
                    return Task::none();
                };
                let Some(item) = c.items.get(index.unwrap_or(c.selected)) else {
                    return Task::none();
                };
                for _ in 0..c.replace {
                    tab.editor.perform(Action::Edit(Edit::Backspace));
                }
                tab.editor.perform(Action::Edit(Edit::Paste(Arc::new(item.insert.clone()))));
                tab.dirty = true;
                // Clicking the popup took focus away from the editor.
                return iced::widget::operation::focus(tab.editor_id.clone());
            }
            Message::CloseCompletion => {
                if let Some(tab) = self.active_tab() {
                    tab.completion = None;
                }
            }
            Message::SchemaLoaded(database, result) => {
                let failed = result.as_ref().err().cloned();
                self.schemas.insert(
                    database,
                    match result {
                        Ok(info) => Load::Loaded(info),
                        Err(e) => Load::Failed(e),
                    },
                );
                if let Some(e) = failed {
                    self.status = Status::Error(format!("Autocomplete could not load the schema: {e}"));
                }
                // Show the suggestions that were waiting for it.
                if let Some(tab) = self.active_tab()
                    && std::mem::take(&mut tab.completion_pending)
                {
                    let forced = tab.completion.as_ref().is_some_and(|c| c.forced);
                    return self.refresh_completion(forced);
                }
            }
            Message::Run => return self.run_tab(self.active),
            Message::Cancel => {
                // Esc closes an open dialog first.
                if self.closing.take().is_some() {
                    return Task::none();
                }
                if self.settings_open {
                    self.settings_open = false;
                    return Task::none();
                }
                if self.dialog_open {
                    self.dialog_open = false;
                    return Task::none();
                }
                let Some(tab) = self.active_tab() else {
                    return Task::none();
                };
                let Some(handle) = tab.running.take() else {
                    return Task::none();
                };
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
                        tab.current_db = c.database.clone();
                        tab.conn = Some(c.conn);
                        return Task::batch([self.execute(id, sql), self.load_schema(c.database)]);
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
                let Some(tab) = self.tab(id) else {
                    return Task::none();
                };
                let changed_schema = tab.ran_ddl && !outcome.failed;
                tab.finish(outcome);
                // `USE` may have switched databases, and DDL makes the cached schema stale.
                let database = tab.current_db.clone();
                if changed_schema {
                    self.schemas.remove(&database);
                }
                return self.load_schema(database);
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
        let label = match profile.name.trim() {
            "" => format!("{} ({})", profile.host, profile.driver),
            name => format!("{name} ({})", profile.host),
        };
        self.status = Status::Info(format!("Connected. {}", c.server_version));
        let database = c.database.clone();
        self.session = Some(Session { profile, password: self.form.password.clone(), explorer_conn: c.conn });
        self.explorer = Some(Explorer::new(label));
        self.dialog_open = false;
        self.sidebar_open = true;
        close.chain(Task::batch([self.fetch(Request::Databases), self.load_schema(database)]))
    }

    /// Closes the explorer's and every tab's connection.
    fn end_session(&mut self) -> Task<Message> {
        let mut conns: Vec<Connection> = self.tabs.iter_mut().filter_map(|t| t.take_connection()).collect();
        if let Some(s) = self.session.take() {
            conns.push(s.explorer_conn);
        }
        self.explorer = None;
        self.schemas.clear();
        for tab in &mut self.tabs {
            tab.completion = None;
        }
        Task::batch(conns.into_iter().map(|c| close_conn(Some(c))))
    }

    fn fetch(&self, request: Request) -> Task<Message> {
        let Some(session) = &self.session else {
            return Task::none();
        };
        let conn = session.explorer_conn.clone();
        match request {
            Request::Databases => {
                Task::perform(catalog::databases(conn), |r| Message::Explorer(ExplorerMsg::DatabasesLoaded(r)))
            }
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

    /// Loads a database's autocomplete metadata unless it is cached or loading.
    fn load_schema(&mut self, database: String) -> Task<Message> {
        let Some(session) = &self.session else {
            return Task::none();
        };
        if database.is_empty() || self.schemas.contains_key(&database) {
            return Task::none();
        }
        let conn = session.explorer_conn.clone();
        self.schemas.insert(database.clone(), Load::Loading);
        Task::perform(catalog::schema_info(conn, database.clone()), move |r| {
            Message::SchemaLoaded(database.clone(), r.map(Arc::new))
        })
    }

    /// Recomputes the active tab's suggestions for the text at its cursor.
    fn refresh_completion(&mut self, forced: bool) -> Task<Message> {
        let Some(session) = &self.session else {
            return Task::none();
        };
        let driver = session.profile.driver;
        let default_db = session.profile.database.clone();
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == self.active) else {
            return Task::none();
        };
        let database = [&tab.current_db, tab.database.as_ref().unwrap_or(&default_db)]
            .into_iter()
            .find(|d| !d.is_empty())
            .cloned()
            .unwrap_or_default();
        let schema = match self.schemas.get(&database) {
            Some(Load::Loaded(info)) => Some(info.clone()),
            _ => None,
        };
        let databases = self.explorer.as_ref().map(|e| e.databases()).unwrap_or_default();
        let sources = Sources { driver, schema: schema.as_deref(), databases };
        let (text, cursor) = tab.text_and_cursor();
        tab.completion = complete::complete(&text, cursor, &sources, forced);
        if schema.is_some() {
            return Task::none();
        }
        tab.completion_pending = true;
        self.load_schema(database)
    }

    /// Runs the tab's script, opening the tab's connection first if needed.
    fn run_tab(&mut self, id: u64) -> Task<Message> {
        let Some(session) = &self.session else {
            self.status = Status::Error("Connect to a server first.".into());
            return Task::none();
        };
        let mut profile = session.profile.clone();
        let password = session.password.clone();
        let Some(tab) = self.tab(id) else {
            return Task::none();
        };
        if tab.running.is_some() {
            return Task::none();
        }
        let Some(sql) = tab.sql_to_run() else {
            return Task::none();
        };
        tab.completion = None;
        tab.ran_ddl = changes_schema(&sql);

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
        let Some(tab) = self.tab(id) else {
            return Task::none();
        };
        let Some(conn) = tab.conn.clone() else {
            return Task::none();
        };
        let (task, handle) =
            Task::perform(conn.run(sql), move |outcome| Message::QueryFinished(id, Arc::new(outcome))).abortable();
        tab.running = Some(handle);
        self.status = Status::Info("Executing…".into());
        task
    }

    // ---------------------------------------------------------------- view

    pub fn view(&self, _window: window::Id) -> Element<'_, Message> {
        let main = column![self.tab_strip(), self.active_tab_view()].width(Length::Fill);
        let body = if self.sidebar_open { row![self.sidebar(), splitter(Drag::Sidebar), main] } else { row![main] };
        let base = column![body.height(Length::Fill), horizontal_divider(), self.status_bar()];

        // The root is always the same stack, so showing a popup or dialog
        // doesn't rebuild the widget tree (which would drop editor focus).
        let prompt = self.closing.as_ref().filter(|c| !c.saving);
        let overlay: Element<'_, Message> = if let Some(closing) = prompt {
            opaque(center(self.confirm_view(closing)).style(backdrop))
        } else if self.settings_open {
            opaque(center(preferences::view(&self.settings, &self.tray)).style(backdrop))
        } else if self.dialog_open {
            let live = self.session.as_ref().map(|s| &s.profile);
            opaque(center(self.form.view(live)).style(backdrop))
        } else if let Some(popup) = self.completion_view() {
            popup
        } else {
            space().into()
        };
        stack![base, overlay].into()
    }

    /// The unsaved-changes prompts: an overview when several tabs are
    /// affected, then one question per file.
    fn confirm_view<'a>(&'a self, c: &'a Closing) -> Element<'a, Message> {
        let action = |label: &'static str, msg: Message, style: fn(&Theme, button::Status) -> button::Style| {
            button(text(label).size(style::BODY)).padding([style::S1 + 2.0, style::S3]).style(style).on_press(msg)
        };
        let mut buttons = row![space().width(Length::Fill)].spacing(style::S2);
        buttons = buttons.push(action("Cancel", Message::ConfirmCancel, style::neutral));
        let mut body = column![].spacing(style::S3);

        let title = if c.overview {
            let verb = if c.then_quit { "quit" } else { "close" };
            body = body.push(style::dim(
                text(format!(
                    "You're about to {verb} {} tabs with unsaved changes. Close them all without saving,                      or choose for each file?",
                    c.queue.len()
                ))
                .size(style::BODY),
            ));
            let mut names = column![].spacing(2);
            for id in &c.queue {
                if let Some(tab) = self.tabs.iter().find(|t| t.id == *id) {
                    names = names.push(
                        row![style::dim(icon::file_code().size(12)), text(tab.title.as_str()).size(style::BODY)]
                            .spacing(style::S2)
                            .align_y(Alignment::Center),
                    );
                }
            }
            body = body.push(container(names).padding(iced::Padding { left: style::S2, ..iced::Padding::ZERO }));
            let discard = if c.then_quit { "Quit Without Saving" } else { "Close Without Saving" };
            buttons = buttons.push(action(discard, Message::ConfirmDiscardAll, style::neutral));
            buttons = buttons.push(action("Choose Per File", Message::ConfirmEach, style::success));
            "Unsaved changes".to_string()
        } else {
            let id = c.queue.front().copied();
            let name = self.tabs.iter().find(|t| Some(t.id) == id).map_or("this query", |t| t.title.as_str());
            body = body.push(style::dim(text("Your changes will be lost if you don't save them.").size(style::BODY)));
            let rest = c.queue.len().saturating_sub(1);
            if rest > 0 {
                body = body.push(
                    checkbox(c.apply_rest)
                        .label(format!(
                            "Do the same for the remaining {rest} {}",
                            if rest == 1 { "file" } else { "files" }
                        ))
                        .on_toggle(Message::ConfirmApplyRest)
                        .size(14)
                        .text_size(style::BODY),
                );
            }
            buttons = buttons.push(action("Don't Save", Message::ConfirmDiscard, style::neutral));
            buttons = buttons.push(action("Save", Message::ConfirmSave, style::success));
            format!("Save changes to {name}?")
        };

        let mut header = row![
            style::icon_colored(icon::warning(), Some(|t: &Theme| t.extended_palette().warning.base.color)),
            text(title).size(style::TITLE).font(style::UI_BOLD),
        ]
        .spacing(style::S2)
        .align_y(Alignment::Center);
        if !c.overview && c.total > 1 {
            let n = c.total - c.queue.len() + 1;
            header = header
                .push(space().width(Length::Fill))
                .push(style::dim(text(format!("{n} of {}", c.total)).size(style::SMALL)));
        }
        let content = column![header, body, buttons].spacing(style::S4).padding(style::S4 + 4.0);
        container(content).width(460).style(style::dialog).into()
    }

    fn completion_view(&self) -> Option<Element<'_, Message>> {
        if self.settings_open || self.dialog_open || self.closing.is_some() {
            return None;
        }
        let tab = self.tabs.iter().find(|t| t.id == self.active)?;
        let c = tab.completion.as_ref()?;
        Some(completion_popup(c, tab.editor.caret_bounds()?, self.window_size, self.settings.editor_font_size))
    }

    fn sidebar(&self) -> Element<'_, Message> {
        let connected = self.session.is_some();
        let header = row![
            style::section_title("Object Explorer"),
            space().width(Length::Fill),
            style::tip(style::tool_button(icon::plug().size(style::ICON), None, Some(Message::OpenDialog)), "Connect…"),
            style::tip(
                style::tool_button(
                    icon::refresh().size(style::ICON),
                    None,
                    connected.then_some(Message::RefreshExplorer)
                ),
                "Refresh",
            ),
            style::tip(
                style::tool_button(
                    icon::debug_disconnect().size(style::ICON),
                    None,
                    connected.then_some(Message::Disconnect)
                ),
                "Disconnect",
            ),
        ]
        .spacing(2)
        .padding([style::S1, style::S1])
        .align_y(Alignment::Center);

        let tree: Element<'_, Message> = match &self.explorer {
            Some(explorer) => scrollable(container(explorer.view()).padding([style::S1, style::S1]))
                .direction(scrollable::Direction::Vertical(scrollable::Scrollbar::new().width(6).scroller_width(6)))
                .height(Length::Fill)
                .into(),
            None => container(
                column![
                    style::icon_colored(icon::server(), None).size(32),
                    style::dim(text("Not connected").size(style::BODY)),
                    button(text("Connect…").size(style::BODY))
                        .padding([style::S1 + 2.0, style::S3])
                        .style(style::success)
                        .on_press(Message::OpenDialog),
                ]
                .spacing(style::S3)
                .align_x(Alignment::Center),
            )
            .center(Length::Fill)
            .into(),
        };

        container(column![container(header).padding([0.0, style::S1]), tree])
            .width(self.settings.sidebar_width.clamp(SIDEBAR_MIN, SIDEBAR_MAX))
            .height(Length::Fill)
            .style(style::chrome_panel)
            .into()
    }

    /// Right-click menu for a tab.
    fn tab_menu(&self, tab: &QueryTab) -> Vec<(&'static str, Message)> {
        let mut items = vec![("Close", Message::CloseTab(tab.id))];
        if self.tabs.len() > 1 {
            items.push(("Close Others", Message::CloseOthers(tab.id)));
        }
        if self.tabs.last().is_some_and(|t| t.id != tab.id) {
            items.push(("Close to the Right", Message::CloseToRight(tab.id)));
        }
        items.push(("Close All", Message::CloseAll));
        if let Some(path) = &tab.path {
            items.push(("Copy Path", Message::CopyPath(path.display().to_string())));
        }
        items
    }

    fn tab_strip(&self) -> Element<'_, Message> {
        let toggle_icon = if self.sidebar_open { icon::layout_sidebar_left() } else { icon::layout_sidebar_left_off() };
        let mut strip = row![style::tip(
            style::tool_button(toggle_icon.size(style::ICON), None, Some(Message::ToggleSidebar)),
            "Toggle Object Explorer",
        )]
        .spacing(2)
        .align_y(Alignment::End);
        for tab in &self.tabs {
            let active = tab.id == self.active;
            let lead: Element<'_, Message> = if tab.running.is_some() {
                style::icon_colored(icon::sync(), Some(style::accent_color)).size(12).into()
            } else {
                style::icon_colored(icon::file_code(), if active { Some(style::accent_color) } else { None })
                    .size(12)
                    .into()
            };
            let mut label = row![lead, text(tab.title.as_str()).size(style::BODY)]
                .spacing(style::S1 + 2.0)
                .align_y(Alignment::Center);
            if tab.dirty {
                label = label.push(style::dim(text("●").size(9)));
            }
            // One button for the whole tab, so it highlights as a unit. The
            // close button inside it only brightens its icon on hover.
            let close = button(icon::close().size(12))
                .style(move |t: &Theme, s| tab_close_button(t, s, active))
                .padding([2.0, 2.0])
                .on_press(Message::CloseTab(tab.id));
            let tab_button = button(row![label, close].spacing(style::S2).align_y(Alignment::Center))
                .style(move |t: &Theme, s| tab_button(t, s, active))
                .padding(iced::Padding {
                    top: style::S2 - 1.0,
                    right: style::S2,
                    bottom: style::S2 - 1.0,
                    left: style::S3,
                })
                .on_press(Message::SelectTab(tab.id));
            let menu = self.tab_menu(tab);
            strip = strip.push(ContextMenu::new(tab_button, move || context_menu(&menu)));
        }
        strip = strip.push(
            container(row![
                style::tip(
                    style::tool_button(icon::add().size(style::ICON), None, Some(Message::NewQuery(None))),
                    "New query (Ctrl+N)",
                ),
                style::tip(
                    style::tool_button(icon::folder_opened().size(style::ICON), None, Some(Message::OpenFile)),
                    "Open file (Ctrl+O)",
                ),
            ])
            .padding([2, 0]),
        );
        container(
            scrollable(strip)
                .direction(scrollable::Direction::Horizontal(scrollable::Scrollbar::new().width(3).scroller_width(3))),
        )
        .padding(iced::Padding { top: style::S1, right: style::S2, bottom: 0.0, left: style::S1 })
        .width(Length::Fill)
        .style(style::chrome_panel)
        .into()
    }

    fn active_tab_view(&self) -> Element<'_, Message> {
        match self.tabs.iter().find(|t| t.id == self.active) {
            Some(tab) => tab.view(self.session.is_some(), style::highlighter(&self.current_theme()), &self.settings),
            None => container(
                column![
                    style::dim(icon::file_code().size(32)),
                    style::dim(text("No open queries").size(style::BODY)),
                    button(text("New Query").size(style::BODY))
                        .padding([style::S1 + 2.0, style::S3])
                        .style(style::neutral)
                        .on_press(Message::NewQuery(None)),
                    style::dim(text("Ctrl+N").size(style::SMALL)),
                ]
                .spacing(style::S3)
                .align_x(Alignment::Center),
            )
            .center(Length::Fill)
            .into(),
        }
    }

    fn status_bar(&self) -> Element<'_, Message> {
        let message: Element<'_, Message> = match &self.status {
            Status::Idle => style::dim(text("Ready").size(style::SMALL)).into(),
            Status::Info(s) => text(s.as_str()).size(style::SMALL).into(),
            Status::Error(s) => row![
                style::icon_colored(icon::error(), Some(style::danger_color)).size(12),
                text(s.as_str())
                    .size(style::SMALL)
                    .style(|t: &Theme| text::Style { color: Some(style::danger_color(t)) }),
            ]
            .spacing(style::S1)
            .align_y(Alignment::Center)
            .into(),
        };

        let mut bar = row![message, space().width(Length::Fill)].spacing(style::S4).align_y(Alignment::Center);

        // Result summary for the active tab, like ADS.
        if let Some(outcome) =
            self.tabs.iter().find(|t| t.id == self.active).and_then(|t| t.outcome.as_ref().map(|o| (o, t.result_index)))
        {
            let (o, i) = outcome;
            if let Some(set) = o.results.get(i) {
                bar = bar.push(segment(icon::list_flat(), format!("{} rows", set.rows.len())));
            }
            bar = bar.push(segment(icon::watch(), format!("{:.3}s", o.elapsed.as_secs_f64())));
        }

        bar = bar.push(match &self.session {
            Some(s) => row![
                style::icon_colored(icon::circle_filled(), Some(style::success_color)).size(9),
                text(format!("{} · {}@{}:{}", s.profile.driver, s.profile.user, s.profile.host, s.profile.port))
                    .size(style::SMALL),
            ]
            .spacing(style::S1 + 2.0)
            .align_y(Alignment::Center),
            None => row![style::dim(icon::circle_filled().size(9)), text("Not connected").size(style::SMALL),]
                .spacing(style::S1 + 2.0)
                .align_y(Alignment::Center),
        });

        bar = bar.push(style::tip(
            button(icon::settings_gear().size(12))
                .padding([1.0, style::S1])
                .style(style::flat)
                .on_press(Message::OpenSettings),
            "Settings (Ctrl+,)",
        ));

        container(bar).padding([3.0, style::S3]).width(Length::Fill).style(style::status_bar).into()
    }
}

/// One status-bar section: small icon and text.
fn segment<'a>(glyph: iced::widget::Text<'a>, label: String) -> Element<'a, Message> {
    row![glyph.size(12), text(label).size(style::SMALL)].spacing(style::S1).align_y(Alignment::Center).into()
}

pub fn shortcut(key: &Key, modifiers: keyboard::Modifiers) -> Option<Message> {
    match key {
        Key::Named(Named::F5) => Some(Message::Run),
        Key::Named(Named::Enter) if modifiers.command() => Some(Message::Run),
        Key::Named(Named::Escape) => Some(Message::Cancel),
        Key::Character(c) if modifiers.command() && c.as_str() == "n" => Some(Message::NewQuery(None)),
        Key::Character(c) if modifiers.command() && c.as_str() == "o" => Some(Message::OpenFile),
        Key::Character(c) if modifiers.command() && c.eq_ignore_ascii_case("s") => {
            Some(if modifiers.shift() { Message::SaveAs } else { Message::Save })
        }
        Key::Character(c) if modifiers.command() && c.as_str() == "," => Some(Message::OpenSettings),
        Key::Character(c) if modifiers.command() && c.eq_ignore_ascii_case("f") => Some(Message::OpenFind(false)),
        Key::Character(c) if modifiers.command() && c.eq_ignore_ascii_case("h") => Some(Message::OpenFind(true)),
        Key::Named(Named::F3) => Some(if modifiers.shift() { Message::FindPrevious } else { Message::FindNext }),
        _ => None,
    }
}

/// Whether a script may create, change or drop objects, so cached
/// autocomplete metadata should be reloaded after it runs.
fn changes_schema(sql: &str) -> bool {
    sql.split(|c: char| !c.is_alphanumeric() && c != '_')
        .any(|w| ["create", "alter", "drop", "sp_rename"].iter().any(|k| w.eq_ignore_ascii_case(k)))
}

/// What to do once a save has finished.
#[derive(Debug, Clone, Copy)]
pub enum AfterSave {
    Nothing,
    /// Close the tab and move on to the next one being closed.
    CloseAndContinue,
}

/// Closing tabs that have unsaved changes.
struct Closing {
    /// Tabs still to decide on; the first one is being asked about.
    queue: VecDeque<u64>,
    /// How many there were to begin with, for "2 of 4".
    total: usize,
    /// Quit once every tab is handled.
    then_quit: bool,
    /// Showing the "N tabs have unsaved changes" overview first.
    overview: bool,
    /// "Do the same for the remaining files" is ticked.
    apply_rest: bool,
    /// The choice to repeat for the remaining files.
    remembered: Option<Choice>,
    /// A save dialog is open, so the prompt is hidden.
    saving: bool,
}

#[derive(Debug, Clone, Copy)]
enum Choice {
    Save,
    Discard,
}

fn close_conn(conn: Option<Connection>) -> Task<Message> {
    match conn {
        Some(c) => Task::future(c.close()).discard(),
        None => Task::none(),
    }
}

fn backdrop(_theme: &Theme) -> container::Style {
    container::Style { background: Some(Color::from_rgba(0.0, 0.0, 0.0, 0.55).into()), ..container::Style::default() }
}

/// A whole tab: the active one takes the content area's colour and joins
/// it; inactive ones are dimmed and get a faint fill on hover.
fn tab_button(theme: &Theme, status: button::Status, active: bool) -> button::Style {
    let text = theme.extended_palette().background.base.text;
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    let tab = style::tab(theme, active);
    let background = if active {
        tab.background
    } else if hovered {
        Some(text.scale_alpha(0.06).into())
    } else {
        None
    };
    button::Style {
        background,
        text_color: if active || hovered { text } else { text.scale_alpha(0.55) },
        border: tab.border,
        ..button::Style::default()
    }
}

/// The × in a tab: no background of its own, just a brighter icon on hover.
fn tab_close_button(theme: &Theme, status: button::Status, active: bool) -> button::Style {
    let text = theme.extended_palette().background.base.text;
    let text_color = match status {
        button::Status::Hovered | button::Status::Pressed => text,
        _ if active => text.scale_alpha(0.6),
        _ => text.scale_alpha(0.35),
    };
    button::Style { background: None, text_color, ..button::Style::default() }
}
