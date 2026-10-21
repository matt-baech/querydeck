//! The app's look: themes, fonts, sizes, spacing, icons and widget styles.
//! Views should take sizes and styles from here rather than hard-coding them.

use std::sync::LazyLock;

use iced::font::Weight;
use iced::theme::Palette;
use iced::widget::{Button, Text, Tooltip, button, container, row, text, tooltip};
use iced::{Background, Border, Color, Element, Font, Theme, border, color};

// ------------------------------------------------------------------ tokens

pub const UI_FONT: Font = Font::with_name("Inter");
pub const UI_BOLD: Font = Font { weight: Weight::Bold, ..UI_FONT };
pub const MONO_FONT: Font = Font::with_name("Hack");

pub const FONT_FILES: [&[u8]; 4] = [
    include_bytes!("../assets/fonts/Inter-Regular.ttf"),
    include_bytes!("../assets/fonts/Inter-Bold.ttf"),
    include_bytes!("../assets/fonts/Hack-Regular.ttf"),
    iced_fonts::CODICON_FONT_BYTES,
];

/// Type scale: captions and section headers, body text, dialog titles.
pub const SMALL: f32 = 11.0;
pub const BODY: f32 = 13.0;
pub const TITLE: f32 = 16.0;
pub const ICON: f32 = 14.0;

/// Spacing scale, in multiples of 4px.
pub const S1: f32 = 4.0;
pub const S2: f32 = 8.0;
pub const S3: f32 = 12.0;
pub const S4: f32 = 16.0;

// ------------------------------------------------------------------ themes

/// Muted charcoal theme with desaturated accents.
pub static DARK: LazyLock<Theme> = LazyLock::new(|| {
    Theme::custom(
        "litedb Dark",
        Palette {
            background: color!(0x232427),
            text: color!(0xd2cfc8),
            primary: color!(0x6d8aa8),
            success: color!(0x7d9a73),
            warning: color!(0xbfa06a),
            danger: color!(0xb5736c),
        },
    )
});

/// Warm cream theme; easier on the eyes than pure white.
pub static LIGHT: LazyLock<Theme> = LazyLock::new(|| {
    Theme::custom(
        "litedb Light",
        Palette {
            background: color!(0xf3eee2),
            text: color!(0x34302a),
            primary: color!(0x56718d),
            success: color!(0x5b7d55),
            warning: color!(0x9c7b3e),
            danger: color!(0xa65d55),
        },
    )
});

/// Syntax-highlighting theme that suits the app theme.
pub fn highlighter(theme: &Theme) -> iced::highlighter::Theme {
    if theme.extended_palette().is_dark {
        iced::highlighter::Theme::Base16Mocha
    } else {
        iced::highlighter::Theme::InspiredGitHub
    }
}

// ------------------------------------------------------------------ colours

/// Background for window chrome (sidebar, tab strip, status bar): a shade
/// darker than the content area, so content sits "on top".
pub fn chrome(theme: &Theme) -> Color {
    let palette = theme.extended_palette();
    let amount = if palette.is_dark { 0.28 } else { 0.07 };
    mix(palette.background.base.color, Color::BLACK, amount)
}

/// Faint line between regions.
pub fn divider(theme: &Theme) -> Color {
    let palette = theme.extended_palette();
    mix(palette.background.base.color, palette.background.base.text, 0.12)
}

pub fn dim_text(theme: &Theme) -> Color {
    theme.extended_palette().background.base.text.scale_alpha(0.6)
}

fn mix(a: Color, b: Color, t: f32) -> Color {
    Color { r: a.r + (b.r - a.r) * t, g: a.g + (b.g - a.g) * t, b: a.b + (b.b - a.b) * t, a: a.a }
}

fn tint(color: Color, alpha: f32) -> Background {
    Background::Color(Color { a: alpha, ..color })
}

// ------------------------------------------------------------------ icons

/// Codicons, the icon set used by VS Code and Azure Data Studio.
pub mod icon {
    pub use iced_fonts::codicon::*;
}

/// An icon tinted with `color` (or the text colour when `None`).
pub fn icon_colored<'a>(glyph: Text<'a>, color: Option<fn(&Theme) -> Color>) -> Text<'a> {
    let glyph = glyph.size(ICON);
    match color {
        Some(f) => glyph.style(move |t: &Theme| text::Style { color: Some(f(t)) }),
        None => glyph,
    }
}

pub fn success_color(t: &Theme) -> Color {
    t.extended_palette().success.base.color
}

pub fn danger_color(t: &Theme) -> Color {
    t.extended_palette().danger.base.color
}

pub fn accent_color(t: &Theme) -> Color {
    t.extended_palette().primary.base.color
}

// ------------------------------------------------------------------ components

/// Flat toolbar button: icon plus optional label, no border; a faint
/// background appears on hover.
pub fn tool_button<'a, M: Clone + 'a>(glyph: Text<'a>, label: Option<&'a str>, msg: Option<M>) -> Button<'a, M> {
    let content: Element<'a, M> = match label {
        Some(l) => row![glyph, text(l).size(BODY)].spacing(S1 + 2.0).align_y(iced::Alignment::Center).into(),
        None => glyph.into(),
    };
    button(content).padding([S1, S2]).style(flat).on_press_maybe(msg)
}

/// Wraps a widget with a hover tooltip.
pub fn tip<'a, M: 'a>(content: impl Into<Element<'a, M>>, label: &'a str) -> Tooltip<'a, M> {
    tooltip(
        content,
        container(text(label).size(SMALL)).padding([S1, S2]).style(tooltip_style),
        tooltip::Position::Bottom,
    )
    .gap(4)
}

/// Upper-case caption used for panel headers ("OBJECT EXPLORER").
pub fn section_title<'a>(label: &str) -> Text<'a> {
    text(label.to_uppercase()).size(SMALL).font(UI_BOLD).style(|t: &Theme| text::Style { color: Some(dim_text(t)) })
}

pub fn dim<'a>(t: Text<'a>) -> Text<'a> {
    t.style(|t: &Theme| text::Style { color: Some(dim_text(t)) })
}

// ------------------------------------------------------------------ button styles

/// No border or fill; faint background on hover. The default for toolbars,
/// tree rows and other dense UI.
pub fn flat(theme: &Theme, status: button::Status) -> button::Style {
    let palette = theme.extended_palette();
    let text_color = palette.background.base.text;
    let hover = palette.background.base.text;
    let base = button::Style {
        background: None,
        text_color,
        border: Border { radius: 4.0.into(), ..Border::default() },
        ..button::Style::default()
    };
    match status {
        button::Status::Active => base,
        button::Status::Hovered => button::Style { background: Some(tint(hover, 0.07)), ..base },
        button::Status::Pressed => button::Style { background: Some(tint(hover, 0.12)), ..base },
        button::Status::Disabled => button::Style { text_color: text_color.scale_alpha(0.35), ..base },
    }
}

/// The highlighted row in a list or tree: a faint accent fill, no border.
pub fn row_selected(theme: &Theme, status: button::Status) -> button::Style {
    let accent = theme.extended_palette().primary.base.color;
    let base = flat(theme, status);
    let alpha = if status == button::Status::Hovered { 0.24 } else { 0.18 };
    button::Style { background: Some(tint(accent, alpha)), ..base }
}

/// Dialog buttons. One `success` (the main action) per dialog, `neutral`
/// for the rest.
pub fn success(theme: &Theme, status: button::Status) -> button::Style {
    outline(theme, status, theme.extended_palette().success.base.color)
}

pub fn neutral(theme: &Theme, status: button::Status) -> button::Style {
    outline(theme, status, theme.extended_palette().background.strong.color)
}

fn outline(theme: &Theme, status: button::Status, accent: Color) -> button::Style {
    let palette = theme.extended_palette();
    let base = button::Style {
        background: None,
        text_color: palette.background.base.text,
        border: Border { color: accent, width: 1.0, radius: 4.0.into() },
        ..button::Style::default()
    };
    match status {
        button::Status::Active => base,
        button::Status::Hovered => button::Style { background: Some(tint(accent, 0.15)), ..base },
        button::Status::Pressed => button::Style { background: Some(tint(accent, 0.28)), ..base },
        button::Status::Disabled => button::Style {
            text_color: palette.background.base.text.scale_alpha(0.4),
            border: Border { color: accent.scale_alpha(0.4), ..base.border },
            ..base
        },
    }
}

// ------------------------------------------------------------------ container styles

pub fn chrome_panel(theme: &Theme) -> container::Style {
    container::Style { background: Some(chrome(theme).into()), ..container::Style::default() }
}

pub fn status_bar(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(chrome(theme).into()),
        text_color: Some(dim_text(theme)),
        ..container::Style::default()
    }
}

/// The active tab takes the content area's colour and joins it.
pub fn tab(theme: &Theme, active: bool) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: active.then(|| palette.background.base.color.into()),
        border: Border { radius: border::top(6), ..Border::default() },
        ..container::Style::default()
    }
}

/// A 1px divider line (use as the style of a container sized 1px).
pub fn divider_line(theme: &Theme) -> container::Style {
    container::Style { background: Some(divider(theme).into()), ..container::Style::default() }
}

/// Underline under the active results tab.
pub fn underline(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(theme.extended_palette().primary.base.color.into()),
        ..container::Style::default()
    }
}

pub fn tooltip_style(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(palette.background.weak.color.into()),
        text_color: Some(palette.background.base.text),
        border: Border { color: divider(theme), width: 1.0, radius: 4.0.into() },
        ..container::Style::default()
    }
}

pub fn dialog(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(palette.background.base.color.into()),
        border: Border { color: divider(theme), width: 1.0, radius: 8.0.into() },
        shadow: iced::Shadow {
            color: Color::from_rgba(0.0, 0.0, 0.0, 0.45),
            offset: iced::Vector::new(0.0, 8.0),
            blur_radius: 32.0,
        },
        ..container::Style::default()
    }
}

pub fn menu(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(palette.background.base.color.into()),
        border: Border { color: divider(theme), width: 1.0, radius: 6.0.into() },
        shadow: iced::Shadow {
            color: Color::from_rgba(0.0, 0.0, 0.0, 0.35),
            offset: iced::Vector::new(0.0, 4.0),
            blur_radius: 14.0,
        },
        ..container::Style::default()
    }
}

// ------------------------------------------------------------------ field styles

/// Input borders: faint at rest, a small step stronger on hover, and a
/// half-strength accent while focused.
fn field_border(theme: &Theme, hovered: bool, focused: bool) -> Border {
    let palette = theme.extended_palette();
    let color = if focused {
        palette.primary.base.color.scale_alpha(0.5)
    } else if hovered {
        mix(palette.background.weak.color, palette.background.strong.color, 0.5)
    } else {
        palette.background.weak.color
    };
    Border { color, width: 1.0, radius: 4.0.into() }
}

/// The query editor fills its pane, so it has no border at all.
pub fn editor(theme: &Theme, status: iced::widget::text_editor::Status) -> iced::widget::text_editor::Style {
    use iced::widget::text_editor;
    let mut style = text_editor::default(theme, status);
    style.border = Border::default();
    style.selection = theme.extended_palette().primary.base.color.scale_alpha(0.3);
    style
}

pub fn input(theme: &Theme, status: iced::widget::text_input::Status) -> iced::widget::text_input::Style {
    use iced::widget::text_input::{self, Status};
    let border = match status {
        Status::Focused { .. } => field_border(theme, false, true),
        Status::Hovered => field_border(theme, true, false),
        _ => field_border(theme, false, false),
    };
    text_input::Style { border, ..text_input::default(theme, status) }
}
