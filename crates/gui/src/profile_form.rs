//! Adding or editing a connection profile.

use datadrop_core::{Command, Driver, Output, Profile, SslMode};
use egui::RichText;
use egui_phosphor::regular as icon;

use crate::backend::{Backend, Task, poll};

pub struct ProfileForm {
    /// The profile being edited (`None` for a new one).
    pub previous: Option<String>,
    pub name: String,
    pub driver: Driver,
    pub use_url: bool,
    pub url: String,
    pub host: String,
    pub port: String,
    pub database: String,
    pub user: String,
    pub password: String,
    pub ssl_mode: SslMode,
    pub ssl_root_cert: String,
    show_password: bool,
    test: Option<Task>,
    test_result: Option<Result<String, String>>,
    saving: Option<Task>,
    pub error: Option<String>,
    focused: bool,
}

pub enum FormDone {
    Saved(String),
    Cancelled,
}

impl ProfileForm {
    pub fn new() -> Self {
        Self::from_profile(None, &Profile::default())
    }

    pub fn edit(p: &Profile) -> Self {
        Self::from_profile(Some(p.name.clone()), p)
    }

    fn from_profile(previous: Option<String>, p: &Profile) -> Self {
        Self {
            previous,
            name: p.name.clone(),
            driver: p.driver,
            use_url: p.url.is_some(),
            url: p.url.clone().unwrap_or_default(),
            host: p.host.clone(),
            port: p.port.map(|n| n.to_string()).unwrap_or_default(),
            database: p.database.clone(),
            user: p.user.clone(),
            password: p.password.clone().unwrap_or_default(),
            ssl_mode: p.ssl_mode,
            ssl_root_cert: p
                .ssl_root_cert
                .as_ref()
                .map(|c| c.display().to_string())
                .unwrap_or_default(),
            show_password: false,
            test: None,
            test_result: None,
            saving: None,
            error: None,
            focused: false,
        }
    }

    pub fn build(&self) -> Result<Profile, String> {
        let mut p = Profile {
            name: self.name.trim().to_owned(),
            driver: self.driver,
            user: self.user.trim().to_owned(),
            password: (!self.password.is_empty()).then(|| self.password.clone()),
            ssl_mode: self.ssl_mode,
            ssl_root_cert: (!self.ssl_root_cert.trim().is_empty())
                .then(|| self.ssl_root_cert.trim().into()),
            ..Profile::default()
        };
        if self.use_url {
            if self.url.trim().is_empty() {
                return Err("enter the connection URL".into());
            }
            p.url = Some(self.url.trim().to_owned());
        } else {
            p.host = self.host.trim().to_owned();
            p.port = match self.port.trim() {
                "" => None,
                s => Some(
                    s.parse::<u16>()
                        .map_err(|_| format!("'{s}' isn't a port number"))?,
                ),
            };
            p.database = self.database.trim().to_owned();
        }
        p.validate().map_err(|e| e.to_string())?;
        Ok(p)
    }

    /// The form as a modal. Returns once it's saved or cancelled.
    pub fn ui(&mut self, ctx: &egui::Context, backend: &Backend) -> Option<FormDone> {
        if let Some(r) = poll(&mut self.test) {
            self.test_result = Some(match r {
                Ok(Output::Connected { server_version }) => Ok(server_version),
                Ok(_) => Ok("Connected".into()),
                Err(e) => Err(e.to_string()),
            });
        }
        if let Some(r) = poll(&mut self.saving) {
            match r {
                Ok(_) => return Some(FormDone::Saved(self.name.trim().to_owned())),
                Err(e) => self.error = Some(e.to_string()),
            }
        }

        let mut done = None;
        let title = if self.previous.is_some() {
            "Edit connection"
        } else {
            "New connection"
        };
        egui::Modal::new(egui::Id::new("profile-form")).show(ctx, |ui| {
            ui.set_width(480.0);
            ui.heading(title);
            ui.add_space(8.0);
            let busy = self.saving.is_some();
            ui.add_enabled_ui(!busy, |ui| self.fields(ui));

            if let Some(r) = &self.test_result {
                ui.add_space(6.0);
                match r {
                    Ok(v) => ui.label(
                        RichText::new(format!("{} Connected: {}", icon::CHECK_CIRCLE, short(v)))
                            .color(egui::Color32::from_rgb(0x3C, 0xA0, 0x5A)),
                    ),
                    Err(e) => ui.label(
                        RichText::new(format!("{} {e}", icon::WARNING_CIRCLE))
                            .color(ui.visuals().error_fg_color),
                    ),
                };
            }
            if let Some(e) = &self.error {
                ui.add_space(6.0);
                ui.label(RichText::new(e).color(ui.visuals().error_fg_color));
            }
            ui.add_space(6.0);
            ui.weak(
                "Passwords are saved as plain text in your profile files (~/.datadrop/profiles), \
                 readable only by your user account.",
            );
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let testing = self.test.is_some();
                if ui
                    .add_enabled(
                        !testing && !busy,
                        egui::Button::new(format!("{} Test", icon::PLUGS)),
                    )
                    .clicked()
                {
                    self.test_result = None;
                    match self.build() {
                        Ok(profile) => {
                            self.test = Some(backend.run(Command::ConnectionTest { profile }))
                        }
                        Err(e) => self.test_result = Some(Err(e)),
                    }
                }
                if testing {
                    ui.spinner();
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add_enabled(!busy, egui::Button::new("Save")).clicked() {
                        self.error = None;
                        match self.build() {
                            Ok(profile) => {
                                self.saving = Some(backend.run(Command::ProfileSave {
                                    profile,
                                    previous_name: self.previous.clone(),
                                    overwrite: false,
                                }));
                            }
                            Err(e) => self.error = Some(e),
                        }
                    }
                    if ui.button("Cancel").clicked() {
                        if let Some(t) = &self.test {
                            t.cancel();
                        }
                        done = Some(FormDone::Cancelled);
                    }
                });
            });
        });
        done
    }

    fn fields(&mut self, ui: &mut egui::Ui) {
        let wide = |ui: &mut egui::Ui, label: &str, value: &mut String, hint: &str| {
            let l = ui.label(label);
            let r = ui
                .add(
                    egui::TextEdit::singleline(value)
                        .hint_text(hint)
                        .desired_width(f32::INFINITY),
                )
                .labelled_by(l.id);
            ui.end_row();
            r
        };
        egui::Grid::new("profile-fields")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .min_col_width(90.0)
            .show(ui, |ui| {
                let name = wide(ui, "Name", &mut self.name, "e.g. Shop (production)");
                if !self.focused {
                    name.request_focus();
                    self.focused = true;
                }
                ui.label("Database");
                ui.horizontal(|ui| {
                    for d in Driver::ALL {
                        ui.radio_value(&mut self.driver, d, d.label());
                    }
                });
                ui.end_row();
                ui.label("Connect with");
                ui.horizontal(|ui| {
                    ui.radio_value(&mut self.use_url, false, "Host and port");
                    ui.radio_value(&mut self.use_url, true, "URL");
                });
                ui.end_row();
                if self.use_url {
                    let hint = match self.driver {
                        Driver::Postgres => "postgres://user@host:5432/database",
                        Driver::Mysql => "mysql://user@host:3306/database",
                    };
                    wide(ui, "URL", &mut self.url, hint);
                    // Pick the driver from the URL as it's typed.
                    if let Some(d) = Driver::from_url(&self.url) {
                        self.driver = d;
                    }
                } else {
                    wide(ui, "Host", &mut self.host, "localhost");
                    let port_hint = self.driver.default_port().to_string();
                    wide(ui, "Port", &mut self.port, &port_hint);
                    wide(ui, "Database", &mut self.database, "");
                }
                let user_hint = if self.use_url {
                    "if not in the URL"
                } else {
                    ""
                };
                wide(ui, "User", &mut self.user, user_hint);
                let l = ui.label("Password");
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.password)
                            .password(!self.show_password)
                            .desired_width(ui.available_width() - 34.0),
                    )
                    .labelled_by(l.id);
                    let eye = if self.show_password {
                        icon::EYE_SLASH
                    } else {
                        icon::EYE
                    };
                    if ui
                        .small_button(eye)
                        .on_hover_text("Show or hide the password")
                        .clicked()
                    {
                        self.show_password = !self.show_password;
                    }
                });
                ui.end_row();
                ui.label("SSL");
                egui::ComboBox::from_id_salt("ssl-mode")
                    .selected_text(self.ssl_mode.as_str())
                    .show_ui(ui, |ui| {
                        for m in SslMode::ALL {
                            ui.selectable_value(&mut self.ssl_mode, m, m.as_str());
                        }
                    })
                    .response
                    .on_hover_text(
                        "prefer: TLS if the server offers it. require: always TLS. \
                         verify-ca / verify-full: also check the certificate (and host name).",
                    );
                ui.end_row();
                let l = ui.label("CA certificate");
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.ssl_root_cert)
                            .hint_text("optional, a PEM file")
                            .desired_width(ui.available_width() - 70.0),
                    )
                    .labelled_by(l.id);
                    if ui.button("Browse…").clicked()
                        && let Some(f) = rfd::FileDialog::new()
                            .add_filter("Certificates", &["pem", "crt", "cer"])
                            .add_filter("All files", &["*"])
                            .pick_file()
                    {
                        self.ssl_root_cert = f.display().to_string();
                    }
                });
                ui.end_row();
            });
    }
}

fn short(s: &str) -> String {
    let first: String = s.chars().take(70).collect();
    if first.len() < s.len() {
        format!("{first}…")
    } else {
        first
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_profiles() {
        let mut f = ProfileForm::new();
        f.name = "shop".into();
        f.host = "db".into();
        f.port = "6543".into();
        f.user = "me".into();
        f.password = "pw".into();
        let p = f.build().unwrap();
        assert_eq!(p.port, Some(6543));
        assert_eq!(p.password.as_deref(), Some("pw"));
        assert!(p.url.is_none());

        f.port = "huge".into();
        assert!(f.build().is_err());

        f.use_url = true;
        f.url = "mysql://h/db".into();
        f.driver = Driver::Mysql;
        let p = f.build().unwrap();
        assert_eq!(p.url.as_deref(), Some("mysql://h/db"));
        assert!(p.host.is_empty(), "host fields aren't saved with a URL");

        // Editing round-trips.
        let again = ProfileForm::edit(&p).build().unwrap();
        assert_eq!(again, p);
    }
}
