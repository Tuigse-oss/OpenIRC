//! The server list (saved servers by network plus the network directory),
//! the prompt for an untrusted server certificate, and the certificates
//! window (client certificate and trusted server certificates).

use crate::i18n::{tr, trf};
use eframe::egui;

use super::{
    OpenIrcApp, color_ink, color_paper, field, muted_color, muted_on_face, paper, section,
};
use crate::servers::{ENCODINGS, Login, ServerEntry};

#[derive(Default)]
pub(super) struct ServerList {
    pub open: bool,
    /// Index in `settings.servers` being edited, or `None` for a new one.
    selected: Option<usize>,
    draft: Option<ServerEntry>,
    filter: String,
    pub note: String,
}

impl OpenIrcApp {
    pub(super) fn open_server_list(&mut self) {
        self.server_list.open = true;
        if self.server_list.draft.is_none() {
            let mut draft = ServerEntry::new(
                "",
                &self.form.server,
                self.form.port.parse().unwrap_or(6697),
                self.form.tls,
            );
            draft.channels = self.form.channel.clone();
            if let Some(index) = self.settings.servers.iter().position(|entry| {
                super::same_name(&entry.host, &draft.host) && entry.port == draft.port
            }) {
                self.server_list.selected = Some(index);
                draft = self.settings.servers[index].clone();
            }
            self.server_list.draft = Some(draft);
        }
    }

    pub(super) fn server_list_window(&mut self, ctx: &egui::Context) {
        if !self.server_list.open {
            return;
        }
        let mut open = true;
        let mut pick_saved = None;
        let mut pick_directory = None;
        let mut save = false;
        let mut delete = false;
        let mut connect = false;
        let mut close = false;
        let mut refresh = false;
        egui::Window::new(tr("Servers"))
            .collapsible(false)
            .resizable(true)
            .default_size([760.0, 500.0])
            .default_pos(super::dialog_pos(ctx, 0))
            .open(&mut open)
            .show(ctx, |ui| {
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(240.0);
                        section(ui, tr("My servers"));
                        paper().show(ui, |ui| {
                            ui.set_min_width(230.0);
                            egui::ScrollArea::vertical()
                                .id_salt("saved-servers")
                                .max_height(170.0)
                                .auto_shrink([false, true])
                                .show(ui, |ui| {
                                    if self.settings.servers.is_empty() {
                                        ui.label(
                                            egui::RichText::new(tr("No saved servers yet. Pick a network below or fill in the form, then Save."))
                                                .color(muted_color()),
                                        );
                                    }
                                    let mut networks: Vec<String> = self
                                        .settings
                                        .servers
                                        .iter()
                                        .map(network_name)
                                        .collect();
                                    networks.sort_by_key(|name| name.to_lowercase());
                                    networks.dedup();
                                    for network in networks {
                                        egui::CollapsingHeader::new(egui::RichText::new(&network).color(color_ink()))
                                            .id_salt(("network", &network))
                                            .default_open(true)
                                            .show(ui, |ui| {
                                                for (index, entry) in self.settings.servers.iter().enumerate() {
                                                    if network_name(entry) != network {
                                                        continue;
                                                    }
                                                    let selected = self.server_list.selected == Some(index);
                                                    if ui
                                                        .selectable_label(
                                                            selected,
                                                            egui::RichText::new(entry.label()).color(if selected {
                                                                super::text_on(super::color_hilite(), egui::Color32::WHITE)
                                                            } else {
                                                                color_ink()
                                                            }),
                                                        )
                                                        .clicked()
                                                    {
                                                        pick_saved = Some(index);
                                                    }
                                                }
                                            });
                                    }
                                });
                        });
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            section(ui, tr("Network directory"));
                            let busy = self.network_rx.is_some();
                            if ui
                                .add_enabled(!busy, egui::Button::new(if busy { tr("Updating…") } else { tr("Update") }).small())
                                .on_hover_text(trf(
                                    "Download the public network list from {0}",
                                    &[&crate::networks::DIRECTORY_HOST],
                                ))
                                .clicked()
                            {
                                refresh = true;
                            }
                        });
                        ui.add(field(&mut self.server_list.filter).hint_text(tr("Filter networks")).desired_width(230.0));
                        paper().show(ui, |ui| {
                            ui.set_min_width(230.0);
                            let filter = self.server_list.filter.to_lowercase();
                            let names: Vec<(usize, String)> = self
                                .networks
                                .iter()
                                .enumerate()
                                .filter(|(_, net)| filter.is_empty() || net.name.to_lowercase().contains(&filter))
                                .map(|(index, net)| (index, net.name.clone()))
                                .collect();
                            let row = ui.text_style_height(&egui::TextStyle::Body) + 2.0;
                            egui::ScrollArea::vertical()
                                .id_salt("directory")
                                .max_height(170.0)
                                .auto_shrink([false, true])
                                .show_rows(ui, row, names.len(), |ui, range| {
                                    for (index, name) in &names[range] {
                                        if ui
                                            .selectable_label(false, egui::RichText::new(name).color(color_ink()))
                                            .clicked()
                                        {
                                            pick_directory = Some(*index);
                                        }
                                    }
                                });
                        });
                        let note = if self.network_note.is_empty() {
                            trf("{0} networks", &[&self.networks.len()])
                        } else {
                            self.network_note.clone()
                        };
                        ui.label(egui::RichText::new(note).small().color(muted_on_face()));
                    });
                    ui.separator();
                    ui.vertical(|ui| {
                        let global_nick = self.settings.nick.clone();
                        let Some(draft) = self.server_list.draft.as_mut() else {
                            return;
                        };
                        section(
                            ui,
                            if self.server_list.selected.is_some() {
                                tr("Edit server")
                            } else {
                                tr("New server")
                            },
                        );
                        super::form_grid("server-form")
                            .num_columns(2)
                            .spacing([8.0, 5.0])
                            .show(ui, |ui| {
                                ui.label(tr("Network"));
                                ui.add(field(&mut draft.network).hint_text(tr("group, such as Libera.Chat")).desired_width(260.0));
                                ui.end_row();
                                ui.label(tr("Description"));
                                ui.add(field(&mut draft.description).hint_text(tr("optional")).desired_width(260.0));
                                ui.end_row();
                                ui.label(tr("Address"));
                                ui.add(field(&mut draft.host).desired_width(260.0));
                                ui.end_row();
                                ui.label(tr("Port"));
                                ui.horizontal(|ui| {
                                    ui.add(egui::DragValue::new(&mut draft.port).range(1..=65535));
                                    ui.checkbox(&mut draft.tls, tr("TLS"));
                                });
                                ui.end_row();
                                ui.label(tr("Server password"));
                                ui.add(field(&mut draft.password).password(true).hint_text(tr("usually empty")).desired_width(260.0));
                                ui.end_row();
                                ui.label(tr("Log in with"));
                                egui::ComboBox::from_id_salt("login-method")
                                    .selected_text(draft.login.label())
                                    .width(260.0)
                                    .show_ui(ui, |ui| {
                                        for login in Login::ALL {
                                            ui.selectable_value(&mut draft.login, login, login.label());
                                        }
                                    });
                                ui.end_row();
                                let needs_password = matches!(draft.login, Login::SaslPlain | Login::NickServ);
                                ui.label(tr("Account"));
                                ui.add_enabled(
                                    draft.login != Login::None && draft.login != Login::SaslExternal,
                                    field(&mut draft.login_user).hint_text(tr("blank uses the nick")).desired_width(260.0),
                                );
                                ui.end_row();
                                ui.label(tr("Password"));
                                ui.add_enabled(
                                    needs_password,
                                    field(&mut draft.login_pass).password(true).desired_width(260.0),
                                );
                                ui.end_row();
                                ui.label(tr("Nick"));
                                ui.add(field(&mut draft.nick).hint_text(trf("blank uses {0}", &[&global_nick])).desired_width(260.0));
                                ui.end_row();
                                ui.label(tr("Alternative"));
                                ui.add(field(&mut draft.alt_nick).hint_text(tr("used when the nick is taken")).desired_width(260.0));
                                ui.end_row();
                                ui.label(tr("Encoding"));
                                egui::ComboBox::from_id_salt("encoding")
                                    .selected_text(draft.encoding.clone())
                                    .width(260.0)
                                    .show_ui(ui, |ui| {
                                        for encoding in ENCODINGS {
                                            ui.selectable_value(&mut draft.encoding, encoding.to_string(), encoding);
                                        }
                                    });
                                ui.end_row();
                                ui.label(tr("Join channels"));
                                ui.add(field(&mut draft.channels).hint_text("#one, #two").desired_width(260.0));
                                ui.end_row();
                            });
                        if draft.login == Login::SaslExternal && self.settings.client_cert.trim().is_empty() {
                            ui.colored_label(
                                super::tone_color(super::Tone::Attention),
                                tr("SASL with a certificate needs a client certificate (File, Certificates)."),
                            );
                        }
                        if draft.encoding == "auto" {
                            ui.label(
                                egui::RichText::new(tr("auto: UTF-8, reading other text as Windows-1252."))
                                    .small()
                                    .color(muted_on_face()),
                            );
                        }
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            let valid = !draft.host.trim().is_empty();
                            if ui.add_enabled(valid, egui::Button::new(tr("Connect"))).clicked() {
                                connect = true;
                            }
                            if ui.add_enabled(valid, egui::Button::new(tr("Save"))).clicked() {
                                save = true;
                            }
                            if ui
                                .add_enabled(self.server_list.selected.is_some(), egui::Button::new(tr("Delete")))
                                .clicked()
                            {
                                delete = true;
                            }
                            if ui.button(tr("New")).clicked() {
                                *draft = ServerEntry::new("", "", 6697, true);
                                self.server_list.selected = None;
                            }
                            if ui.button(tr("Close")).clicked() {
                                close = true;
                            }
                        });
                        if !self.server_list.note.is_empty() {
                            ui.label(egui::RichText::new(&self.server_list.note).color(muted_on_face()));
                        }
                    });
                });
            });
        if refresh {
            self.refresh_networks();
        }
        if let Some(index) = pick_saved {
            self.server_list.selected = Some(index);
            self.server_list.draft = self.settings.servers.get(index).cloned();
            self.server_list.note.clear();
        }
        if let Some(index) = pick_directory
            && let Some(net) = self.networks.get(index)
        {
            let mut draft = ServerEntry::new(&net.name, &net.host, net.port, net.tls);
            if let Some(old) = &self.server_list.draft {
                draft.channels = old.channels.clone();
            }
            self.server_list.draft = Some(draft);
            self.server_list.selected = None;
            self.server_list.note = format!(
                "{} from the directory. Save keeps it in My servers.",
                net.name
            );
        }
        if save && let Some(draft) = self.server_list.draft.clone() {
            match self.server_list.selected {
                Some(index) if index < self.settings.servers.len() => {
                    self.settings.servers[index] = draft
                }
                _ => {
                    self.settings.servers.push(draft);
                    self.server_list.selected = Some(self.settings.servers.len() - 1);
                }
            }
            self.persist();
            self.server_list.note = tr("Saved.").into();
        }
        if delete && let Some(index) = self.server_list.selected.take() {
            if index < self.settings.servers.len() {
                self.settings.servers.remove(index);
                self.persist();
            }
            self.server_list.note = tr("Deleted.").into();
        }
        if connect && let Some(draft) = self.server_list.draft.clone() {
            self.settings.server = draft.host.clone();
            self.settings.port = draft.port;
            self.settings.tls = draft.tls;
            self.form.server = draft.host.clone();
            self.form.port = draft.port.to_string();
            self.form.tls = draft.tls;
            self.form.channel = draft.channels.clone();
            self.persist();
            self.open_server(draft);
            close = true;
        }
        if close || !open {
            self.server_list.open = false;
        }
    }

    /// Ask whether to trust a server certificate that did not verify.
    pub(super) fn trust_dialog(&mut self, ctx: &egui::Context) {
        let Some((id, cert)) = self.trust_prompt.clone() else {
            return;
        };
        let mut choice = None;
        egui::Modal::new(egui::Id::new("trust-cert")).show(ctx, |ui| {
            ui.set_max_width(460.0);
            ui.heading(tr("Untrusted certificate"));
            ui.label(trf(
                "The certificate of {0} could not be verified: it is {1}.",
                &[&cert.host, &cert.reason],
            ));
            ui.label(
                tr("Many IRC servers use their own certificates. Trust it only if you know this is the \
                 server you meant to reach. OpenIRC will then accept exactly this certificate from \
                 this server, and ask again if it ever changes."),
            );
            ui.add_space(4.0);
            ui.label(tr("SHA-256 fingerprint"));
            egui::Frame::new()
                .fill(color_paper())
                .inner_margin(egui::Margin::same(6))
                .show(ui, |ui| {
                    ui.label(egui::RichText::new(group_hex(&cert.fingerprint)).monospace().color(color_ink()));
                });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button(tr("Trust and connect")).clicked() {
                    choice = Some(true);
                }
                if ui.button(tr("Cancel")).clicked() {
                    choice = Some(false);
                }
            });
        });
        match choice {
            Some(true) => {
                self.trust_prompt = None;
                self.settings
                    .trusted_certs
                    .push((cert.host.clone(), cert.fingerprint.clone()));
                self.persist();
                if let Some(session) = self.sessions.iter_mut().find(|session| session.id == id) {
                    session.hangup = false;
                    session.attempts = 0;
                }
                self.reconnect(id);
            }
            Some(false) => {
                self.trust_prompt = None;
                if let Some(session) = self.sessions.iter_mut().find(|session| session.id == id) {
                    session.hangup = true;
                }
            }
            None => {}
        }
    }

    /// The server's certificate, the client certificate, and trusted
    /// server certificates.
    pub(super) fn certificates_window(&mut self, ctx: &egui::Context) {
        if !self.show_cert {
            return;
        }
        let server = self
            .active_server()
            .and_then(|id| self.sessions.iter().find(|session| session.id == id))
            .map(|session| (session.host.clone(), session.fingerprint.clone()));
        let mut open = true;
        let mut create = false;
        let mut remove = None;
        egui::Window::new(tr("Certificates"))
            .collapsible(false)
            .default_width(520.0)
            .default_pos(super::dialog_pos(ctx, 9))
            .open(&mut open)
            .show(ctx, |ui| {
                section(ui, tr("This server"));
                match &server {
                    Some((host, Some(fingerprint))) => {
                        ui.label(format!("{host} — SHA-256"));
                        ui.label(egui::RichText::new(group_hex(fingerprint)).monospace().small());
                    }
                    Some((host, None)) => {
                        ui.label(trf("{0} is not using TLS.", &[&host]));
                    }
                    None => {
                        ui.label(tr("Not connected."));
                    }
                }
                ui.separator();
                section(ui, tr("My client certificate"));
                ui.label(
                    tr("Used to log in with CertFP or SASL EXTERNAL. Register its fingerprint once with \
                     your network's services, for example /msg NickServ CERT ADD."),
                );
                ui.horizontal(|ui| {
                    ui.label(tr("PEM file"));
                    ui.add(
                        field(&mut self.settings.client_cert)
                            .hint_text(tr("certificate and private key"))
                            .desired_width(300.0),
                    );
                });
                ui.horizontal(|ui| {
                    if ui.button(tr("Create a certificate")).clicked() {
                        create = true;
                    }
                    if ui.button(tr("Save")).clicked() {
                        self.cert_note = match crate::certs::fingerprint(self.settings.client_cert.trim()) {
                            Ok(fingerprint) => format!("SHA-256 {}", group_hex(&fingerprint)),
                            Err(err) => err.to_string(),
                        };
                        self.persist();
                    }
                });
                if !self.cert_note.is_empty() {
                    ui.label(egui::RichText::new(&self.cert_note).monospace().small());
                }
                ui.separator();
                section(ui, tr("Trusted server certificates"));
                if self.settings.trusted_certs.is_empty() {
                    ui.label(egui::RichText::new(tr("None.")).color(muted_on_face()));
                }
                egui::Grid::new("trusted-certs")
                    .num_columns(3)
                    .spacing([10.0, 4.0])
                    .show(ui, |ui| {
                        for (index, (host, fingerprint)) in
                            self.settings.trusted_certs.iter().enumerate()
                        {
                            ui.label(host);
                            ui.label(
                                egui::RichText::new(fingerprint.get(..23).unwrap_or(fingerprint))
                                    .monospace()
                                    .small(),
                            )
                            .on_hover_text(group_hex(fingerprint));
                            if ui.small_button(tr("Remove")).clicked() {
                                remove = Some(index);
                            }
                            ui.end_row();
                        }
                    });
            });
        if create {
            let path = crate::settings::config_dir().join("client-cert.pem");
            let name = self.settings.nick.clone();
            self.cert_note = match crate::certs::create(&name, &path) {
                Ok(fingerprint) => {
                    self.settings.client_cert = path.display().to_string();
                    self.persist();
                    format!(
                        "Created {}.\nSHA-256 {}",
                        path.display(),
                        group_hex(&fingerprint)
                    )
                }
                Err(err) => trf("Could not create the certificate: {0}", &[&err]),
            };
        }
        if let Some(index) = remove {
            self.settings.trusted_certs.remove(index);
            self.persist();
        }
        if !open {
            self.show_cert = false;
        }
    }
}

fn network_name(entry: &ServerEntry) -> String {
    if entry.network.trim().is_empty() {
        "Other".into()
    } else {
        entry.network.trim().to_string()
    }
}

/// `ab12cd34…` as `AB:12:CD:34…`, the usual way fingerprints are shown.
pub(super) fn group_hex(hex: &str) -> String {
    hex.as_bytes()
        .chunks(2)
        .map(|pair| String::from_utf8_lossy(pair).to_uppercase())
        .collect::<Vec<_>>()
        .join(":")
}

#[cfg(test)]
mod tests {
    #[test]
    fn fingerprints_are_grouped() {
        assert_eq!(super::group_hex("ab12cd"), "AB:12:CD");
    }
}
