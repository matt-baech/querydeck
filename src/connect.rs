//! The "Connect to Server" dialog: saved profiles plus an editable form.

use iced::widget::{button, checkbox, column, container, pick_list, row, scrollable, space, text, text_input};
use iced::{Alignment, Element, Length, Theme};

use crate::app::Message;
use crate::db::{Driver, Profile};
use crate::profiles;
use crate::style::{self, icon};

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
        self.password =
            if index.is_some() { profiles::load_password(&profile).unwrap_or_default() } else { String::new() };
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
        // Left: saved connections.
        let mut saved = column![].spacing(2);
        for (i, p) in self.profiles.iter().enumerate() {
            let is_live = live == Some(p);
            let dot: Element<'_, Message> = if is_live {
                style::icon_colored(icon::circle_filled(), Some(style::success_color)).size(8).into()
            } else {
                space().width(8).into()
            };
            let content = row![
                style::icon_colored(icon::server(), None),
                column![
                    text(p.display_name()).size(style::BODY),
                    style::dim(text(format!("{} · {}", p.driver, p.host)).size(style::SMALL)),
                ]
                .spacing(1)
                .width(Length::Fill),
                dot,
            ]
            .spacing(style::S2)
            .align_y(Alignment::Center);
            saved = saved.push(
                button(content)
                    .width(Length::Fill)
                    .padding([style::S1 + 2.0, style::S2])
                    .style(if self.selected == Some(i) { style::row_selected } else { style::flat })
                    .on_press(Message::Form(FormMsg::SelectProfile(i))),
            );
        }
        if self.profiles.is_empty() {
            saved =
                saved.push(container(style::dim(text("No saved connections").size(style::SMALL))).padding(style::S2));
        }
        let saved_pane = container(
            column![
                row![
                    style::section_title("Saved"),
                    space().width(Length::Fill),
                    style::tip(
                        style::tool_button(
                            icon::add().size(style::ICON),
                            None,
                            Some(Message::Form(FormMsg::NewProfile))
                        ),
                        "New connection",
                    ),
                ]
                .align_y(Alignment::Center),
                scrollable(saved).height(Length::Fill),
            ]
            .spacing(style::S2),
        )
        .padding(style::S3)
        .width(230)
        .height(Length::Fill)
        .style(style::chrome_panel);

        // Right: the form.
        let f = &self.form;
        let field = |label: &'static str, input: Element<'static, Message>| -> Element<'static, Message> {
            column![style::dim(text(label).size(style::SMALL)), input].spacing(style::S1).into()
        };
        let form_msg = |m: fn(String) -> FormMsg| move |v: String| Message::Form(m(v));
        let input = |placeholder: &'static str, value: &str| {
            text_input(placeholder, value).style(style::input).size(style::BODY).padding([style::S1 + 2.0, style::S2])
        };
        let mut form = column![
            field("Name", input("My server", &f.name).on_input(form_msg(FormMsg::Name)).into()),
            field(
                "Server type",
                pick_list(&Driver::ALL[..], Some(f.driver), |d| Message::Form(FormMsg::Driver(d)))
                    .text_size(style::BODY)
                    .padding([style::S1 + 2.0, style::S2])
                    .width(Length::Fill)
                    .into()
            ),
            row![
                field("Host", input("localhost", &f.host).on_input(form_msg(FormMsg::Host)).into()),
                container(field("Port", input("", &self.port_text).on_input(form_msg(FormMsg::Port)).into())).width(90),
            ]
            .spacing(style::S2),
            row![
                field("User", input("", &f.user).on_input(form_msg(FormMsg::User)).into()),
                field(
                    "Password",
                    input("", &self.password)
                        .secure(true)
                        .on_input(form_msg(FormMsg::Password))
                        .on_submit(Message::Connect)
                        .into()
                ),
            ]
            .spacing(style::S2),
            field("Database", input("", &f.database).on_input(form_msg(FormMsg::Database)).into()),
        ]
        .spacing(style::S3);
        let mut options = column![
            checkbox(self.save_password)
                .label("Save password in keyring")
                .on_toggle(|v| Message::Form(FormMsg::SavePassword(v)))
                .size(14)
                .text_size(style::BODY)
        ]
        .spacing(style::S2);
        if f.driver == Driver::SqlServer {
            options = options.push(
                checkbox(f.trust_cert)
                    .label("Trust server certificate")
                    .on_toggle(|v| Message::Form(FormMsg::TrustCert(v)))
                    .size(14)
                    .text_size(style::BODY),
            );
        }
        form = form.push(container(options).padding([style::S1, 0.0]));

        let notice: Element<'_, Message> = match &self.notice {
            Some(Ok(m)) => style::dim(text(m).size(style::SMALL)).into(),
            Some(Err(m)) => {
                text(m).size(style::SMALL).style(|t: &Theme| text::Style { color: Some(style::danger_color(t)) }).into()
            }
            None => space().into(),
        };

        let dialog_button =
            |label: &'static str| button(text(label).size(style::BODY)).padding([style::S1 + 2.0, style::S3]);
        let connect_btn = if self.connecting {
            dialog_button("Connecting…").style(style::success)
        } else {
            dialog_button("Connect").style(style::success).on_press(Message::Connect)
        };
        let delete_btn = style::tip(
            style::tool_button(
                style::icon_colored(icon::trash(), Some(style::danger_color)),
                None,
                self.selected.map(|_| Message::Form(FormMsg::Delete)),
            ),
            "Delete saved connection",
        );
        let actions = row![
            delete_btn,
            notice,
            space().width(Length::Fill),
            dialog_button("Save").style(style::neutral).on_press(Message::Form(FormMsg::Save)),
            connect_btn,
        ]
        .spacing(style::S2)
        .align_y(Alignment::Center);

        let title = row![
            text("Connect to Server").size(style::TITLE).font(style::UI_BOLD),
            space().width(Length::Fill),
            style::tip(
                style::tool_button(icon::close().size(style::ICON), None, Some(Message::CloseDialog)),
                "Close (Esc)"
            ),
        ]
        .align_y(Alignment::Center);

        let right =
            column![title, form, space().height(Length::Fill), actions].spacing(style::S4).padding(style::S4 + 4.0);

        container(row![saved_pane, right.width(Length::Fill)])
            .width(760)
            .height(530)
            .clip(true)
            .style(style::dialog)
            .into()
    }
}

fn port_invalid(text: &str) -> bool {
    !matches!(text.parse::<u16>(), Ok(p) if p > 0)
}
