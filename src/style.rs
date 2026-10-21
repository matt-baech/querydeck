//! Shared widget styles. Buttons are outlined: the accent colour goes on the
//! border, with only a faint tint on hover, instead of solid colour fills.

use std::sync::LazyLock;

use iced::theme::Palette;
use iced::widget::button::{Status, Style};
use iced::widget::container;
use iced::{Background, Border, Color, Theme, border, color};

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

/// Background for window chrome such as the tab strip: a shade darker than
/// the content area, so the active tab reads as sitting on top.
pub fn chrome(theme: &Theme) -> Color {
    let palette = theme.extended_palette();
    let amount = if palette.is_dark { 0.28 } else { 0.07 };
    mix(palette.background.base.color, Color::BLACK, amount)
}

pub fn tab_strip(theme: &Theme) -> container::Style {
    container::Style { background: Some(chrome(theme).into()), ..container::Style::default() }
}

/// The active tab takes the content area's colour and joins it; inactive tabs
/// blend into the strip.
pub fn tab(theme: &Theme, active: bool) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: active.then(|| palette.background.base.color.into()),
        border: Border { radius: border::top(6), ..Border::default() },
        ..container::Style::default()
    }
}

fn mix(a: Color, b: Color, t: f32) -> Color {
    Color {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: a.a,
    }
}

pub fn success(theme: &Theme, status: Status) -> Style {
    outline(theme, status, theme.extended_palette().success.base.color)
}

pub fn danger(theme: &Theme, status: Status) -> Style {
    outline(theme, status, theme.extended_palette().danger.base.color)
}

pub fn neutral(theme: &Theme, status: Status) -> Style {
    outline(theme, status, theme.extended_palette().background.strong.color)
}

/// The chosen item in a list or set of toggles (tabs, tree nodes, profiles).
pub fn selected(theme: &Theme, status: Status) -> Style {
    let accent = theme.extended_palette().primary.base.color;
    let mut style = outline(theme, status, accent);
    if status == Status::Active {
        style.background = Some(tint(accent, 0.12));
    }
    style
}

fn outline(theme: &Theme, status: Status, accent: Color) -> Style {
    let palette = theme.extended_palette();
    let base = Style {
        background: None,
        text_color: palette.background.base.text,
        border: Border { color: accent, width: 1.0, radius: 4.0.into() },
        ..Style::default()
    };
    match status {
        Status::Active => base,
        Status::Hovered => Style { background: Some(tint(accent, 0.15)), ..base },
        Status::Pressed => Style { background: Some(tint(accent, 0.28)), ..base },
        Status::Disabled => Style {
            text_color: palette.background.base.text.scale_alpha(0.4),
            border: Border { color: accent.scale_alpha(0.4), ..base.border },
            ..base
        },
    }
}

fn tint(color: Color, alpha: f32) -> Background {
    Background::Color(Color { a: alpha, ..color })
}

/// Input borders: faint at rest, a small step brighter on hover, and a
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
    Border { color, width: 1.0, radius: 3.0.into() }
}

pub fn editor(theme: &Theme, status: iced::widget::text_editor::Status) -> iced::widget::text_editor::Style {
    use iced::widget::text_editor::{self, Status};
    let border = match status {
        Status::Focused { .. } => field_border(theme, false, true),
        Status::Hovered => field_border(theme, true, false),
        _ => field_border(theme, false, false),
    };
    text_editor::Style { border, ..text_editor::default(theme, status) }
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

/// Syntax-highlighting theme that suits the app theme.
pub fn highlighter(theme: &Theme) -> iced::highlighter::Theme {
    if theme.extended_palette().is_dark {
        iced::highlighter::Theme::Base16Mocha
    } else {
        iced::highlighter::Theme::InspiredGitHub
    }
}
