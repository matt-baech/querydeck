//! A query tab: SQL editor, its own server connection, and its results.

use std::sync::Arc;

use iced::widget::{button, column, container, mouse_area, row, scrollable, space, table, text, text_editor};
use iced::{Alignment, Element, Font, Length, mouse, task};

use crate::app::{Drag, Message, shortcut};
use crate::db::{self, Connection, QueryOutcome};
use crate::settings::Settings;
use crate::style::{self, icon};

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
        self.pane =
            if outcome.failed || outcome.results.is_empty() { ResultsPane::Messages } else { ResultsPane::Grid };
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

    /// Pane sizes come from the app's settings so they survive tab switches.
    pub fn view<'a>(
        &'a self,
        connected: bool,
        highlighter: iced::highlighter::Theme,
        settings: &Settings,
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
        let toolbar = row![
            run,
            vertical_divider(),
            style::icon_colored(icon::database(), Some(style::accent_color)),
            text(db_label).size(style::BODY),
        ]
        .spacing(style::S2)
        .padding([style::S1, style::S2])
        .align_y(Alignment::Center);

        let editor = text_editor(&self.editor)
            .on_action(Message::Edit)
            .highlight("sql", highlighter)
            .font(style::MONO_FONT)
            .size(settings.editor_font_size)
            .style(style::editor)
            .height(settings.editor_height)
            .padding([style::S2, style::S3])
            .key_binding(|press| {
                if let Some(m) = shortcut(&press.key, press.modifiers) {
                    return Some(text_editor::Binding::Custom(m));
                }
                text_editor::Binding::from_key_press(press)
            });

        column![toolbar, horizontal_divider(), editor, splitter(Drag::Editor), self.results_view(settings.page_size),]
            .into()
    }

    fn results_view(&self, page_size: usize) -> Element<'_, Message> {
        let Some(outcome) = &self.outcome else {
            return container(style::dim(text("Run a query to see results here.").size(style::BODY)))
                .center(Length::Fill)
                .into();
        };

        let tab = |label: String, pane: ResultsPane| -> Element<'_, Message> {
            let active = self.pane == pane;
            let label = text(label).size(style::SMALL).font(style::UI_BOLD);
            let label = if active { label } else { style::dim(label) };
            column![
                button(label)
                    .padding([style::S1 + 2.0, style::S2])
                    .style(style::flat)
                    .on_press(Message::ShowPane(pane)),
                container(space()).height(2).width(Length::Fill).style(if active {
                    style::underline
                } else {
                    transparent
                }),
            ]
            .width(Length::Shrink)
            .into()
        };
        let mut tabs = row![tab("RESULTS".into(), ResultsPane::Grid), tab("MESSAGES".into(), ResultsPane::Messages)]
            .spacing(style::S1)
            .align_y(Alignment::End);

        if self.pane == ResultsPane::Grid && outcome.results.len() > 1 {
            tabs = tabs.push(space().width(style::S4));
            for i in 0..outcome.results.len() {
                tabs = tabs.push(
                    button(text(format!("{}", i + 1)).size(style::SMALL))
                        .padding([2.0, style::S2])
                        .style(if self.result_index == i { style::row_selected } else { style::flat })
                        .on_press(Message::SelectResult(i)),
                );
            }
        }

        let body: Element<'_, Message> = match self.pane {
            ResultsPane::Messages => {
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
            ResultsPane::Grid => match outcome.results.get(self.result_index) {
                None => container(style::dim(text("The query returned no result sets.").size(style::BODY)))
                    .center(Length::Fill)
                    .into(),
                Some(set) => self.grid(set, page_size),
            },
        };

        column![container(tabs).padding([0.0, style::S2]), horizontal_divider(), body].height(Length::Fill).into()
    }

    fn grid<'a>(&'a self, set: &'a db::ResultSet, page_size: usize) -> Element<'a, Message> {
        let page_size = page_size.max(1);
        let pages = set.rows.len().div_ceil(page_size).max(1);
        let page = self.page.min(pages - 1);
        let start = page * page_size;
        let end = (start + page_size).min(set.rows.len());
        let page_rows = &set.rows[start..end];

        let mut columns = vec![
            table::column(style::dim(text("#").size(style::SMALL)), move |(n, _): (usize, &'a Vec<String>)| {
                style::dim(text(n + 1).size(style::SMALL).font(style::MONO_FONT))
            })
            .align_x(Alignment::End),
        ];
        for (ci, name) in set.columns.iter().enumerate() {
            let header = text(if name.is_empty() { "(No column name)" } else { name.as_str() })
                .size(style::BODY)
                .font(style::UI_BOLD);
            let numeric = is_numeric_column(page_rows, ci);
            let col = table::column(header, move |(_, row): (usize, &'a Vec<String>)| {
                let value = row.get(ci).map(String::as_str).unwrap_or_default();
                if value == db::NULL {
                    style::dim(
                        text("NULL")
                            .size(style::BODY)
                            .font(Font { style: iced::font::Style::Italic, ..style::MONO_FONT }),
                    )
                } else {
                    text(truncate(value)).size(style::BODY).font(style::MONO_FONT)
                }
            });
            columns.push(if numeric { col.align_x(Alignment::End) } else { col });
        }
        let rows = page_rows.iter().enumerate().map(move |(i, r)| (start + i, r));
        let grid = table(columns, rows).padding_x(style::S2 + 2.0).padding_y(style::S1);

        let scroll = scrollable(grid)
            .direction(scrollable::Direction::Both {
                vertical: scrollable::Scrollbar::default(),
                horizontal: scrollable::Scrollbar::default(),
            })
            .width(Length::Fill)
            .height(Length::Fill);

        if pages == 1 {
            return scroll.into();
        }
        let pager = row![
            space().width(Length::Fill),
            style::tool_button(
                icon::chevron_left().size(style::ICON),
                None,
                (page > 0).then(|| Message::Page(page - 1))
            ),
            style::dim(
                text(format!("Rows {}–{} of {} · page {}/{}", start + 1, end, set.rows.len(), page + 1, pages))
                    .size(style::SMALL)
            ),
            style::tool_button(
                icon::chevron_right().size(style::ICON),
                None,
                (page + 1 < pages).then(|| Message::Page(page + 1))
            ),
        ]
        .spacing(style::S1)
        .align_y(Alignment::Center);

        column![scroll, horizontal_divider(), container(pager).padding([2.0, style::S2])].into()
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

fn truncate(s: &str) -> String {
    let one_line = s.replace(['\n', '\r'], " ");
    match one_line.char_indices().nth(MAX_CELL_CHARS) {
        Some((i, _)) => format!("{}…", &one_line[..i]),
        None => one_line,
    }
}
