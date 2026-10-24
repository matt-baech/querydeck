//! The Settings dialog.

use iced::widget::{Column, button, column, container, pick_list, row, space, text};
use iced::{Alignment, Element, Length};

use crate::app::Message;
use crate::settings::{EDITOR_FONT_SIZES, Settings, ThemeChoice};
use crate::style::{self, icon};

/// Whether the tray icon could be shown, for the "minimize to tray" hint.
#[derive(Debug, Clone, PartialEq)]
pub enum TrayStatus {
    Off,
    Starting,
    Ready,
    Unavailable(String),
}

pub fn view<'a>(settings: &'a Settings, tray: &'a TrayStatus) -> Element<'a, Message> {
    let title = row![
        style::icon_colored(icon::settings_gear(), None),
        text("Settings").size(style::TITLE).font(style::UI_BOLD),
        space().width(Length::Fill),
        style::icon_button(icon::close(), Message::CloseSettings, "Close (Esc)"),
    ]
    .spacing(style::S2)
    .align_y(Alignment::Center);

    // Theme: a segmented control.
    let mut segments = row![].spacing(2);
    for choice in ThemeChoice::ALL {
        let glyph = match choice {
            ThemeChoice::System => icon::color_mode(),
            ThemeChoice::Light => icon::lightbulb(),
            ThemeChoice::Dark => icon::circle_filled(),
        };
        let content = row![glyph.size(12), text(choice.to_string()).size(style::BODY)]
            .spacing(style::S1 + 2.0)
            .align_y(Alignment::Center);
        segments = segments.push(
            button(content)
                .padding([style::S1 + 2.0, style::S3])
                .style(if settings.theme == choice { style::row_selected } else { style::flat })
                .on_press(Message::SetTheme(choice)),
        );
    }
    let appearance = section(
        "Appearance",
        column![setting_row("Theme", "System follows your desktop's light/dark setting.", segments.into())],
    );

    let editor = section(
        "Editor",
        column![
            setting_row(
                "Editor font size",
                "Size of the SQL text in query tabs.",
                pick_list(&EDITOR_FONT_SIZES[..], Some(settings.editor_font_size), Message::SetEditorFontSize)
                    .text_size(style::BODY)
                    .padding([style::S1, style::S2])
                    .width(90)
                    .into(),
            ),
            style::check(
                settings.complete_while_typing,
                "Suggest while typing (otherwise press Ctrl+Space)",
                Message::SetCompleteWhileTyping
            ),
        ]
        .spacing(style::S4),
    );

    let tray_hint: Element<'a, Message> = match tray {
        TrayStatus::Unavailable(e) => row![
            style::icon_colored(icon::warning(), Some(style::warning_color)).size(12),
            style::dim(text(format!("No system tray available, so closing will quit. ({e})")).size(style::SMALL)),
        ]
        .spacing(style::S1)
        .into(),
        _ => style::dim(
            text("litedb keeps running in the system tray. Quit from the tray icon's menu.").size(style::SMALL),
        )
        .into(),
    };
    let window = section(
        "Window",
        column![
            style::check(
                settings.minimize_to_tray,
                "Minimize to tray when closing the window",
                Message::SetMinimizeToTray
            ),
            container(tray_hint).padding(iced::Padding { left: 22.0, ..iced::Padding::ZERO }),
        ]
        .spacing(style::S1),
    );

    let path = crate::settings::config_file("settings.json").map(|p| p.display().to_string()).unwrap_or_default();
    let footer = style::dim(text(format!("Saved automatically to {path}")).size(style::SMALL));

    let body = column![title, appearance, editor, window, space().height(Length::Fill), footer]
        .spacing(style::S4 + 4.0)
        .padding(style::S4 + 4.0);

    container(body).width(560).height(560).style(style::dialog).into()
}

fn section<'a>(title: &'a str, content: Column<'a, Message>) -> Element<'a, Message> {
    column![style::section_title(title), content].spacing(style::S3).into()
}

/// Label and description on the left, control on the right.
fn setting_row<'a>(label: &'a str, description: &'a str, control: Element<'a, Message>) -> Element<'a, Message> {
    row![
        column![text(label).size(style::BODY), style::dim(text(description).size(style::SMALL))]
            .spacing(2)
            .width(Length::Fill),
        control,
    ]
    .spacing(style::S4)
    .align_y(Alignment::Center)
    .into()
}
