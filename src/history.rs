//! The last few queries run, across all tabs. Kept in memory only, so the
//! history is gone when querydeck closes.

use std::collections::VecDeque;
use std::time::Duration;

use chrono::{DateTime, Local};
use iced::widget::{button, column, container, row, scrollable, space, text};
use iced::{Alignment, Element, Length};
use iced_aw::ContextMenu;

use crate::app::Message;
use crate::db::QueryOutcome;
use crate::explorer::context_menu;
use crate::style::{self, icon};

/// How many queries are kept.
pub const LIMIT: usize = 25;

const STATUS_WIDTH: f32 = 16.0;
const TIME_WIDTH: f32 = 60.0;
const DATABASE_WIDTH: f32 = 120.0;
const DURATION_WIDTH: f32 = 70.0;
const ROWS_WIDTH: f32 = 150.0;
const COLUMN_SPACING: f32 = style::S3;

/// How much of a query the list shows.
const PREVIEW_CHARS: usize = 300;

/// What a query returned or changed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Rows {
    /// Result sets returned.
    pub sets: usize,
    /// Rows across all result sets.
    pub returned: usize,
    /// Rows changed, when the driver reports it.
    pub affected: Option<u64>,
}

impl Rows {
    fn describe(&self) -> String {
        let returned = match self.sets {
            0 => None,
            1 => Some(plural(self.returned, "row")),
            n => Some(format!("{} in {n} sets", plural(self.returned, "row"))),
        };
        let affected = self.affected.map(|n| format!("{n} affected"));
        match (returned, affected) {
            (Some(r), Some(a)) => format!("{r}, {a}"),
            (Some(r), None) => r,
            (None, Some(a)) => a,
            (None, None) => "–".into(),
        }
    }
}

fn plural(n: usize, word: &str) -> String {
    if n == 1 { format!("1 {word}") } else { format!("{n} {word}s") }
}

impl RunState {
    /// How a query that ran to the end went.
    pub fn of(outcome: &QueryOutcome) -> Self {
        if outcome.failed {
            return RunState::Failed(Some(outcome.elapsed));
        }
        let rows = Rows {
            sets: outcome.results.len(),
            returned: outcome.results.iter().map(|r| r.rows.len()).sum(),
            affected: outcome.rows_affected,
        };
        RunState::Succeeded(outcome.elapsed, rows)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RunState {
    Running,
    Succeeded(Duration, Rows),
    /// Failed, or couldn't connect (no duration then).
    Failed(Option<Duration>),
    Cancelled,
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub id: u64,
    /// The tab it ran in, to record how it ended.
    pub tab: u64,
    pub sql: String,
    /// The database it ran in, if known.
    pub database: String,
    pub at: DateTime<Local>,
    pub state: RunState,
}

#[derive(Debug, Default)]
pub struct History {
    /// Newest first.
    entries: VecDeque<Entry>,
    next_id: u64,
}

impl History {
    /// Records a query as it starts. Running the same query again in the
    /// same database moves it to the top instead of adding a copy.
    pub fn start(&mut self, tab: u64, sql: &str, database: &str) {
        let sql = sql.trim().to_string();
        self.entries.retain(|e| e.sql != sql || e.database != database || e.state == RunState::Running);
        self.next_id += 1;
        self.entries.push_front(Entry {
            id: self.next_id,
            tab,
            sql,
            database: database.to_string(),
            at: Local::now(),
            state: RunState::Running,
        });
        self.entries.truncate(LIMIT);
    }

    /// Records how the query running in `tab` ended.
    pub fn finish(&mut self, tab: u64, state: RunState) {
        if let Some(entry) = self.entries.iter_mut().find(|e| e.tab == tab && e.state == RunState::Running) {
            entry.state = state;
        }
    }

    /// Marks every running query as cancelled (on disconnect).
    pub fn cancel_running(&mut self) {
        for entry in self.entries.iter_mut().filter(|e| e.state == RunState::Running) {
            entry.state = RunState::Cancelled;
        }
    }

    pub fn get(&self, id: u64) -> Option<&Entry> {
        self.entries.iter().find(|e| e.id == id)
    }

    pub fn clear(&mut self) {
        self.entries.retain(|e| e.state == RunState::Running);
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn view(&self) -> Element<'_, Message> {
        if self.entries.is_empty() {
            return style::placeholder(format!(
                "Queries you run show up here (the last {LIMIT}, until querydeck closes)."
            ));
        }

        let heading = |label: &'static str| style::section_title(label).wrapping(text::Wrapping::None);
        let header = row![
            space().width(STATUS_WIDTH),
            heading("Time").width(TIME_WIDTH),
            heading("Database").width(DATABASE_WIDTH),
            heading("Duration").width(DURATION_WIDTH).align_x(Alignment::End),
            heading("Rows").width(ROWS_WIDTH),
            heading("Query"),
        ]
        .spacing(COLUMN_SPACING)
        .padding([style::S1, style::S2 * 2.0]);

        let mut list = column![].spacing(1).padding([style::S1, style::S2]);
        for entry in &self.entries {
            let status = match entry.state {
                RunState::Running => style::icon_colored(icon::loading(), None),
                RunState::Succeeded(..) => style::icon_colored(icon::pass(), Some(style::success_color)),
                RunState::Failed(_) => style::icon_colored(icon::error(), Some(style::danger_color)),
                RunState::Cancelled => style::icon_colored(icon::circle_slash(), None),
            };
            let duration = match entry.state {
                RunState::Succeeded(d, _) | RunState::Failed(Some(d)) => format!("{:.3}s", d.as_secs_f64()),
                RunState::Cancelled => "cancelled".into(),
                _ => String::new(),
            };
            let rows = match entry.state {
                RunState::Succeeded(_, rows) => rows.describe(),
                _ => String::new(),
            };
            let line = row![
                status.size(style::ICON).width(STATUS_WIDTH),
                style::dim(text(entry.at.format("%H:%M:%S").to_string()).size(style::SMALL).font(style::MONO_FONT))
                    .width(TIME_WIDTH),
                style::dim(text(entry.database.clone()).size(style::SMALL).wrapping(text::Wrapping::None))
                    .width(DATABASE_WIDTH),
                style::dim(text(duration).size(style::SMALL).font(style::MONO_FONT))
                    .width(DURATION_WIDTH)
                    .align_x(Alignment::End),
                style::dim(text(rows).size(style::SMALL).wrapping(text::Wrapping::None)).width(ROWS_WIDTH),
                text(preview(&entry.sql))
                    .size(style::BODY)
                    .font(style::MONO_FONT)
                    .wrapping(text::Wrapping::None)
                    .width(Length::Fill),
            ]
            .spacing(COLUMN_SPACING)
            .align_y(Alignment::Center);
            let item = style::tip(
                button(container(line).clip(true))
                    .width(Length::Fill)
                    .padding([3.0, style::S2])
                    .style(style::flat)
                    .on_press(Message::OpenHistory(entry.id)),
                "Click to open in a new tab, right-click for more",
            );
            let id = entry.id;
            let menu = [
                ("Open in New Tab", Message::OpenHistory(id)),
                ("Insert at Cursor", Message::InsertHistory(id)),
                ("Copy", Message::CopyHistory(id)),
            ];
            list = list.push(ContextMenu::new(item, move || context_menu(&menu)));
        }
        column![header, scrollable(list).width(Length::Fill).height(Length::Fill)].into()
    }
}

/// The query on one line, cut short.
fn preview(sql: &str) -> String {
    let mut out = String::new();
    for word in sql.split_whitespace() {
        if out.chars().count() >= PREVIEW_CHARS {
            out.push('…');
            break;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_newest_and_moves_repeats_to_the_top() {
        let mut history = History::default();
        for i in 0..30 {
            history.start(1, &format!("select {i}"), "db");
            history.finish(1, RunState::Succeeded(Duration::ZERO, Rows::default()));
        }
        assert_eq!(history.entries.len(), LIMIT);
        assert_eq!(history.entries[0].sql, "select 29");

        history.start(2, "  select 20\n", "db");
        assert_eq!(history.entries.len(), LIMIT);
        assert_eq!(history.entries[0].sql, "select 20");
        assert_eq!(history.entries.iter().filter(|e| e.sql == "select 20").count(), 1);

        // Same query in another database is a separate entry.
        history.start(3, "select 29", "other");
        assert_eq!(history.entries.iter().filter(|e| e.sql == "select 29").count(), 2);
    }

    #[test]
    fn finish_updates_the_running_entry_of_that_tab() {
        let mut history = History::default();
        history.start(1, "select 1", "db");
        history.start(2, "select 2", "db");
        history.finish(1, RunState::Failed(None));
        assert_eq!(history.entries[1].state, RunState::Failed(None));
        assert_eq!(history.entries[0].state, RunState::Running);
    }

    #[test]
    fn describes_rows() {
        let rows = |sets, returned, affected| Rows { sets, returned, affected }.describe();
        assert_eq!(rows(1, 1, None), "1 row");
        assert_eq!(rows(2, 40, None), "40 rows in 2 sets");
        assert_eq!(rows(0, 0, Some(3)), "3 affected");
        assert_eq!(rows(1, 5, Some(2)), "5 rows, 2 affected");
        assert_eq!(rows(0, 0, None), "–");
    }

    #[test]
    fn previews_on_one_line() {
        assert_eq!(preview("select *\n  from  t\n"), "select * from t");
    }
}
