//! Adding or editing a connection profile.

use datadrop_core::{
    Command, CoreError, Driver, Keychain, Output, PasswordStorage, Profile, SshAuth, SshTunnel,
    SslMode,
};
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
    pub create_if_missing: bool,
    pub use_ssh: bool,
    pub ssh_host: String,
    pub ssh_port: String,
    pub ssh_user: String,
    pub ssh_method: SshMethod,
    pub ssh_password: String,
    pub ssh_key: String,
    pub ssh_passphrase: String,
    pub passwords: PasswordStorage,
    /// Whether the keychain can be used (and why not).
    keychain: Result<(), String>,
    show_password: bool,
    test: Option<Task>,
    test_result: Option<Result<String, String>>,
    /// The SSH server's key isn't known: offer to trust it.
    pub untrusted: Option<TrustPrompt>,
    trusting: Option<Task>,
    saving: Option<Task>,
    pub error: Option<String>,
    focused: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SshMethod {
    Agent,
    Password,
    Key,
}

/// An SSH server whose key isn't known yet.
#[derive(Clone, Debug)]
pub struct TrustPrompt {
    pub host: String,
    pub port: u16,
    pub fingerprint: String,
    pub key: String,
}

impl TrustPrompt {
    pub fn from_error(e: &CoreError) -> Option<Self> {
        match e {
            CoreError::UnknownHostKey {
                host,
                port,
                fingerprint,
                key,
            } => Some(Self {
                host: host.clone(),
                port: *port,
                fingerprint: fingerprint.clone(),
                key: key.clone(),
            }),
            _ => None,
        }
    }

    pub fn command(&self) -> Command {
        Command::SshTrustHost {
            host: self.host.clone(),
            port: self.port,
            key: self.key.clone(),
        }
    }
}

pub enum FormDone {
    Saved(String),
    Cancelled,
}

impl ProfileForm {
    /// A new profile: its passwords go in the keychain if there is one.
    pub fn new(keychain: Result<(), String>) -> Self {
        let mut f = Self::from_profile(None, &Profile::default());
        if keychain.is_ok() {
            f.passwords = PasswordStorage::Keychain;
        }
        f.keychain = keychain;
        f
    }

    pub fn edit(p: &Profile, keychain: Result<(), String>) -> Self {
        let mut f = Self::from_profile(Some(p.name.clone()), p);
        f.keychain = keychain;
        f
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
            create_if_missing: p.create_if_missing,
            use_ssh: p.ssh.is_some(),
            ssh_host: p.ssh.as_ref().map(|t| t.host.clone()).unwrap_or_default(),
            ssh_port: p
                .ssh
                .as_ref()
                .and_then(|t| t.port)
                .map(|n| n.to_string())
                .unwrap_or_default(),
            ssh_user: p.ssh.as_ref().map(|t| t.user.clone()).unwrap_or_default(),
            ssh_method: match p.ssh.as_ref().map(|t| &t.auth) {
                Some(SshAuth::Password { .. }) => SshMethod::Password,
                Some(SshAuth::Key { .. }) => SshMethod::Key,
                _ => SshMethod::Agent,
            },
            ssh_password: match p.ssh.as_ref().map(|t| &t.auth) {
                Some(SshAuth::Password { password }) => password.clone(),
                _ => String::new(),
            },
            ssh_key: match p.ssh.as_ref().map(|t| &t.auth) {
                Some(SshAuth::Key { path, .. }) => path.display().to_string(),
                _ => String::new(),
            },
            ssh_passphrase: match p.ssh.as_ref().map(|t| &t.auth) {
                Some(SshAuth::Key {
                    passphrase: Some(pp),
                    ..
                }) => pp.clone(),
                _ => String::new(),
            },
            passwords: p.passwords,
            keychain: Ok(()),
            show_password: false,
            test: None,
            test_result: None,
            untrusted: None,
            trusting: None,
            saving: None,
            error: None,
            focused: false,
        }
    }

    pub fn build(&self) -> Result<Profile, String> {
        if self.driver.is_file() {
            let p = Profile {
                name: self.name.trim().to_owned(),
                driver: self.driver,
                database: self.database.trim().to_owned(),
                create_if_missing: self.create_if_missing,
                ..Profile::default()
            };
            p.validate().map_err(|e| e.to_string())?;
            return Ok(p);
        }
        let mut p = Profile {
            name: self.name.trim().to_owned(),
            driver: self.driver,
            user: self.user.trim().to_owned(),
            password: (!self.password.is_empty()).then(|| self.password.clone()),
            passwords: self.passwords,
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
        if self.use_ssh {
            p.ssh = Some(SshTunnel {
                host: self.ssh_host.trim().to_owned(),
                port: match self.ssh_port.trim() {
                    "" => None,
                    s => Some(
                        s.parse::<u16>()
                            .map_err(|_| format!("'{s}' isn't an SSH port number"))?,
                    ),
                },
                user: self.ssh_user.trim().to_owned(),
                auth: match self.ssh_method {
                    SshMethod::Agent => SshAuth::Agent,
                    SshMethod::Password => SshAuth::Password {
                        password: self.ssh_password.clone(),
                    },
                    SshMethod::Key => SshAuth::Key {
                        path: self.ssh_key.trim().into(),
                        passphrase: (!self.ssh_passphrase.is_empty())
                            .then(|| self.ssh_passphrase.clone()),
                    },
                },
            });
        }
        p.validate().map_err(|e| e.to_string())?;
        Ok(p)
    }

    /// The form as a modal. Returns once it's saved or cancelled.
    pub fn ui(&mut self, ctx: &egui::Context, backend: &Backend) -> Option<FormDone> {
        if let Some(r) = poll(&mut self.test) {
            self.untrusted = r.as_ref().err().and_then(TrustPrompt::from_error);
            self.test_result = Some(match r {
                Ok(Output::Connected { server_version }) => Ok(server_version),
                Ok(_) => Ok("Connected".into()),
                Err(e) if self.untrusted.is_some() => Err(e.to_string()),
                Err(e) => Err(e.to_string()),
            });
        }
        if let Some(r) = poll(&mut self.trusting) {
            match r {
                // Trusted: test again.
                Ok(_) => {
                    self.untrusted = None;
                    if let Ok(profile) = self.build() {
                        self.test_result = None;
                        self.test = Some(backend.run(Command::ConnectionTest { profile }));
                    }
                }
                Err(e) => self.test_result = Some(Err(e.to_string())),
            }
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
            if let Some(t) = self.untrusted.clone() {
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(format!("{} Key fingerprint:", icon::FINGERPRINT));
                    ui.monospace(&t.fingerprint);
                });
                if ui
                    .add_enabled(
                        self.trusting.is_none(),
                        egui::Button::new(format!(
                            "{} Trust this server and connect",
                            icon::SEAL_CHECK
                        )),
                    )
                    .on_hover_text("Only if the fingerprint matches the server's real key")
                    .clicked()
                {
                    self.trusting = Some(backend.run(t.command()));
                }
            }
            if let Some(e) = &self.error {
                ui.add_space(6.0);
                ui.label(RichText::new(e).color(ui.visuals().error_fg_color));
            }
            ui.add_space(6.0);
            if !self.driver.is_file() {
                ui.weak(
                "Passwords are saved as plain text in your profile files (~/.datadrop/profiles), \
                 readable only by your user account.",
                );
            }
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
                if self.driver.is_file() {
                    self.file_fields(ui);
                    return;
                }
                ui.label("Connect with");
                ui.horizontal(|ui| {
                    ui.radio_value(&mut self.use_url, false, "Host and port");
                    ui.radio_value(&mut self.use_url, true, "URL");
                });
                ui.end_row();
                if self.use_url {
                    let hint = match self.driver {
                        Driver::Postgres => "postgres://user@host:5432/database",
                        Driver::Mysql | Driver::Sqlite => "mysql://user@host:3306/database",
                        Driver::SqlServer => "sqlserver://user@host:1433/database",
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
                self.storage_field(ui);
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
                ui.label("SSH tunnel");
                ui.checkbox(&mut self.use_ssh, "Connect through an SSH server");
                ui.end_row();
                if self.use_ssh {
                    self.ssh_fields(ui);
                }
            });
    }

    fn ssh_fields(&mut self, ui: &mut egui::Ui) {
        let field =
            |ui: &mut egui::Ui, label: &str, value: &mut String, hint: &str, secret: bool| {
                let l = ui.label(label);
                ui.add(
                    egui::TextEdit::singleline(value)
                        .hint_text(hint)
                        .password(secret)
                        .desired_width(f32::INFINITY),
                )
                .labelled_by(l.id);
                ui.end_row();
            };
        field(
            ui,
            "SSH host",
            &mut self.ssh_host,
            "bastion.example.com",
            false,
        );
        field(ui, "SSH port", &mut self.ssh_port, "22", false);
        field(ui, "SSH user", &mut self.ssh_user, "", false);
        ui.label("Sign in with");
        ui.horizontal(|ui| {
            ui.radio_value(&mut self.ssh_method, SshMethod::Agent, "SSH agent");
            ui.radio_value(&mut self.ssh_method, SshMethod::Key, "Key file");
            ui.radio_value(&mut self.ssh_method, SshMethod::Password, "Password");
        });
        ui.end_row();
        match self.ssh_method {
            SshMethod::Agent => {}
            SshMethod::Password => {
                field(ui, "SSH password", &mut self.ssh_password, "", true);
            }
            SshMethod::Key => {
                let l = ui.label("Key file");
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.ssh_key)
                            .hint_text("~/.ssh/id_ed25519")
                            .desired_width(ui.available_width() - 70.0),
                    )
                    .labelled_by(l.id);
                    if ui.button("Browse…").clicked() {
                        let mut d = rfd::FileDialog::new();
                        if let Some(home) = std::env::home_dir() {
                            d = d.set_directory(home.join(".ssh"));
                        }
                        if let Some(f) = d.pick_file() {
                            self.ssh_key = f.display().to_string();
                        }
                    }
                });
                ui.end_row();
                field(
                    ui,
                    "Passphrase",
                    &mut self.ssh_passphrase,
                    "if the key has one",
                    true,
                );
            }
        }
    }
}

impl ProfileForm {
    /// Where the passwords are kept.
    fn storage_field(&mut self, ui: &mut egui::Ui) {
        ui.label("Keep passwords");
        let text = |s: PasswordStorage| match s {
            PasswordStorage::Keychain => "In the keychain",
            PasswordStorage::File => "In the profile file",
        };
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("password-storage")
                .selected_text(text(self.passwords))
                .show_ui(ui, |ui| {
                    ui.add_enabled_ui(self.keychain.is_ok(), |ui| {
                        ui.selectable_value(
                            &mut self.passwords,
                            PasswordStorage::Keychain,
                            text(PasswordStorage::Keychain),
                        )
                    });
                    ui.selectable_value(
                        &mut self.passwords,
                        PasswordStorage::File,
                        text(PasswordStorage::File),
                    );
                })
                .response
                .on_hover_text(format!(
                    "The keychain is {}. The profile file is plain text, readable only by \
                     your user account.",
                    Keychain::name()
                ));
            if let Err(why) = &self.keychain {
                ui.label(RichText::new(icon::WARNING).color(ui.visuals().warn_fg_color))
                    .on_hover_text(why);
            }
        });
        ui.end_row();
    }

    fn file_fields(&mut self, ui: &mut egui::Ui) {
        let l = ui.label("File");
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.database)
                    .hint_text("the database file")
                    .desired_width(ui.available_width() - 70.0),
            )
            .labelled_by(l.id);
            if ui.button("Browse…").clicked() {
                let dialog = rfd::FileDialog::new()
                    .add_filter("SQLite databases", &["db", "sqlite", "sqlite3", "db3"])
                    .add_filter("All files", &["*"]);
                let picked = if self.create_if_missing {
                    dialog.set_file_name("database.db").save_file()
                } else {
                    dialog.pick_file()
                };
                if let Some(f) = picked {
                    self.database = f.display().to_string();
                }
            }
        });
        ui.end_row();
        ui.label("");
        ui.checkbox(
            &mut self.create_if_missing,
            "Create the file if it doesn't exist",
        );
        ui.end_row();
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
        let mut f = ProfileForm::new(Ok(()));
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

        // Through SSH, with a key.
        let mut t = ProfileForm::new(Ok(()));
        t.name = "tunnelled".into();
        t.host = "10.0.0.5".into();
        t.use_ssh = true;
        t.ssh_host = "bastion".into();
        t.ssh_user = "ops".into();
        t.ssh_method = SshMethod::Key;
        assert!(t.build().is_err(), "needs the key file");
        t.ssh_key = "/home/me/.ssh/id_ed25519".into();
        t.ssh_passphrase = "pp".into();
        let p = t.build().unwrap();
        let ssh = p.ssh.clone().unwrap();
        assert_eq!(ssh.port(), 22);
        assert_eq!(
            ssh.auth,
            SshAuth::Key {
                path: "/home/me/.ssh/id_ed25519".into(),
                passphrase: Some("pp".into())
            }
        );
        assert_eq!(ProfileForm::edit(&p, Ok(())).build().unwrap(), p);

        // SQLite needs only the file.
        let mut lite = ProfileForm::new(Ok(()));
        lite.name = "local".into();
        lite.driver = Driver::Sqlite;
        assert!(lite.build().is_err());
        lite.database = "/tmp/x.db".into();
        lite.create_if_missing = true;
        lite.password = "ignored".into();
        let l = lite.build().unwrap();
        assert_eq!(l.database, "/tmp/x.db");
        assert!(l.create_if_missing && l.password.is_none());

        // Editing round-trips.
        let again = ProfileForm::edit(&p, Ok(())).build().unwrap();
        assert_eq!(again, p);
    }
}
