//! A query tab: SQL editor, its own server connection, and its results.

use std::sync::Arc;

use iced::keyboard::{Key, key::Named};
use iced::widget::{
    Id, button, column, container, mouse_area, operation, pin, row, scrollable, space, text, text_input,
};
use iced::{Alignment, Element, Length, Rectangle, Size, Task, mouse, task};
use iced_aw::ContextMenu;
use iced_code::{Action, Binding, Edit, LineEnding, code_editor};

use crate::app::{Drag, Message, shortcut};
use crate::complete::{Completion, Kind};
use crate::db::{self, Connection, QueryOutcome};
use crate::explorer::context_menu;
use crate::history::History;
use crate::settings::Settings;
use crate::style::{self, icon};

/// The find (and replace) bar above the editor.
#[derive(Debug, Clone, Default)]
pub struct FindBar {
    pub query: String,
    pub replacement: String,
    pub match_case: bool,
    pub whole_word: bool,
    /// The replace row is showing.
    pub replace: bool,
}

impl FindBar {
    pub fn search(&self) -> iced_code::Search {
        iced_code::Search { text: self.query.clone(), match_case: self.match_case, whole_word: self.whole_word }
    }
}

/// Messages handled by the active tab on its own.
#[derive(Debug, Clone)]
pub enum TabMsg {
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
    ShowPane(ResultsPane),
    SelectResult(usize),
    /// Copies the selected cells; `true` adds the column titles.
    CopyCells(bool),
    SelectAllCells,
    CompletionMove(isize),
    /// Insert the given suggestion, or the selected one.
    AcceptCompletion(Option<usize>),
    CloseCompletion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultsPane {
    Grid,
    Messages,
    History,
}

pub struct QueryTab {
    pub id: u64,
    pub title: String,
    /// The file the tab was opened from or saved to.
    pub path: Option<std::path::PathBuf>,
    /// Edited since it was opened or last saved.
    pub dirty: bool,
    pub editor: iced_code::Content,
    pub editor_id: Id,
    /// The open autocomplete popup.
    pub completion: Option<Completion>,
    pub find: Option<FindBar>,
    pub find_id: Id,
    pub replace_id: Id,
    /// Autocomplete is waiting for the database's metadata to load.
    pub completion_pending: bool,
    /// The last script run may have changed the schema (CREATE, ALTER, DROP).
    pub ran_ddl: bool,
    /// Database the tab's connection opens in (None: the profile's default).
    pub database: Option<String>,
    /// Opened on the first run, so each tab has its own session like in SSMS.
    pub conn: Option<Connection>,
    /// The session's current database as last reported by the server.
    pub current_db: String,
    /// Connect-or-run task in flight.
    pub running: Option<task::Handle>,
    pub outcome: Option<Arc<QueryOutcome>>,
    pub pane: ResultsPane,
    pub result_index: usize,
    /// One grid per result set: its columns, scroll, widths and selection.
    pub grids: Vec<(Vec<iced_grid::Column>, iced_grid::State)>,
}

impl QueryTab {
    pub fn new(id: u64, title: String, sql: &str, database: Option<String>) -> Self {
        Self {
            id,
            title,
            path: None,
            dirty: false,
            editor: iced_code::Content::with_text(sql),
            editor_id: Id::unique(),
            completion: None,
            find: None,
            find_id: Id::unique(),
            replace_id: Id::unique(),
            completion_pending: false,
            ran_ddl: false,
            current_db: database.clone().unwrap_or_default(),
            database,
            conn: None,
            running: None,
            outcome: None,
            pane: ResultsPane::Grid,
            result_index: 0,
            grids: Vec::new(),
        }
    }

    /// `shift` is whether Shift is held, for Shift+Enter in the find box.
    pub fn update(&mut self, msg: TabMsg, shift: bool) -> Task<Message> {
        match msg {
            TabMsg::OpenFind(replace) => return self.open_find(replace),
            TabMsg::CloseFind if self.find.take().is_some() => {
                self.editor.set_search(None);
                return operation::focus(self.editor_id.clone());
            }
            TabMsg::CloseFind => {}
            TabMsg::FindInput(query) => self.change_find(|f| f.query = query),
            TabMsg::ToggleMatchCase => self.change_find(|f| f.match_case = !f.match_case),
            TabMsg::ToggleWholeWord => self.change_find(|f| f.whole_word = !f.whole_word),
            TabMsg::ReplaceInput(replacement) => {
                if let Some(find) = &mut self.find {
                    find.replacement = replacement;
                }
            }
            TabMsg::ToggleReplace => {
                if let Some(find) = &mut self.find {
                    find.replace = !find.replace;
                    return operation::focus(if find.replace { &self.replace_id } else { &self.find_id }.clone());
                }
            }
            TabMsg::FindSubmit | TabMsg::FindNext | TabMsg::FindPrevious if self.find.is_none() => {
                return self.open_find(false);
            }
            TabMsg::FindSubmit if shift => _ = self.editor.find_previous(),
            TabMsg::FindSubmit | TabMsg::FindNext => _ = self.editor.find_next(),
            TabMsg::FindPrevious => _ = self.editor.find_previous(),
            TabMsg::Replace => {
                if let Some(find) = &self.find {
                    self.dirty |= self.editor.replace(&find.replacement);
                }
            }
            TabMsg::ShowPane(pane) => self.pane = pane,
            TabMsg::SelectResult(i) => self.result_index = i,
            TabMsg::SelectAllCells => {
                if let Some((_, grid)) = self.grids.get(self.result_index) {
                    grid.select(iced_grid::Target::All, false);
                }
            }
            TabMsg::CopyCells(headers) => {
                let set = self.outcome.as_ref().and_then(|o| o.results.get(self.result_index));
                if let (Some(set), Some((columns, grid))) = (set, self.grids.get(self.result_index))
                    && let Some(text) = grid.selected_text(columns, &set.rows, headers)
                {
                    return iced::clipboard::write(text);
                }
            }
            TabMsg::CompletionMove(step) => {
                if let Some(c) = &mut self.completion {
                    c.select_next(step);
                }
            }
            TabMsg::AcceptCompletion(index) => {
                let Some(c) = self.completion.take() else {
                    return Task::none();
                };
                let Some(item) = c.items.get(index.unwrap_or(c.selected)) else {
                    return Task::none();
                };
                for _ in 0..c.replace {
                    self.editor.perform(Action::Edit(Edit::Backspace));
                }
                // Clicking the popup took focus away from the editor.
                return self.paste(item.insert.clone());
            }
            TabMsg::CloseCompletion => self.completion = None,
        }
        Task::none()
    }

    /// Opens the find bar, searching for the selected text like most editors.
    fn open_find(&mut self, replace: bool) -> Task<Message> {
        self.completion = None;
        let find = self.find.get_or_insert_with(FindBar::default);
        find.replace = replace;
        if let Some(selected) = self.editor.selection().filter(|s| !s.is_empty() && !s.contains('\n')) {
            find.query = selected;
        }
        self.editor.set_search(Some(find.search()));
        let id = if replace && !find.query.is_empty() { &self.replace_id } else { &self.find_id };
        Task::batch([operation::focus(id.clone()), operation::select_all(id.clone())])
    }

    /// Changes the find bar, then searches again from the cursor so the
    /// selection follows what's typed.
    fn change_find(&mut self, change: impl FnOnce(&mut FindBar)) {
        if let Some(find) = &mut self.find {
            change(find);
            self.editor.set_search(Some(find.search()));
            let _ = self.editor.find_from_cursor();
        }
    }

    /// Replaces every match; returns how many, if the find bar is open.
    pub fn replace_all(&mut self) -> Option<usize> {
        let count = self.editor.replace_all(&self.find.as_ref()?.replacement);
        self.dirty |= count > 0;
        Some(count)
    }

    /// Inserts text at the cursor and gives the editor focus.
    pub fn paste(&mut self, text: String) -> Task<Message> {
        self.completion = None;
        self.editor.perform(Action::Edit(Edit::Paste(Arc::new(text))));
        self.dirty = true;
        operation::focus(self.editor_id.clone())
    }

    /// The selected text if any, otherwise the whole script.
    pub fn sql_to_run(&self) -> Option<String> {
        let sql = self.editor.selection().filter(|s| !s.trim().is_empty()).unwrap_or_else(|| self.editor.text());
        (!sql.trim().is_empty()).then_some(sql)
    }

    /// The editor's text and the cursor's byte offset into it.
    pub fn text_and_cursor(&self) -> (String, usize) {
        let position = self.editor.cursor().position;
        let mut text = String::new();
        let mut cursor = 0;
        let mut lines = self.editor.lines().enumerate().peekable();
        while let Some((i, line)) = lines.next() {
            if i == position.line {
                cursor = text.len() + position.column.min(line.text.len());
            }
            text.push_str(&line.text);
            if lines.peek().is_some() {
                text.push_str(if line.ending == LineEnding::None { "\n" } else { line.ending.as_str() });
            }
        }
        (text, cursor)
    }

    pub fn finish(&mut self, outcome: Arc<QueryOutcome>) {
        self.running = None;
        self.pane =
            if outcome.failed || outcome.results.is_empty() { ResultsPane::Messages } else { ResultsPane::Grid };
        self.result_index = 0;
        self.grids = outcome.results.iter().map(|set| (grid_columns(set), iced_grid::State::new())).collect();
        if let Some(db) = &outcome.current_database {
            self.current_db = db.clone();
        }
        self.outcome = Some(outcome);
    }

    /// Drops the tab's connection (e.g. after disconnecting from the server).
    pub fn take_connection(&mut self) -> Option<Connection> {
        if let Some(h) = self.running.take() {
            h.abort();
        }
        self.conn.take()
    }

    /// Pane sizes come from the app's settings so they survive tab switches.
    pub fn view<'a>(
        &'a self,
        connected: bool,
        highlighter: iced::highlighter::Theme,
        settings: &Settings,
        history: &'a History,
    ) -> Element<'a, Message> {
        let run: Element<'_, Message> = if self.running.is_some() {
            style::tip(
                style::tool_button(
                    style::icon_colored(icon::debug_stop(), Some(style::danger_color)),
                    Some("Cancel"),
                    Some(Message::Cancel),
                ),
                "Cancel query (Esc)",
            )
            .into()
        } else {
            style::tip(
                style::tool_button(
                    style::icon_colored(icon::play(), Some(style::success_color)),
                    Some("Run"),
                    connected.then_some(Message::Run),
                ),
                "Run script or selection (F5, Ctrl+Enter)",
            )
            .into()
        };
        let db_label = if self.current_db.is_empty() { "default database" } else { self.current_db.as_str() };
        let save = style::icon_button(icon::save(), Message::Save, "Save (Ctrl+S)");
        let toolbar = row![
            run,
            save,
            vertical_divider(),
            style::icon_colored(icon::database(), Some(style::accent_color)),
            text(db_label).size(style::BODY),
        ]
        .spacing(style::S2)
        .padding([style::S1, style::S2])
        .align_y(Alignment::Center);

        let popup = self.completion.is_some();
        let editor = code_editor(&self.editor)
            .id(self.editor_id.clone())
            .on_action(Message::Edit)
            .highlight("sql", highlighter)
            .font(style::MONO_FONT)
            .size(settings.editor_font_size)
            .style(style::editor)
            .height(settings.editor_height)
            .padding([style::S2, style::S3])
            .comment_token("--")
            .key_binding(move |press| {
                if let Some(m) = completion_key(&press.key, press.modifiers, popup) {
                    return Some(Binding::Custom(m));
                }
                if let Some(m) = shortcut(&press.key, press.modifiers) {
                    return Some(Binding::Custom(m));
                }
                Binding::from_key_press(press)
            });

        let mut layout = column![toolbar, horizontal_divider()];
        if let Some(find) = &self.find {
            layout = layout.push(self.find_view(find)).push(horizontal_divider());
        }
        layout.push(editor).push(splitter(Drag::Editor)).push(self.results_view(history)).into()
    }

    fn find_view<'a>(&'a self, find: &'a FindBar) -> Element<'a, Message> {
        const INPUT_WIDTH: f32 = 280.0;
        let input = |id: &Id, placeholder: &str, value: &str| {
            text_input(placeholder, value)
                .id(id.clone())
                .size(style::BODY)
                .padding([3.0, style::S2])
                .style(style::input)
        };
        let toggle = |glyph: text::Text<'a>, on: bool, msg: Message, label: &'a str| {
            style::tip(
                button(glyph.size(style::ICON))
                    .padding([style::S1, style::S1 + 2.0])
                    .style(if on { style::row_selected } else { style::flat })
                    .on_press(msg),
                label,
            )
        };

        let has_query = !find.query.is_empty();
        let count = self.editor.match_count();
        let summary = match self.editor.current_match() {
            _ if !has_query => String::new(),
            _ if count == 0 => "No results".into(),
            Some(i) => format!("{} of {count}", i + 1),
            None => format!("{count} found"),
        };

        let chevron = if find.replace { icon::chevron_down() } else { icon::chevron_right() };
        let find_row = row![
            input(&self.find_id, "Find", &find.query)
                .on_input(|s| Message::Tab(TabMsg::FindInput(s)))
                .on_submit(Message::Tab(TabMsg::FindSubmit))
                .width(INPUT_WIDTH),
            toggle(icon::case_sensitive(), find.match_case, Message::Tab(TabMsg::ToggleMatchCase), "Match case"),
            toggle(icon::whole_word(), find.whole_word, Message::Tab(TabMsg::ToggleWholeWord), "Whole word"),
            style::dim(text(summary).size(style::SMALL)).width(80),
            style::icon_button(
                icon::arrow_up(),
                (count > 0).then_some(Message::Tab(TabMsg::FindPrevious)),
                "Previous match (Shift+F3)"
            ),
            style::icon_button(
                icon::arrow_down(),
                (count > 0).then_some(Message::Tab(TabMsg::FindNext)),
                "Next match (F3)"
            ),
            space().width(Length::Fill),
            style::icon_button(icon::close(), Some(Message::Tab(TabMsg::CloseFind)), "Close (Esc)"),
        ]
        .spacing(style::S1)
        .align_y(Alignment::Center);
        let find_row = row![
            style::icon_button(chevron, Some(Message::Tab(TabMsg::ToggleReplace)), "Toggle replace (Ctrl+H)"),
            container(find_row).width(Length::Fill)
        ]
        .spacing(style::S1)
        .align_y(Alignment::Center);

        let mut bar = column![find_row].spacing(style::S1);
        if find.replace {
            let action = |label: &'a str, msg: Message| {
                button(text(label).size(style::SMALL))
                    .padding([3.0, style::S2])
                    .style(style::neutral)
                    .on_press_maybe((count > 0).then_some(msg))
            };
            bar = bar.push(
                row![
                    space().width(26),
                    input(&self.replace_id, "Replace", &find.replacement)
                        .on_input(|s| Message::Tab(TabMsg::ReplaceInput(s)))
                        .on_submit(Message::Tab(TabMsg::Replace))
                        .width(INPUT_WIDTH),
                    action("Replace", Message::Tab(TabMsg::Replace)),
                    action("Replace All", Message::ReplaceAll),
                ]
                .spacing(style::S1)
                .align_y(Alignment::Center),
            );
        }
        container(bar).padding([style::S1, style::S2]).into()
    }

    fn results_view<'a>(&'a self, history: &'a History) -> Element<'a, Message> {
        let tab = |label: String, pane: ResultsPane| -> Element<'a, Message> {
            let active = self.pane == pane;
            let label = text(label).size(style::SMALL).font(style::UI_BOLD);
            let label = if active { label } else { style::dim(label) };
            column![
                button(label)
                    .padding([style::S1 + 2.0, style::S2])
                    .style(style::flat)
                    .on_press(Message::Tab(TabMsg::ShowPane(pane))),
                container(space()).height(2).width(Length::Fill).style(if active {
                    style::underline
                } else {
                    transparent
                }),
            ]
            .width(Length::Shrink)
            .into()
        };
        let mut tabs = row![
            tab("RESULTS".into(), ResultsPane::Grid),
            tab("MESSAGES".into(), ResultsPane::Messages),
            tab("HISTORY".into(), ResultsPane::History),
        ]
        .spacing(style::S1)
        .align_y(Alignment::End);

        let outcome = self.outcome.as_deref();
        if self.pane == ResultsPane::Grid
            && let Some(outcome) = outcome
            && outcome.results.len() > 1
        {
            tabs = tabs.push(space().width(style::S4));
            for i in 0..outcome.results.len() {
                tabs = tabs.push(
                    button(text(format!("{}", i + 1)).size(style::SMALL))
                        .padding([2.0, style::S2])
                        .style(if self.result_index == i { style::row_selected } else { style::flat })
                        .on_press(Message::Tab(TabMsg::SelectResult(i))),
                );
            }
        }

        if self.pane == ResultsPane::History && !history.is_empty() {
            tabs = tabs.push(space().width(Length::Fill)).push(style::icon_button(
                icon::clear_all(),
                Message::ClearHistory,
                "Clear history",
            ));
        }

        let body: Element<'a, Message> = match (self.pane, outcome) {
            (ResultsPane::History, _) => history.view(),
            (_, None) => style::placeholder("Run a query to see results here."),
            (ResultsPane::Messages, Some(outcome)) => {
                let mut list = column![].spacing(style::S1).padding([style::S2, style::S3]);
                for m in &outcome.messages {
                    let line = text(m).font(style::MONO_FONT).size(style::BODY);
                    list = list.push(if outcome.failed && m.contains("rror") {
                        line.style(|t: &iced::Theme| text::Style { color: Some(style::danger_color(t)) })
                    } else {
                        line
                    });
                }
                list = list.push(style::dim(
                    text(format!("Total execution time: {:.3}s", outcome.elapsed.as_secs_f64())).size(style::BODY),
                ));
                scrollable(list).width(Length::Fill).height(Length::Fill).into()
            }
            (ResultsPane::Grid, Some(outcome)) => match outcome.results.get(self.result_index) {
                None => style::placeholder("The query returned no result sets."),
                Some(set) => self.grid(set),
            },
        };

        column![container(tabs).padding([0.0, style::S2]), horizontal_divider(), body].height(Length::Fill).into()
    }

    fn grid<'a>(&'a self, set: &'a db::ResultSet) -> Element<'a, Message> {
        let Some((columns, state)) = self.grids.get(self.result_index) else {
            return space().into();
        };
        let grid = iced_grid::grid(state, columns, &set.rows)
            .font(style::MONO_FONT)
            .header_font(style::UI_BOLD)
            .text_size(style::BODY)
            .row_height(24.0)
            .style(style::grid)
            .format(|v| if v == db::NULL { iced_grid::Format { dim: true, italic: true } } else { Default::default() });
        let menu = [
            ("Copy", Message::Tab(TabMsg::CopyCells(false))),
            ("Copy with Headers", Message::Tab(TabMsg::CopyCells(true))),
            ("Select All", Message::Tab(TabMsg::SelectAllCells)),
            ("Export to CSV…", Message::ExportCsv),
        ];
        ContextMenu::new(grid, move || context_menu(&menu)).into()
    }
}

/// Column titles, with numbers right-aligned.
fn grid_columns(set: &db::ResultSet) -> Vec<iced_grid::Column> {
    let sample = &set.rows[..set.rows.len().min(1000)];
    (set.columns.iter().enumerate())
        .map(|(i, name)| {
            let title = if name.is_empty() { "(No column name)" } else { name.as_str() };
            let align = if is_numeric_column(sample, i) { iced_grid::Align::End } else { iced_grid::Align::Start };
            iced_grid::Column::new(title).align(align)
        })
        .collect()
}

/// Keys that drive autocomplete. Arrows, Enter, Tab and Esc only belong to
/// the popup while it is open.
fn completion_key(key: &Key, modifiers: iced::keyboard::Modifiers, popup: bool) -> Option<Message> {
    let plain = !modifiers.command() && !modifiers.alt() && !modifiers.shift();
    match key {
        Key::Named(Named::Space) if modifiers.command() => Some(Message::TriggerCompletion),
        Key::Character(c) if modifiers.command() && c.as_str() == " " => Some(Message::TriggerCompletion),
        _ if !popup || !plain => None,
        Key::Named(Named::ArrowDown) => Some(Message::Tab(TabMsg::CompletionMove(1))),
        Key::Named(Named::ArrowUp) => Some(Message::Tab(TabMsg::CompletionMove(-1))),
        Key::Named(Named::PageDown) => Some(Message::Tab(TabMsg::CompletionMove(POPUP_ROWS as isize))),
        Key::Named(Named::PageUp) => Some(Message::Tab(TabMsg::CompletionMove(-(POPUP_ROWS as isize)))),
        Key::Named(Named::Enter | Named::Tab) => Some(Message::Tab(TabMsg::AcceptCompletion(None))),
        Key::Named(Named::Escape) => Some(Message::Tab(TabMsg::CloseCompletion)),
        _ => None,
    }
}

const POPUP_ROWS: usize = 10;
const POPUP_ROW_HEIGHT: f32 = 22.0;
const POPUP_WIDTH: f32 = 380.0;
/// From the popup's left edge to where its labels start.
const POPUP_LABEL_INSET: f32 = style::S1 + style::S2 + style::ICON + style::S2;

/// The autocomplete list, placed under the caret (or above it near the
/// bottom of the window) with labels lined up with the word being typed.
pub fn completion_popup<'a>(c: &'a Completion, caret: Rectangle, window: Size, font_size: f32) -> Element<'a, Message> {
    let start = (c.selected + 1).saturating_sub(POPUP_ROWS);
    let mut list = column![].padding(style::S1);
    for (i, item) in c.items.iter().enumerate().skip(start).take(POPUP_ROWS) {
        let (glyph, color) = kind_icon(item.kind);
        let row = row![
            style::icon_colored(glyph, Some(color)),
            text(item.label.as_str()).font(style::MONO_FONT).size(style::BODY).wrapping(text::Wrapping::None),
            space().width(Length::Fill),
            style::dim(text(item.detail.as_str()).size(style::SMALL).wrapping(text::Wrapping::None)),
        ]
        .spacing(style::S2)
        .align_y(Alignment::Center);
        list = list.push(
            button(row)
                .width(Length::Fill)
                .height(POPUP_ROW_HEIGHT)
                .padding([0.0, style::S2])
                .style(if i == c.selected { style::row_selected } else { style::flat })
                .on_press(Message::Tab(TabMsg::AcceptCompletion(Some(i)))),
        );
    }
    let rows = c.items.len().min(POPUP_ROWS) as f32;
    let height = rows * POPUP_ROW_HEIGHT + 2.0 * style::S1 + 2.0;
    let popup = container(list).width(POPUP_WIDTH).style(style::menu);

    // Hack's advance width is about 0.6 em.
    let typed = c.replace as f32 * font_size * 0.6;
    let x = (caret.x - typed - POPUP_LABEL_INSET).clamp(0.0, (window.width - POPUP_WIDTH - style::S2).max(0.0));
    let below = caret.y + caret.height + 2.0;
    let y = if below + height > window.height - style::S4 { (caret.y - height - 2.0).max(0.0) } else { below };
    pin(popup).x(x).y(y).into()
}

fn kind_icon<'a>(kind: Kind) -> (iced::widget::Text<'a>, fn(&iced::Theme) -> iced::Color) {
    match kind {
        Kind::Column => (icon::symbol_field(), style::accent_color),
        Kind::Alias => (icon::symbol_variable(), style::accent_color),
        Kind::Table => (icon::table(), style::accent_color),
        Kind::View => (icon::eye(), style::accent_color),
        Kind::Schema => (icon::symbol_namespace(), style::dim_text),
        Kind::Procedure => (icon::symbol_method(), crate::explorer::purple),
        Kind::Function => (icon::symbol_operator(), crate::explorer::purple),
        Kind::Database => (icon::database(), style::accent_color),
        Kind::Keyword => (icon::symbol_keyword(), style::dim_text),
    }
}

/// True if every non-NULL value in the column looks like a number.
fn is_numeric_column(rows: &[Vec<String>], col: usize) -> bool {
    let mut seen = false;
    for v in rows.iter().filter_map(|r| r.get(col)).filter(|v| v.as_str() != db::NULL) {
        if v.parse::<f64>().is_err() {
            return false;
        }
        seen = true;
    }
    seen
}

pub fn horizontal_divider<'a>() -> Element<'a, Message> {
    container(space()).height(1).width(Length::Fill).style(style::divider_line).into()
}

pub fn vertical_divider<'a>() -> Element<'a, Message> {
    container(space()).width(1).height(16).style(style::divider_line).into()
}

/// A thin draggable bar between two panes. The line is 1px; the grab area is wider.
pub fn splitter<'a>(drag: Drag) -> Element<'a, Message> {
    let (line, interaction) = match drag {
        Drag::Editor => (
            container(container(space()).height(1).width(Length::Fill).style(style::divider_line))
                .height(GRAB)
                .center_y(GRAB),
            mouse::Interaction::ResizingVertically,
        ),
        Drag::Sidebar => (
            container(container(space()).width(1).height(Length::Fill).style(style::divider_line))
                .width(GRAB)
                .center_x(GRAB),
            mouse::Interaction::ResizingHorizontally,
        ),
    };
    mouse_area(line).interaction(interaction).on_press(Message::DragStart(drag)).into()
}

const GRAB: f32 = 5.0;

fn transparent(_: &iced::Theme) -> container::Style {
    container::Style::default()
}
