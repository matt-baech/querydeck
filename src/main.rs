mod app;
mod complete;
mod connect;
mod db;
mod explorer;
mod files;
mod history;
mod preferences;
mod profiles;
mod query_tab;
mod settings;
mod style;
mod tray;

fn main() -> iced::Result {
    settings::migrate_from_litedb();
    // A daemon keeps running with no windows open, which is what lets the
    // app live in the system tray: on Wayland a window can't be hidden, only
    // closed and opened again.
    let mut app = iced::daemon(app::App::new, app::App::update, app::App::view)
        .title(app::App::title)
        .theme(app::App::theme)
        .subscription(app::App::subscription)
        .default_font(style::UI_FONT);
    for font in style::FONT_FILES {
        app = app.font(font);
    }
    app.run()
}
