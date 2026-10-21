//! The "Connect to Server" dialog: saved profiles plus an editable form.

use iced::widget::{button, checkbox, column, container, pick_list, row, rule, scrollable, space, text, text_input};
use iced::{Alignment, Element, Length, Theme};

use crate::app::Message;
use crate::db::{Driver, Profile};
use crate::profiles;
use crate::style;

#[derive(Debug, Clone)]
pub enum FormMsg {
    SelectProfile(usize),
    NewProfile,
    Name(String),
    Driver(Driver),
    Host(String),
    Port(String),
    User(String),
    Password(String),
    Database(String),
    TrustCert(bool),
    SavePassword(bool),
    Save,
    Delete,
}

pub struct ConnectForm {
    pub profiles: Vec<Profile>,
    /// Index into `profiles` of the profile shown in the form, if it is saved.
    pub selected: Option<usize>,
    pub form: Profile,
    port_text: String,
    pub password: String,
    pub save_password: bool,
    pub connecting: bool,
    pub notice: Option<Result<String, String>>,
}

impl ConnectForm {
    pub fn new() -> Self {
        let profiles = profiles::load();
        let mut f = Self {
            profiles,
            selected: None,
            form: Profile::new(Driver::SqlServer),
            port_text: String::new(),
            password: String::new(),
            save_password: true,
            connecting: false,
            notice: None,
        };
        match f.profiles.first().cloned() {
            Some(p) => f.load(p, Some(0)),
            None => f.load(Profile::new(Driver::SqlServer), None),
        }
        f
    }

    fn load(&mut self, profile: Profile, index: Option<usize>) {
        self.password = if index.is_some() { profiles::load_password(&profile).unwrap_or_default() } else { String::new() };
        self.port_text = profile.port.to_string();
        self.form = profile;
        self.selected = index;
        self.notice = None;
    }

    /// Checks the form before connecting; the error is shown in the dialog.
    pub fn validate(&mut self) -> bool {
        if port_invalid(&self.port_text) {
            self.notice = Some(Err("Port must be a number between 1 and 65535.".into()));
            return false;
        }
        true
    }

    /// Called after a successful connect, to remember the password if asked.
    pub fn remember_password(&mut self) {
        if self.save_password
            && self.selected.is_some()
            && let Err(e) = profiles::store_password(&self.form, &self.password)
        {
            self.notice = Some(Err(e));
        }
    }

    pub fn update(&mut self, msg: FormMsg) {
        match msg {
            FormMsg::SelectProfile(i) => {
                if let Some(p) = self.profiles.get(i).cloned() {
                    self.load(p, Some(i));
                }
            }
            FormMsg::NewProfile => self.load(Profile::new(self.form.driver), None),
            FormMsg::Name(v) => self.form.name = v,
            FormMsg::Driver(d) => {
                // Swap defaults only if the user hasn't changed them.
                let old = self.form.driver;
                if self.form.port == old.default_port() {
                    self.form.port = d.default_port();
                    self.port_text = self.form.port.to_string();
                }
                if self.form.user == old.default_user() {
                    self.form.user = d.default_user().into();
                }
                if self.form.database == old.default_database() {
                    self.form.database = d.default_database().into();
                }
                self.form.driver = d;
            }
            FormMsg::Host(v) => self.form.host = v,
            FormMsg::Port(v) => {
                if v.is_empty() || v.chars().all(|c| c.is_ascii_digit()) {
                    if let Ok(p) = v.parse() {
                        self.form.port = p;
                    }
                    self.port_text = v;
                }
            }
            FormMsg::User(v) => self.form.user = v,
            FormMsg::Password(v) => self.password = v,
            FormMsg::Database(v) => self.form.database = v,
            FormMsg::TrustCert(v) => self.form.trust_cert = v,
            FormMsg::SavePassword(v) => self.save_password = v,
            FormMsg::Save => self.save(),
            FormMsg::Delete => {
                if let Some(i) = self.selected {
                    let removed = self.profiles.remove(i);
                    profiles::forget_password(&removed);
                    let saved = profiles::save(&self.profiles);
                    match self.profiles.first().cloned() {
                        Some(p) => self.load(p, Some(0)),
                        None => self.load(Profile::new(Driver::SqlServer), None),
                    }
                    self.notice = Some(saved.map(|()| format!("Deleted “{}”.", removed.display_name())));
                }
            }
        }
    }

    fn save(&mut self) {
        if !self.validate() {
            return;
        }
        let profile = self.form.clone();
        let index = match self.selected {
            Some(i) => {
                // The keyring is keyed by host/user/port, so drop the old entry if those changed.
                if profiles::load_password(&self.profiles[i]).is_some() && self.profiles[i] != profile {
                    profiles::forget_password(&self.profiles[i]);
                }
                self.profiles[i] = profile.clone();
                i
            }
            None => {
                self.profiles.push(profile.clone());
                self.profiles.len() - 1
            }
        };
        self.selected = Some(index);
        if self.save_password && !self.password.is_empty() {
            if let Err(e) = profiles::store_password(&profile, &self.password) {
                self.notice = Some(Err(e));
                return;
            }
        } else if !self.save_password {
            profiles::forget_password(&profile);
        }
        self.notice = Some(profiles::save(&self.profiles).map(|()| format!("Saved “{}”.", profile.display_name())));
    }

    pub fn view(&self, live: Option<&Profile>) -> Element<'_, Message> {
        let mut saved = column![].spacing(2);
        for (i, p) in self.profiles.iter().enumerate() {
            let is_live = live == Some(p);
            let label = format!("{}{}", if is_live { "● " } else { "" }, p.display_name());
            saved = saved.push(
                button(text(label).size(14))
                    .width(Length::Fill)
                    .style(if self.selected == Some(i) { style::selected } else { button::text })
                    .on_press(Message::Form(FormMsg::SelectProfile(i))),
            );
        }
        if self.profiles.is_empty() {
            saved = saved.push(text("No saved connections yet.").size(13));
        }
        let saved_pane = column![
            row![
                text("Saved").size(14),
                space().width(Length::Fill),
                button(text("+ New").size(13)).style(button::text).on_press(Message::Form(FormMsg::NewProfile)),
            ]
            .align_y(Alignment::Center),
            scrollable(saved).height(Length::Fill),
        ]
        .spacing(6)
        .width(200);

        let f = &self.form;
        let field = |label: &'static str, input: Element<'static, Message>| -> Element<'static, Message> {
            column![text(label).size(12), input].spacing(2).into()
        };
        let form_msg = |m: fn(String) -> FormMsg| move |v: String| Message::Form(m(v));
        let mut form = column![
            field("Name", text_input("My server", &f.name).style(style::input).on_input(form_msg(FormMsg::Name)).into()),
            field(
                "Server type",
                pick_list(&Driver::ALL[..], Some(f.driver), |d| Message::Form(FormMsg::Driver(d)))
                    .width(Length::Fill)
                    .into()
            ),
            row![
                field("Host", text_input("localhost", &f.host).style(style::input).on_input(form_msg(FormMsg::Host)).into()),
                container(field("Port", text_input("", &self.port_text).style(style::input).on_input(form_msg(FormMsg::Port)).into()))
                    .width(90),
            ]
            .spacing(8),
            row![
                field("User", text_input("", &f.user).style(style::input).on_input(form_msg(FormMsg::User)).into()),
                field(
                    "Password",
                    text_input("", &self.password).style(style::input)
                        .secure(true)
                        .on_input(form_msg(FormMsg::Password))
                        .on_submit(Message::Connect)
                        .into()
                ),
            ]
            .spacing(8),
            field("Database", text_input("", &f.database).style(style::input).on_input(form_msg(FormMsg::Database)).into()),
        ]
        .spacing(10);
        let mut options = row![
            checkbox(self.save_password)
                .label("Save password in keyring")
                .on_toggle(|v| Message::Form(FormMsg::SavePassword(v)))
                .size(16)
                .text_size(14)
        ]
        .spacing(16);
        if f.driver == Driver::SqlServer {
            options = options.push(
                checkbox(f.trust_cert)
                    .label("Trust server certificate")
                    .on_toggle(|v| Message::Form(FormMsg::TrustCert(v)))
                    .size(16)
                    .text_size(14),
            );
        }
        form = form.push(options);

        let notice: Element<'_, Message> = match &self.notice {
            Some(Ok(m)) => text(m).size(13).into(),
            Some(Err(m)) => text(m).size(13).style(text::danger).into(),
            None => space().into(),
        };

        let connect_btn = if self.connecting {
            button("Connecting…")
        } else {
            button("Connect").on_press(Message::Connect).style(style::success)
        };
        let actions = row![
            button("Save").on_press(Message::Form(FormMsg::Save)).style(style::neutral),
            button("Delete")
                .on_press_maybe(self.selected.map(|_| Message::Form(FormMsg::Delete)))
                .style(style::danger),
            space().width(Length::Fill),
            button("Close").on_press(Message::CloseDialog).style(button::text),
            connect_btn,
        ]
        .spacing(8)
        .align_y(Alignment::Center);

        let body = column![
            text("Connect to Server").size(20),
            rule::horizontal(1),
            row![saved_pane, rule::vertical(1), form.width(Length::Fill)].spacing(14).height(330),
            notice,
            actions,
        ]
        .spacing(12);

        container(body).width(720).padding(18).style(dialog_style).into()
    }
}

fn dialog_style(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(palette.background.base.color.into()),
        border: iced::Border { color: palette.background.strong.color, width: 1.0, radius: 8.0.into() },
        shadow: iced::Shadow {
            color: iced::Color::from_rgba(0.0, 0.0, 0.0, 0.5),
            offset: iced::Vector::new(0.0, 6.0),
            blur_radius: 24.0,
        },
        ..container::Style::default()
    }
}

fn port_invalid(text: &str) -> bool {
    !matches!(text.parse::<u16>(), Ok(p) if p > 0)
}
