mod app;
mod connect;
mod db;
mod explorer;
mod profiles;
mod query_tab;
mod settings;
mod style;

fn main() -> iced::Result {
    iced::application(app::App::new, app::App::update, app::App::view)
        .title(app::App::title)
        .theme(app::App::theme)
        .subscription(app::App::subscription)
        .window_size((1280.0, 800.0))
        .run()
}
