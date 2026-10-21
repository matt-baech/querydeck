//! A query tab: SQL editor, its own server connection, and its results.

use std::sync::Arc;

use iced::widget::{button, column, container, row, rule, scrollable, space, table, text, text_editor};
use iced::{Alignment, Element, Font, Length, task};

use crate::app::{Message, shortcut};
use crate::db::{self, Connection, QueryOutcome};
use crate::style;

/// Rows rendered per page in the results grid. The grid builds a widget per
/// cell, so showing everything at once gets slow with large results.
const PAGE_SIZE: usize = 500;
/// Long cell values are cut to this many characters in the grid.
const MAX_CELL_CHARS: usize = 120;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultsPane {
    Grid,
    Messages,
}

pub struct QueryTab {
    pub id: u64,
    pub title: String,
    pub editor: text_editor::Content,
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
    pub page: usize,
}

impl QueryTab {
    pub fn new(id: u64, title: String, sql: &str, database: Option<String>) -> Self {
        Self {
            id,
            title,
            editor: text_editor::Content::with_text(sql),
            current_db: database.clone().unwrap_or_default(),
            database,
            conn: None,
            running: None,
            outcome: None,
            pane: ResultsPane::Grid,
            result_index: 0,
            page: 0,
        }
    }

    /// The selected text if any, otherwise the whole script.
    pub fn sql_to_run(&self) -> Option<String> {
        let sql = self.editor.selection().filter(|s| !s.trim().is_empty()).unwrap_or_else(|| self.editor.text());
        (!sql.trim().is_empty()).then_some(sql)
    }

    pub fn finish(&mut self, outcome: Arc<QueryOutcome>) {
        self.running = None;
        self.pane = if outcome.failed || outcome.results.is_empty() { ResultsPane::Messages } else { ResultsPane::Grid };
        self.result_index = 0;
        self.page = 0;
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

    pub fn view(&self, connected: bool, highlighter: iced::highlighter::Theme) -> Element<'_, Message> {
        let run = if self.running.is_some() {
            button("■ Cancel").on_press(Message::Cancel).style(style::danger)
        } else {
            button("▶ Run").on_press_maybe(connected.then_some(Message::Run)).style(style::success)
        };
        let db_label = if self.current_db.is_empty() { "(default database)" } else { self.current_db.as_str() };
        let toolbar = row![run, text(format!("Database: {db_label}")).size(13).style(text::secondary)]
            .spacing(12)
            .padding([6, 8])
            .align_y(Alignment::Center);

        let editor = text_editor(&self.editor)
            .on_action(Message::Edit)
            .highlight("sql", highlighter)
            .font(Font::MONOSPACE)
            .size(14)
            .style(style::editor)
            .height(Length::FillPortion(2))
            .padding(8)
            .key_binding(|press| {
                if let Some(m) = shortcut(&press.key, press.modifiers) {
                    return Some(text_editor::Binding::Custom(m));
                }
                text_editor::Binding::from_key_press(press)
            });

        column![toolbar, editor, rule::horizontal(1), self.results_view()].into()
    }

    fn results_view(&self) -> Element<'_, Message> {
        let Some(outcome) = &self.outcome else {
            return container(text("Run a query to see results here.").size(14))
                .center(Length::Fill)
                .height(Length::FillPortion(3))
                .into();
        };

        let tab = |label: String, pane: ResultsPane| {
            button(text(label).size(14))
                .style(if self.pane == pane { style::selected } else { button::text })
                .on_press(Message::ShowPane(pane))
        };
        let mut tabs = row![
            tab(format!("Results ({})", outcome.results.len()), ResultsPane::Grid),
            tab("Messages".into(), ResultsPane::Messages),
        ]
        .spacing(4)
        .align_y(Alignment::Center);

        if self.pane == ResultsPane::Grid && outcome.results.len() > 1 {
            tabs = tabs.push(space().width(16));
            for i in 0..outcome.results.len() {
                tabs = tabs.push(
                    button(text(format!("#{}", i + 1)).size(13))
                        .style(if self.result_index == i { style::selected } else { button::text })
                        .on_press(Message::SelectResult(i)),
                );
            }
        }

        let body: Element<'_, Message> = match self.pane {
            ResultsPane::Messages => {
                let mut list = column![].spacing(4).padding(8);
                for m in &outcome.messages {
                    list = list.push(text(m).font(Font::MONOSPACE).size(13));
                }
                list = list.push(text(format!("Total execution time: {:.3}s", outcome.elapsed.as_secs_f64())).size(13));
                scrollable(list).width(Length::Fill).height(Length::Fill).into()
            }
            ResultsPane::Grid => match outcome.results.get(self.result_index) {
                None => container(text("The query returned no result sets.").size(14)).center(Length::Fill).into(),
                Some(set) => self.grid(set),
            },
        };

        column![container(tabs).padding([4, 8]), body].height(Length::FillPortion(3)).into()
    }

    fn grid<'a>(&'a self, set: &'a db::ResultSet) -> Element<'a, Message> {
        let pages = set.rows.len().div_ceil(PAGE_SIZE).max(1);
        let page = self.page.min(pages - 1);
        let start = page * PAGE_SIZE;
        let end = (start + PAGE_SIZE).min(set.rows.len());

        let mut columns = vec![table::column(text("#").size(13), move |(n, _): (usize, &'a Vec<String>)| {
            text(n + 1).size(13).font(Font::MONOSPACE)
        })];
        for (ci, name) in set.columns.iter().enumerate() {
            let header = text(if name.is_empty() { "(No column name)" } else { name.as_str() })
                .size(13)
                .font(Font { weight: iced::font::Weight::Bold, ..Font::default() });
            columns.push(table::column(header, move |(_, row): (usize, &'a Vec<String>)| {
                let value = row.get(ci).map(String::as_str).unwrap_or_default();
                let cell = text(truncate(value)).size(13).font(Font::MONOSPACE);
                if value == db::NULL { cell.style(text::secondary) } else { cell }
            }));
        }
        let rows = set.rows[start..end].iter().enumerate().map(move |(i, r)| (start + i, r));
        let grid = table(columns, rows).padding_x(8).padding_y(3);

        let scroll = scrollable(grid)
            .direction(scrollable::Direction::Both {
                vertical: scrollable::Scrollbar::default(),
                horizontal: scrollable::Scrollbar::default(),
            })
            .width(Length::Fill)
            .height(Length::Fill);

        let mut pager = row![
            text(format!(
                "{} row(s){}",
                set.rows.len(),
                if set.truncated { format!(" — truncated at {}", db::MAX_ROWS) } else { String::new() }
            ))
            .size(13)
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        if pages > 1 {
            pager = pager.push(space().width(Length::Fill)).push(
                row![
                    button("‹").style(button::text).on_press_maybe((page > 0).then(|| Message::Page(page - 1))),
                    text(format!("rows {}–{} · page {}/{}", start + 1, end, page + 1, pages)).size(13),
                    button("›")
                        .style(button::text)
                        .on_press_maybe((page + 1 < pages).then(|| Message::Page(page + 1))),
                ]
                .spacing(6)
                .align_y(Alignment::Center),
            );
        }

        column![scroll, container(pager).padding([4, 8])].into()
    }
}

fn truncate(s: &str) -> String {
    let one_line = s.replace(['\n', '\r'], " ");
    match one_line.char_indices().nth(MAX_CELL_CHARS) {
        Some((i, _)) => format!("{}…", &one_line[..i]),
        None => one_line,
    }
}
