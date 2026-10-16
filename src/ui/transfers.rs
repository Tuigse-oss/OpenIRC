//! DCC file transfers: the prompt for an incoming file (save, resume,
//! overwrite, or a new name), automatic accepts and refused file types,
//! folders by file type, and the Transfers window with progress, speed,
//! and cancel.

use crate::i18n::{fill, tr, trf};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Instant;

use eframe::egui;

use super::{OpenIrcApp, dialog_pos, field, muted_on_face};
use crate::dcc;
use crate::dcc_send::{self, DccSendOffer, Meter};

/// One file being sent or received.
pub(super) struct Transfer {
    nick: String,
    name: String,
    path: PathBuf,
    sending: bool,
    size: Option<u64>,
    meter: Arc<Meter>,
    started: Instant,
    /// Bytes and time of the last speed sample, and the speed then.
    sample: (u64, Instant, f64),
    done: Receiver<Result<String, String>>,
    /// The outcome once the thread ends: whether it worked, and a note.
    result: Option<(bool, String)>,
    /// The server the transfer was arranged on.
    server: Option<u64>,
    /// For a file we offer by listening: the port, so a resume request
    /// from the receiver can be matched to it.
    port: Option<u16>,
}

impl Transfer {
    pub(super) fn running(&self) -> bool {
        self.result.is_none()
    }
}

/// A file offer waiting in the DCC window, with the server it came from.
pub(super) struct QueuedOffer {
    pub(super) from: String,
    pub(super) offer: DccSendOffer,
    pub(super) server: Option<u64>,
}

/// An incoming file waiting for an answer.
pub(super) struct GetPrompt {
    from: String,
    offer: DccSendOffer,
    /// The server the offer came on: the answer goes there, and so does
    /// everything the transfer arranges later.
    server: Option<u64>,
    /// Where to save it; editable.
    dest: String,
    /// Accept files from this nick without asking from now on.
    always: bool,
}

/// What to do with a file that already exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Existing {
    /// Save under the chosen name (which does not exist).
    New,
    Overwrite,
    Resume,
}

/// A resume asked for and waiting for the sender's ACCEPT.
pub(super) struct ResumeWait {
    from: String,
    filename: String,
    host: String,
    port: u16,
    dest: PathBuf,
    size: Option<u64>,
    /// The position asked for; the sender must accept exactly this one.
    offset: u64,
    server: Option<u64>,
}

impl OpenIrcApp {
    /// The folder for a received file: the first folder rule that lists
    /// its extension, else the download folder.
    fn download_dir(&self, name: &str) -> PathBuf {
        let base = PathBuf::from(self.settings.dcc_folder.trim());
        let ext = extension(name);
        for (types, folder) in &self.settings.dcc_folder_rules {
            if !ext.is_empty()
                && types
                    .split([' ', ',', ';'])
                    .map(|item| item.trim().trim_start_matches('.').to_ascii_lowercase())
                    .any(|item| item == ext)
            {
                let folder = PathBuf::from(folder.trim());
                return if folder.is_absolute() {
                    folder
                } else {
                    base.join(folder)
                };
            }
        }
        base
    }

    fn refused_type(&self, name: &str) -> bool {
        let ext = extension(name);
        !ext.is_empty()
            && self
                .settings
                .dcc_ignore_types
                .split([' ', ',', ';'])
                .any(|item| {
                    item.trim()
                        .trim_start_matches('.')
                        .eq_ignore_ascii_case(&ext)
                })
    }

    fn auto_accepts(&self, nick: &str) -> bool {
        let address = self
            .addresses
            .get(&(self.scope(), super::nick_key(nick)))
            .cloned()
            .unwrap_or_else(|| "*@*".into());
        let full = format!("{nick}!{address}");
        self.settings.dcc_auto_accept.iter().any(|mask| {
            let mask = mask.trim();
            if mask.contains(['!', '@']) {
                crate::script::mask_match(&crate::rules::full_mask(mask), &full)
            } else {
                crate::script::mask_match(mask, nick)
            }
        })
    }

    /// Someone offered a file: refuse it by type, accept it for a trusted
    /// sender, or ask.
    pub(super) fn incoming_file(&mut self, from: String, offer: DccSendOffer) {
        let safe = dcc_send::safe_filename(&offer.filename);
        if self.refused_type(&safe) {
            self.note_status(trf(
                "Refused {0} from {1}: that file type is not accepted (Options, DCC).",
                &[&safe, &from],
            ));
            return;
        }
        let size = offer
            .size
            .map(|size| format!(" ({})", human_bytes(size)))
            .unwrap_or_default();
        self.note_status(trf("{0} offers {1}{2}", &[&from, &safe, &size]));
        let dest = self.download_dir(&safe).join(&safe);
        if self.auto_accepts(&from) {
            let existing = std::fs::metadata(&dest).ok().map(|meta| meta.len());
            let choice = match (existing, self.settings.dcc_exists.as_str()) {
                (None, _) => Some((dest.clone(), Existing::New)),
                (Some(_), "overwrite") => Some((dest.clone(), Existing::Overwrite)),
                (Some(have), "resume")
                    if !dcc_send::is_passive(&offer)
                        && offer.size.is_none_or(|size| have < size) =>
                {
                    Some((dest.clone(), Existing::Resume))
                }
                (Some(_), "resume" | "rename") => Some((
                    dcc_send::unique_path(dest.parent().unwrap_or(Path::new(".")), &safe),
                    Existing::New,
                )),
                _ => None,
            };
            if let Some((dest, how)) = choice {
                self.accept_file(from, offer, dest, how);
                return;
            }
        }
        let server = self.scope();
        if self.get_prompt.is_none() {
            self.get_prompt = Some(GetPrompt {
                from,
                offer,
                server,
                dest: dest.display().to_string(),
                always: false,
            });
        } else {
            self.send_offers.push(QueuedOffer {
                from,
                offer,
                server,
            });
            self.show_dcc = true;
        }
        self.cue("dcc");
    }

    /// Open the prompt for an offer listed in the DCC window.
    pub(super) fn review_offer(&mut self, index: usize) {
        if index >= self.send_offers.len() {
            return;
        }
        let QueuedOffer {
            from,
            offer,
            server,
        } = self.send_offers.remove(index);
        if let Some(previous) = self.get_prompt.take() {
            self.send_offers.push(QueuedOffer {
                from: previous.from,
                offer: previous.offer,
                server: previous.server,
            });
        }
        let safe = dcc_send::safe_filename(&offer.filename);
        let dest = self.download_dir(&safe).join(&safe);
        self.get_prompt = Some(GetPrompt {
            from,
            offer,
            server,
            dest: dest.display().to_string(),
            always: false,
        });
    }

    fn accept_file(&mut self, from: String, offer: DccSendOffer, dest: PathBuf, how: Existing) {
        let name = dcc_send::safe_filename(&offer.filename);
        let passive = dcc_send::is_passive(&offer);
        if how == Existing::Resume {
            if passive {
                self.note_status(
                    tr("A file offered without listening cannot be resumed; save it under a new name or overwrite it.").into(),
                );
                return;
            }
            let offset = std::fs::metadata(&dest).map(|meta| meta.len()).unwrap_or(0);
            let body = dcc_send::encode_resume_request(&offer.filename, offer.port, offset);
            self.send_user(format!("/raw PRIVMSG {from} :\u{1}DCC {body}\u{1}"));
            self.note_status(trf(
                "Asked {0} to resume {1} at byte {2}.",
                &[&from, &name, &offset],
            ));
            self.resume_wait = Some(ResumeWait {
                from,
                filename: offer.filename,
                host: offer.host,
                port: offer.port,
                dest,
                size: offer.size,
                offset,
                server: self.scope(),
            });
            return;
        }
        // Open the file now: a new name is taken at once, so another
        // download cannot end up writing to it too.
        let fresh = how == Existing::New;
        let opened = if fresh {
            dcc_send::create_new_file(&dest)
        } else {
            dcc_send::create_overwrite(&dest).map(|file| (file, dest.clone()))
        };
        let (file, dest) = match opened {
            Ok(opened) => opened,
            Err(err) => {
                self.note_status(trf("Cannot save {0}: {1}", &[&name, &err]));
                return;
            }
        };
        if passive {
            let Some(token) = offer.token.clone() else {
                self.note_status(tr("The passive offer is missing its token.").into());
                discard(fresh, &dest);
                return;
            };
            let listener = match crate::net::listen_dcc() {
                Ok(listener) => listener,
                Err(err) => {
                    self.note_status(trf("DCC listen failed: {0}", &[&err]));
                    discard(fresh, &dest);
                    return;
                }
            };
            let port = match listener.local_addr() {
                Ok(addr) => addr.port(),
                Err(err) => {
                    self.note_status(trf("DCC listen failed: {0}", &[&err]));
                    discard(fresh, &dest);
                    return;
                }
            };
            let ip = dcc::encode_dcc_ip(crate::net::dcc_ip());
            let line = dcc_send::passive_accept_message(
                &from,
                &offer.filename,
                ip,
                port,
                offer.size.unwrap_or(0),
                &token,
            );
            self.send_user(format!("/raw {line}"));
            let size = offer.size;
            let target = dest.clone();
            self.spawn_transfer(&from, &name, dest, false, size, move |meter| {
                let result = dcc_send::receive_listener(listener, file, size, meter);
                if result.is_err() && meter.bytes() == 0 {
                    discard(fresh, &target);
                }
                result
            });
            return;
        }
        self.start_receive(
            &from,
            &name,
            &offer.host,
            offer.port,
            (file, dest, fresh),
            offer.size,
            0,
        );
    }

    /// The sender agreed to resume: receive the rest, if the answer is from
    /// the same person on the same server, for the same file, at exactly
    /// the position asked for.
    pub(super) fn resume_accepted(&mut self, from: &str, filename: &str, port: u16, position: u64) {
        let server = self.scope();
        let matches = self.resume_wait.as_ref().is_some_and(|wait| {
            super::same_nick(&wait.from, from)
                && wait.server == server
                && wait.filename == filename
                && wait.port == port
        });
        if !matches {
            return;
        }
        let Some(wait) = self.resume_wait.take() else {
            return;
        };
        let name = dcc_send::safe_filename(&wait.filename);
        if position != wait.offset {
            self.note_status(trf(
                "{0} offered to resume {1} at byte {2} instead of {3}; the file was not changed.",
                &[&from, &name, &position, &wait.offset],
            ));
            return;
        }
        let file = match dcc_send::open_resume(&wait.dest, wait.offset) {
            Ok(file) => file,
            Err(err) => {
                self.note_status(trf("Cannot resume {0}: {1}", &[&name, &err]));
                return;
            }
        };
        self.start_receive(
            &wait.from,
            &name,
            &wait.host,
            wait.port,
            (file, wait.dest, false),
            wait.size,
            position,
        );
    }

    /// The receiver of a file we offer asks to resume it. Agree when the
    /// request matches a transfer still waiting for its connection.
    pub(super) fn resume_requested(
        &mut self,
        from: &str,
        filename: &str,
        port: u16,
        position: u64,
    ) {
        let server = self.scope();
        if port == 0 {
            self.note_status(trf(
                "{0} asked to resume {1}, but a file offered without listening cannot be resumed.",
                &[&from, &filename],
            ));
            return;
        }
        // The request names the file we offered on that port. Some clients
        // write the stand-in name `file.ext` instead of the real one; the
        // port and person still have to match.
        let Some(transfer) = self.transfers.iter().find(|transfer| {
            transfer.sending
                && transfer.running()
                && transfer.port == Some(port)
                && transfer.server == server
                && super::same_nick(&transfer.nick, from)
                && (transfer.name == filename || filename == "file.ext")
        }) else {
            return;
        };
        if transfer.size.is_some_and(|size| position >= size) {
            return;
        }
        // Only while the receiver has not connected yet: after that the
        // send has begun, and an ACCEPT would promise a start it never uses.
        if !transfer.meter.start_at(position) {
            return;
        }
        let name = transfer.name.clone();
        let body = dcc_send::encode_accept(filename, port, position);
        self.send_user(format!("/raw PRIVMSG {from} :\u{1}DCC {body}\u{1}"));
        self.note_status(trf(
            "{0} resumes {1} at byte {2}.",
            &[&from, &name, &position],
        ));
    }

    /// Receive into `file`, which is open at `offset`. `fresh` means the
    /// file was created for this download, so it goes if nothing arrives.
    #[allow(clippy::too_many_arguments)]
    fn start_receive(
        &mut self,
        from: &str,
        name: &str,
        host: &str,
        port: u16,
        (file, dest, fresh): (std::fs::File, PathBuf, bool),
        size: Option<u64>,
        offset: u64,
    ) {
        let host = host.to_string();
        let target = dest.clone();
        self.note_status(trf("Receiving {0} from {1}.", &[&name, &from]));
        self.spawn_transfer(from, name, dest, false, size, move |meter| {
            let result = dcc_send::receive_from(&host, port, file, offset, size, meter);
            if result.is_err() && meter.bytes() == 0 {
                discard(fresh, &target);
            }
            result.map(|total| total.saturating_sub(offset))
        });
    }

    /// Connect to a receiver that answered a passive offer and send.
    pub(super) fn start_upload(
        &mut self,
        nick: &str,
        path: &Path,
        host: &str,
        port: u16,
        name: &str,
    ) {
        let source = path.to_path_buf();
        let host = host.to_string();
        let size = std::fs::metadata(path).ok().map(|meta| meta.len());
        self.note_status(trf("Sending {0} to {1}.", &[&name, &nick]));
        self.spawn_transfer(nick, name, path.to_path_buf(), true, size, move |meter| {
            dcc_send::upload_to(&host, port, &source, meter)
        });
    }

    /// Offer the file in the DCC window to its nick.
    pub(super) fn offer_send(&mut self) {
        let nick = self.dcc_nick.trim().to_string();
        let path = PathBuf::from(self.send_path.trim());
        if nick.is_empty() || !path.is_file() {
            self.note_status(tr("Choose a nick and an existing file.").into());
            return;
        }
        let size = match std::fs::metadata(&path).map(|meta| meta.len()) {
            Ok(size) => size,
            Err(err) => {
                self.note_status(trf("Cannot read the file: {0}", &[&err]));
                return;
            }
        };
        let filename = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("file")
            .to_string();
        if self.dcc_passive {
            let token = dcc_send::fresh_token();
            let line = dcc_send::passive_offer_message(&nick, &filename, size, &token);
            self.send_user(format!("/raw {line}"));
            self.passive_out.push(super::PassiveOut {
                token,
                path,
                filename: filename.clone(),
                nick: nick.clone(),
                server: self.scope(),
                made: Instant::now(),
            });
            self.note_status(trf(
                "Offered {0} to {1} without listening.",
                &[&filename, &nick],
            ));
            return;
        }
        let listener = match crate::net::listen_dcc() {
            Ok(listener) => listener,
            Err(err) => {
                self.note_status(trf("DCC listen failed: {0}", &[&err]));
                return;
            }
        };
        let port = match listener.local_addr() {
            Ok(addr) => addr.port(),
            Err(err) => {
                self.note_status(trf("DCC listen failed: {0}", &[&err]));
                return;
            }
        };
        let ip = dcc::encode_dcc_ip(crate::net::dcc_ip());
        let line = dcc_send::offer_message(&nick, &filename, ip, port, size);
        self.send_user(format!("/raw {line}"));
        self.note_status(trf("Offered {0} to {1}.", &[&filename, &nick]));
        let source = path.clone();
        self.spawn_transfer(&nick, &filename, path, true, Some(size), move |meter| {
            dcc_send::serve_file(listener, &source, meter)
        });
        if let Some(transfer) = self.transfers.last_mut() {
            transfer.port = Some(port);
        }
    }

    fn spawn_transfer(
        &mut self,
        nick: &str,
        name: &str,
        path: PathBuf,
        sending: bool,
        size: Option<u64>,
        job: impl FnOnce(&Meter) -> std::io::Result<u64> + Send + 'static,
    ) {
        let meter = Arc::new(Meter::default());
        let (tx, rx) = mpsc::channel();
        let shared = meter.clone();
        let label = name.to_string();
        let shown = path.display().to_string();
        // The thread has no language of its own: translate first.
        let sent = tr("Sent {0} ({1}).");
        let saved = tr("Saved {0} ({1}) to {2}.");
        let cancelled = tr("{0}: cancelled.");
        thread::spawn(move || {
            let result = match job(&shared) {
                Ok(bytes) if sending => Ok(fill(sent, &[&label, &human_bytes(bytes)])),
                Ok(bytes) => Ok(fill(saved, &[&label, &human_bytes(bytes), &shown])),
                Err(err) if err.kind() == std::io::ErrorKind::Interrupted => {
                    Err(fill(cancelled, &[&label]))
                }
                Err(err) => Err(format!("{label}: {err}")),
            };
            let _ = tx.send(result);
        });
        let now = Instant::now();
        self.transfers.push(Transfer {
            nick: nick.to_string(),
            name: name.to_string(),
            path,
            sending,
            size,
            meter,
            started: now,
            sample: (0, now, 0.0),
            done: rx,
            result: None,
            server: self.scope(),
            port: None,
        });
        self.show_transfers = true;
    }

    /// Collect finished transfers and note them in the status window.
    pub(super) fn poll_transfers(&mut self) {
        let mut notes = Vec::new();
        for transfer in &mut self.transfers {
            if transfer.result.is_some() {
                continue;
            }
            match transfer.done.try_recv() {
                Ok(Ok(text)) => {
                    notes.push(text.clone());
                    transfer.result = Some((true, text));
                }
                Ok(Err(text)) => {
                    notes.push(text.clone());
                    transfer.result = Some((false, text));
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    transfer.result = Some((false, tr("The transfer ended.").into()));
                }
            }
        }
        for text in notes {
            self.note_status(text);
        }
    }

    pub(super) fn get_prompt_dialog(&mut self, ctx: &egui::Context) {
        let Some(prompt) = self.get_prompt.as_mut() else {
            return;
        };
        let mut choice = None;
        let mut decline = false;
        let mut later = false;
        let name = dcc_send::safe_filename(&prompt.offer.filename);
        egui::Window::new(tr("Incoming file"))
            .default_pos(dialog_pos(ctx, 6))
            .collapsible(false)
            .resizable([false, false])
            .show(ctx, |ui| {
                let size = prompt
                    .offer
                    .size
                    .map(human_bytes)
                    .unwrap_or_else(|| "size not given".into());
                ui.label(
                    egui::RichText::new(trf(
                        "{0} offers {1} ({2}).",
                        &[&prompt.from, &name, &size],
                    ))
                    .strong(),
                );
                ui.add_space(4.0);
                ui.label(tr("Save to"));
                ui.add(field(&mut prompt.dest).desired_width(380.0));
                let dest = PathBuf::from(prompt.dest.trim());
                let existing = std::fs::metadata(&dest).ok().map(|meta| meta.len());
                ui.checkbox(
                    &mut prompt.always,
                    trf("Accept files from {0} without asking", &[&prompt.from]),
                );
                ui.add_space(4.0);
                match existing {
                    Some(have) => {
                        ui.label(
                            egui::RichText::new(trf(
                                "That file exists ({0}).",
                                &[&human_bytes(have)],
                            ))
                            .color(muted_on_face()),
                        );
                        ui.horizontal(|ui| {
                            let resumable = !dcc_send::is_passive(&prompt.offer)
                                && prompt.offer.size.is_none_or(|size| have < size);
                            if ui
                                .add_enabled(resumable, egui::Button::new(tr("Resume")))
                                .on_hover_text(tr(
                                    "Ask the sender to continue from the end of the file",
                                ))
                                .clicked()
                            {
                                choice = Some((dest.clone(), Existing::Resume));
                            }
                            if ui.button(tr("Overwrite")).clicked() {
                                choice = Some((dest.clone(), Existing::Overwrite));
                            }
                            let folder = dest.parent().unwrap_or(Path::new(".")).to_path_buf();
                            let file = dest
                                .file_name()
                                .map(|file| file.to_string_lossy().into_owned())
                                .unwrap_or_else(|| name.clone());
                            let renamed = dcc_send::unique_path(&folder, &file);
                            let label = renamed
                                .file_name()
                                .map(|file| file.to_string_lossy().into_owned())
                                .unwrap_or_default();
                            if ui.button(trf("Save as {0}", &[&label])).clicked() {
                                choice = Some((renamed, Existing::New));
                            }
                        });
                    }
                    None => {
                        if ui.button(tr("Save")).clicked() {
                            choice = Some((dest.clone(), Existing::New));
                        }
                    }
                }
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button(tr("Decline")).clicked() {
                        decline = true;
                    }
                    if ui
                        .button(tr("Later"))
                        .on_hover_text(tr("Keep it in Tools, Send file / chat"))
                        .clicked()
                    {
                        later = true;
                    }
                });
            });
        if let Some((dest, how)) = choice
            && let Some(prompt) = self.get_prompt.take()
        {
            if prompt.always {
                let nick = prompt.from.clone();
                if !self
                    .settings
                    .dcc_auto_accept
                    .iter()
                    .any(|mask| mask.eq_ignore_ascii_case(&nick))
                {
                    self.settings.dcc_auto_accept.push(nick);
                    self.persist();
                }
            }
            self.answer_offer(prompt, dest, how);
        } else if decline && let Some(prompt) = self.get_prompt.take() {
            self.note_status(trf("Declined {0} from {1}.", &[&name, &prompt.from]));
        } else if later && let Some(prompt) = self.get_prompt.take() {
            self.send_offers.push(QueuedOffer {
                from: prompt.from,
                offer: prompt.offer,
                server: prompt.server,
            });
        }
    }

    /// Accept an offer from the prompt on the server it came from, whatever
    /// window is in front now.
    pub(super) fn answer_offer(&mut self, prompt: GetPrompt, dest: PathBuf, how: Existing) {
        self.on_server(prompt.server, |app| {
            app.accept_file(prompt.from, prompt.offer, dest, how);
        });
    }

    pub(super) fn transfers_window(&mut self, ctx: &egui::Context) {
        if !self.show_transfers {
            return;
        }
        let mut open = true;
        let mut clear = false;
        let mut cancel = None;
        let mut reveal = None;
        egui::Window::new(tr("Transfers"))
            .default_pos(dialog_pos(ctx, 7))
            .default_size([620.0, 220.0])
            .collapsible(false)
            .open(&mut open)
            .show(ctx, |ui| {
                if self.transfers.is_empty() {
                    ui.label(tr(
                        "No transfers yet. Tools, Send file / chat offers a file.",
                    ));
                    return;
                }
                egui::ScrollArea::vertical()
                    .max_height(260.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        egui::Grid::new("transfers")
                            .num_columns(5)
                            .spacing([10.0, 6.0])
                            .striped(true)
                            .show(ui, |ui| {
                                for (index, transfer) in self.transfers.iter_mut().enumerate() {
                                    let bytes = transfer.meter.bytes();
                                    ui.label(if transfer.sending { "↑" } else { "↓" })
                                        .on_hover_text(if transfer.sending {
                                            "Sending"
                                        } else {
                                            "Receiving"
                                        });
                                    ui.vertical(|ui| {
                                        ui.set_min_width(160.0);
                                        ui.add(
                                            egui::Label::new(
                                                egui::RichText::new(&transfer.name).strong(),
                                            )
                                            .truncate(),
                                        )
                                        .on_hover_text(transfer.path.display().to_string());
                                        ui.label(
                                            egui::RichText::new(format!(
                                                "{} {}",
                                                if transfer.sending { "to" } else { "from" },
                                                transfer.nick
                                            ))
                                            .small()
                                            .color(muted_on_face()),
                                        );
                                    });
                                    let fraction = transfer
                                        .size
                                        .filter(|size| *size > 0)
                                        .map(|size| (bytes as f32 / size as f32).clamp(0.0, 1.0));
                                    let amount = match transfer.size {
                                        Some(size) => {
                                            format!(
                                                "{} of {}",
                                                human_bytes(bytes),
                                                human_bytes(size)
                                            )
                                        }
                                        None => human_bytes(bytes),
                                    };
                                    // The amount sits under the bar: text drawn on the
                                    // bar itself is lost against the unfilled part.
                                    ui.vertical(|ui| {
                                        ui.add(
                                            egui::ProgressBar::new(fraction.unwrap_or(0.0))
                                                .desired_width(180.0)
                                                .desired_height(10.0),
                                        );
                                        ui.label(
                                            egui::RichText::new(amount)
                                                .small()
                                                .color(muted_on_face()),
                                        );
                                    });
                                    let status = match &transfer.result {
                                        Some((true, _)) => tr("Done").to_string(),
                                        Some((false, text)) => text.clone(),
                                        None => {
                                            let now = Instant::now();
                                            let (then, at, rate) = transfer.sample;
                                            let elapsed = now.duration_since(at).as_secs_f64();
                                            let rate = if elapsed >= 1.0 {
                                                let fresh =
                                                    (bytes.saturating_sub(then)) as f64 / elapsed;
                                                transfer.sample = (bytes, now, fresh);
                                                fresh
                                            } else if rate == 0.0 && bytes > 0 {
                                                bytes as f64
                                                    / transfer
                                                        .started
                                                        .elapsed()
                                                        .as_secs_f64()
                                                        .max(0.1)
                                            } else {
                                                rate
                                            };
                                            format!("{}/s", human_bytes(rate as u64))
                                        }
                                    };
                                    ui.label(status);
                                    ui.horizontal(|ui| {
                                        if transfer.running() {
                                            if ui.button(tr("Cancel")).clicked() {
                                                cancel = Some(index);
                                            }
                                        } else if !transfer.sending
                                            && ui.button(tr("Show folder")).clicked()
                                        {
                                            reveal = Some(index);
                                        }
                                    });
                                    ui.end_row();
                                }
                            });
                    });
                ui.separator();
                if ui.button(tr("Clear finished")).clicked() {
                    clear = true;
                }
            });
        if let Some(index) = cancel
            && let Some(transfer) = self.transfers.get(index)
        {
            transfer.meter.cancel();
        }
        if let Some(index) = reveal
            && let Some(folder) = self
                .transfers
                .get(index)
                .and_then(|transfer| transfer.path.parent())
        {
            super::open_url(&folder.display().to_string());
        }
        if clear {
            self.transfers.retain(Transfer::running);
        }
        if !open {
            self.show_transfers = false;
        }
        if self.transfers.iter().any(Transfer::running) {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
    }

    /// The DCC part of Options: folders, file types, and trusted senders.
    pub(super) fn dcc_options(ui: &mut egui::Ui, settings: &mut crate::settings::Settings) {
        super::section(ui, tr("Receiving files"));
        super::form_grid("dcc-receive")
            .num_columns(2)
            .spacing([8.0, 4.0])
            .show(ui, |ui| {
                ui.label(tr("Download folder"));
                ui.add(field(&mut settings.dcc_folder).desired_width(320.0));
                ui.end_row();
                ui.label(tr("Never accept file types"));
                ui.add(field(&mut settings.dcc_ignore_types).desired_width(320.0));
                ui.end_row();
                ui.label(tr("If the file already exists"));
                egui::ComboBox::from_id_salt("dcc-exists")
                    .selected_text(match settings.dcc_exists.as_str() {
                        "resume" => tr("resume it"),
                        "overwrite" => tr("overwrite it"),
                        "rename" => tr("save under a new name"),
                        _ => tr("ask"),
                    })
                    .show_ui(ui, |ui| {
                        for (key, label) in [
                            ("ask", tr("ask")),
                            ("resume", tr("resume it")),
                            ("overwrite", tr("overwrite it")),
                            ("rename", tr("save under a new name")),
                        ] {
                            ui.selectable_value(&mut settings.dcc_exists, key.into(), label);
                        }
                    });
                ui.end_row();
            });
        ui.add_space(4.0);
        ui.label(tr("Folders by file type (extensions separated by spaces; a relative folder is inside the download folder)"));
        let mut remove = None;
        egui::Grid::new("dcc-folders")
            .num_columns(3)
            .spacing([6.0, 4.0])
            .show(ui, |ui| {
                for (index, (types, folder)) in settings.dcc_folder_rules.iter_mut().enumerate() {
                    ui.add(
                        field(types)
                            .desired_width(140.0)
                            .hint_text(tr("jpg png gif")),
                    );
                    ui.add(field(folder).desired_width(180.0).hint_text(tr("Pictures")));
                    if ui.small_button(tr("Remove")).clicked() {
                        remove = Some(index);
                    }
                    ui.end_row();
                }
            });
        if let Some(index) = remove {
            settings.dcc_folder_rules.remove(index);
        }
        if ui.button(tr("Add folder rule")).clicked() {
            settings
                .dcc_folder_rules
                .push((String::new(), String::new()));
        }
        ui.add_space(6.0);
        ui.checkbox(
            &mut settings.dcc_keep_awake,
            tr("Keep the computer awake while a transfer runs"),
        );
        ui.add_space(6.0);
        super::section(ui, tr("Trusted senders"));
        ui.label(tr(
            "Accept files from these nicks or masks without asking, one per line",
        ));
        let mut text = settings.dcc_auto_accept.join("\n");
        if ui
            .add(
                super::field_multi(&mut text)
                    .desired_rows(3)
                    .desired_width(360.0)
                    .hint_text(tr("friend\nbot!*@trusted.example")),
            )
            .changed()
        {
            settings.dcc_auto_accept = text
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(str::to_string)
                .collect();
        }
    }
}

/// The lowercase extension of a file name, without the dot.
fn extension(name: &str) -> String {
    Path::new(name)
        .extension()
        .map(|ext| ext.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

/// A byte count for people: `512 B`, `1.5 KiB`, `12.3 MiB`.
pub(super) fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KiB", "MiB", "GiB", "TiB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if value >= 100.0 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Remove a file made for a download that never received anything.
fn discard(fresh: bool, path: &Path) {
    if fresh && std::fs::metadata(path).is_ok_and(|meta| meta.len() == 0) {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_read_well() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(1536), "1.5 KiB");
        assert_eq!(human_bytes(150 * 1024 * 1024), "150 MiB");
        assert_eq!(extension("Photo.JPG"), "jpg");
        assert_eq!(extension("README"), "");
    }

    use crate::pump::Event;
    use crate::ui::tests::{sent, texts, two_servers};

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("openirc-ui-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_passive_reply_counts_only_from_the_person_and_server_it_went_to() {
        let (mut app, _rx1, _rx2, _, _) = two_servers();
        let dir = scratch("passive");
        let path = dir.join("file.txt");
        std::fs::write(&path, b"x").unwrap();
        app.passive_out.push(super::super::PassiveOut {
            token: "123456789".into(),
            path,
            filename: "file.txt".into(),
            nick: "bob".into(),
            server: Some(1),
            made: Instant::now(),
        });
        let reply = |from: &str, port: u16| Event::DccSend {
            from: from.into(),
            filename: "file.txt".into(),
            host: "127.0.0.1".into(),
            port,
            size: Some(1),
            token: Some("123456789".into()),
        };
        for (server, from, port) in [(2, "bob", 9), (1, "eve", 9), (1, "bob", 0)] {
            app.routing = Some(server);
            app.apply_event(reply(from, port));
            app.routing = None;
            assert_eq!(app.passive_out.len(), 1, "{server} {from} {port}");
        }
        assert!(app.transfers.iter().all(|transfer| !transfer.sending));
        app.routing = Some(1);
        app.apply_event(reply("Bob", 9));
        app.routing = None;
        assert!(app.passive_out.is_empty());
        assert!(app.transfers.iter().any(|transfer| transfer.sending));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_offer_is_answered_on_the_server_it_came_from() {
        let (mut app, rx1, rx2, _, ops2) = two_servers();
        let dir = scratch("origin");
        app.settings.dcc_folder = dir.display().to_string();
        std::fs::write(dir.join("report.bin"), vec![7u8; 100]).unwrap();
        let offer = |filename: &str, token: Option<&str>| Event::DccSend {
            from: "alice".into(),
            filename: filename.into(),
            host: "127.0.0.1".into(),
            port: if token.is_some() { 0 } else { 9 },
            size: Some(200),
            token: token.map(str::to_string),
        };
        // Two offers from alice on server 1: one asks, one waits in the list.
        app.routing = Some(1);
        app.apply_event(offer("report.bin", None));
        app.apply_event(offer("passive.bin", Some("77")));
        app.routing = None;
        assert_eq!(app.send_offers.len(), 1);

        // The user looks at server 2, then resumes the first file.
        app.active = ops2;
        let prompt = app.get_prompt.take().unwrap();
        app.answer_offer(prompt, dir.join("report.bin"), Existing::Resume);
        let to_one = sent(&rx1);
        assert!(
            to_one
                .iter()
                .any(|line| line.contains("DCC RESUME report.bin 9 100")),
            "{to_one:?}"
        );
        assert!(sent(&rx2).is_empty(), "nothing goes to server 2");
        // Alice's answer on server 1 is the one that counts.
        app.routing = Some(2);
        app.apply_event(Event::DccAccept {
            from: "alice".into(),
            filename: "report.bin".into(),
            port: 9,
            position: 100,
        });
        app.routing = None;
        assert!(
            app.resume_wait.is_some(),
            "an answer on server 2 is ignored"
        );

        // The passive offer, opened from the list later, also answers on
        // server 1.
        app.active = ops2;
        app.review_offer(0);
        let prompt = app.get_prompt.take().unwrap();
        app.answer_offer(prompt, dir.join("passive.bin"), Existing::New);
        let to_one = sent(&rx1);
        assert!(
            to_one
                .iter()
                .any(|line| line.contains("DCC SEND passive.bin") && line.contains(" 77")),
            "{to_one:?}"
        );
        assert!(sent(&rx2).is_empty());
        assert!(
            app.transfers
                .iter()
                .all(|transfer| transfer.server == Some(1))
        );
        for transfer in &app.transfers {
            transfer.meter.cancel();
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_resume_request_must_name_the_file_on_that_port_and_come_in_time() {
        let (mut app, rx1, _rx2, ops1, _) = two_servers();
        app.active = ops1;
        let dir = scratch("resume-match");
        let path = dir.join("new.bin");
        std::fs::write(&path, vec![5u8; 64]).unwrap();
        app.dcc_nick = "bob".into();
        app.send_path = path.display().to_string();
        app.dcc_passive = false;
        app.offer_send();
        let port = app.transfers[0].port.unwrap();
        let _ = sent(&rx1);
        let resume = |app: &mut OpenIrcApp, filename: &str| {
            app.routing = Some(1);
            app.apply_event(Event::DccResume {
                from: "bob".into(),
                filename: filename.into(),
                port,
                position: 8,
            });
            app.routing = None;
        };
        // An old request for another file offered on the same port earlier.
        resume(&mut app, "old.bin");
        assert!(sent(&rx1).is_empty(), "no ACCEPT for a file not offered");
        resume(&mut app, "new.bin");
        let accepted = sent(&rx1);
        assert_eq!(accepted.len(), 1);
        assert!(accepted[0].contains("DCC ACCEPT new.bin"), "{accepted:?}");
        // Once the receiver connects the start is fixed, and a late
        // request gets no answer.
        let mut peer = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        peer.set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        let mut first = [0u8; 56];
        std::io::Read::read_exact(&mut peer, &mut first).unwrap();
        assert!(first.iter().all(|byte| *byte == 5), "the rest after byte 8");
        resume(&mut app, "new.bin");
        assert!(sent(&rx1).is_empty(), "no ACCEPT after the send began");
        drop(peer);
        app.transfers[0].meter.cancel();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_resume_answer_at_another_position_leaves_the_file_alone() {
        let (mut app, rx1, _rx2, ops1, _) = two_servers();
        app.active = ops1;
        let dir = scratch("resume");
        let dest = dir.join("report.bin");
        std::fs::write(&dest, vec![7u8; 100]).unwrap();
        let offer = DccSendOffer {
            filename: "report.bin".into(),
            host: "127.0.0.1".into(),
            port: 9,
            size: Some(200),
            token: None,
        };
        app.routing = Some(1);
        app.accept_file("alice".into(), offer, dest.clone(), Existing::Resume);
        app.routing = None;
        assert!(sent(&rx1)[0].contains("DCC RESUME report.bin 9 100"));
        // Someone else answering, or another server, does not count.
        for (server, from) in [(1, "eve"), (2, "alice")] {
            app.routing = Some(server);
            app.apply_event(Event::DccAccept {
                from: from.into(),
                filename: "report.bin".into(),
                port: 9,
                position: 100,
            });
            app.routing = None;
            assert!(app.resume_wait.is_some());
        }
        app.routing = Some(1);
        app.apply_event(Event::DccAccept {
            from: "alice".into(),
            filename: "report.bin".into(),
            port: 9,
            position: 0,
        });
        app.routing = None;
        assert!(app.resume_wait.is_none());
        assert!(app.transfers.is_empty());
        assert_eq!(std::fs::read(&dest).unwrap(), vec![7u8; 100]);
        assert!(
            texts(&app, 0)
                .iter()
                .any(|line| line.contains("instead of 100"))
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_receiver_of_our_file_can_resume_it() {
        let (mut app, rx1, _rx2, _, _) = two_servers();
        let dir = scratch("serve");
        let path = dir.join("song.ogg");
        std::fs::write(&path, b"0123456789").unwrap();
        app.routing = Some(1);
        app.dcc_nick = "carol".into();
        app.send_path = path.display().to_string();
        app.dcc_passive = false;
        app.offer_send();
        let offered = sent(&rx1);
        let port = app.transfers[0].port.unwrap();
        assert!(offered[0].contains(&format!(" {port} 10")), "{offered:?}");
        app.apply_event(Event::DccResume {
            from: "carol".into(),
            filename: "song.ogg".into(),
            port,
            position: 4,
        });
        app.routing = None;
        assert_eq!(
            sent(&rx1),
            vec![format!(
                "/raw PRIVMSG carol :\u{1}DCC ACCEPT song.ogg {port} 4\u{1}"
            )]
        );
        let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        let mut got = Vec::new();
        use std::io::{Read, Write};
        let mut buf = [0u8; 16];
        loop {
            let count = stream.read(&mut buf).unwrap();
            if count == 0 {
                break;
            }
            got.extend_from_slice(&buf[..count]);
            if got.len() == 6 {
                stream.write_all(&10u32.to_be_bytes()).unwrap();
            }
        }
        assert_eq!(got, b"456789");
        let _ = std::fs::remove_dir_all(dir);
    }
}
