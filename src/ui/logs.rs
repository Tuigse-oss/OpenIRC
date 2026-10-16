//! The log browser: every log under the log folder (network folders too),
//! filtered by name or searched by content, with the chosen log shown and
//! ways to rename or delete it.

use crate::i18n::{tr, trf};
use std::path::{Path, PathBuf};

use eframe::egui;

use super::{OpenIrcApp, color_hilite, dialog_pos, field, muted_on_face, text_on};
use crate::settings;

/// How much of the end of a log is shown and searched.
const TAIL: u64 = 512 * 1024;

#[derive(Default)]
pub(super) struct LogBrowser {
    pub open: bool,
    files: Vec<PathBuf>,
    /// Show files whose name contains this.
    filter: String,
    /// Text to look for inside logs.
    search: String,
    /// Files that contain `search`, after a search.
    hits: Option<Vec<PathBuf>>,
    selected: Option<PathBuf>,
    body: Vec<String>,
    rename: String,
    confirm_delete: bool,
    note: String,
    /// A search running on its own thread, and the text it looks for.
    searching: Option<(String, std::sync::mpsc::Receiver<Vec<PathBuf>>)>,
    /// The lines of `body` that hold the search text, kept until the text
    /// or the log changes.
    matching: Option<(String, Vec<usize>)>,
}

impl OpenIrcApp {
    /// Open the log browser on the active window's log.
    pub(super) fn open_logs(&mut self) {
        self.logs.open = true;
        self.logs.files = settings::list_logs(&self.settings.log_dir);
        let current = (self.active < self.windows.len()).then(|| self.log_path_for(self.active));
        let pick = current
            .filter(|path| path.is_file())
            .or_else(|| self.logs.files.first().cloned());
        self.select_log(pick);
    }

    fn select_log(&mut self, path: Option<PathBuf>) {
        let logs = &mut self.logs;
        logs.confirm_delete = false;
        logs.note.clear();
        logs.body = path
            .as_deref()
            .and_then(|path| settings::read_log_tail(path, TAIL).ok())
            .map(|text| text.lines().map(str::to_string).collect())
            .unwrap_or_default();
        logs.matching = None;
        logs.rename = path
            .as_deref()
            .and_then(Path::file_name)
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        logs.selected = path;
    }

    /// Look for the search text in every log, on a thread of its own:
    /// reading them all can take a while.
    fn start_log_search(&mut self) {
        let needle = self.logs.search.trim().to_lowercase();
        if needle.is_empty() {
            self.logs.hits = None;
            self.logs.searching = None;
            return;
        }
        let files = self.logs.files.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let wanted = needle.clone();
        std::thread::spawn(move || {
            let hits: Vec<PathBuf> = files
                .into_iter()
                .filter(|path| {
                    settings::read_log_tail(path, TAIL)
                        .is_ok_and(|text| text.to_lowercase().contains(&wanted))
                })
                .collect();
            let _ = tx.send(hits);
        });
        self.logs.searching = Some((needle, rx));
    }

    /// Take a finished search's result. True while one is still running.
    fn poll_log_search(&mut self) -> bool {
        let found = self
            .logs
            .searching
            .as_ref()
            .and_then(|(_, rx)| match rx.try_recv() {
                Ok(hits) => Some(Some(hits)),
                Err(std::sync::mpsc::TryRecvError::Empty) => None,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(None),
            });
        let Some(hits) = found else {
            return self.logs.searching.is_some();
        };
        self.logs.searching = None;
        let hits = hits.unwrap_or_default();
        if !hits.contains(self.logs.selected.as_ref().unwrap_or(&PathBuf::new())) {
            let first = hits.first().cloned();
            self.select_log(first);
        }
        self.logs.hits = Some(hits);
        false
    }

    pub(super) fn log_browser(&mut self, ctx: &egui::Context) {
        if !self.logs.open {
            return;
        }
        let root = PathBuf::from(&self.settings.log_dir);
        let mut open = true;
        let mut pick = None;
        let mut search = false;
        let mut refresh = false;
        let mut rename = false;
        let mut delete = false;
        egui::Window::new(tr("Logs"))
            .default_pos(dialog_pos(ctx, 4))
            .default_size([760.0, 440.0])
            .collapsible(false)
            .open(&mut open)
            .show(ctx, |ui| {
                let logs = &mut self.logs;
                ui.horizontal(|ui| {
                    ui.label(tr("Name"));
                    ui.add(
                        field(&mut logs.filter)
                            .desired_width(120.0)
                            .hint_text(tr("any")),
                    );
                    ui.label(tr("Text"));
                    let response = ui.add(
                        field(&mut logs.search)
                            .desired_width(160.0)
                            .hint_text(tr("find in logs")),
                    );
                    if ui.button(tr("Search")).clicked()
                        || (response.lost_focus()
                            && ui.input(|input| input.key_pressed(egui::Key::Enter)))
                    {
                        search = true;
                    }
                    if logs.hits.is_some() && ui.button(tr("Show all")).clicked() {
                        logs.hits = None;
                        logs.searching = None;
                        logs.search.clear();
                    }
                    if ui.button(tr("Refresh")).clicked() {
                        refresh = true;
                    }
                });
                if !self.settings.logging {
                    ui.label(
                        egui::RichText::new(tr(
                            "Logging is off; turn it on under Options, Logging.",
                        ))
                        .small()
                        .color(muted_on_face()),
                    );
                }
                ui.separator();
                let filter = logs.filter.to_lowercase();
                let shown: Vec<&PathBuf> = logs
                    .files
                    .iter()
                    .filter(|path| {
                        let name = shown_name(&root, path).to_lowercase();
                        filter.is_empty() || name.contains(&filter)
                    })
                    .filter(|path| logs.hits.as_ref().is_none_or(|hits| hits.contains(path)))
                    .collect();
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(220.0);
                        ui.label(
                            egui::RichText::new(super::plural(shown.len(), "{0} log", "{0} logs"))
                                .small()
                                .color(muted_on_face()),
                        );
                        egui::ScrollArea::vertical()
                            .id_salt("log-files")
                            .max_height(340.0)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                for path in &shown {
                                    let chosen = logs.selected.as_ref() == Some(*path);
                                    let label = egui::RichText::new(shown_name(&root, path));
                                    let label = if chosen {
                                        label.color(text_on(color_hilite(), egui::Color32::WHITE))
                                    } else {
                                        label
                                    };
                                    let button = egui::Button::new(label)
                                        .fill(if chosen {
                                            color_hilite()
                                        } else {
                                            egui::Color32::TRANSPARENT
                                        })
                                        .min_size(egui::vec2(ui.available_width(), 0.0));
                                    if ui.add(button).clicked() {
                                        pick = Some((*path).clone());
                                    }
                                }
                                if shown.is_empty() {
                                    ui.label(tr("No logs."));
                                }
                            });
                    });
                    ui.separator();
                    ui.vertical(|ui| {
                        let Some(selected) = logs.selected.clone() else {
                            ui.label(tr("Choose a log on the left."));
                            return;
                        };
                        ui.label(egui::RichText::new(selected.display().to_string()).small());
                        ui.horizontal(|ui| {
                            ui.add(field(&mut logs.rename).desired_width(200.0));
                            if ui.button(tr("Rename")).clicked() {
                                rename = true;
                            }
                            if logs.confirm_delete {
                                if ui.button(tr("Delete for good")).clicked() {
                                    delete = true;
                                }
                                if ui.button(tr("Keep")).clicked() {
                                    logs.confirm_delete = false;
                                }
                            } else if ui.button(tr("Delete")).clicked() {
                                logs.confirm_delete = true;
                            }
                        });
                        if !logs.note.is_empty() {
                            ui.label(egui::RichText::new(&logs.note).small());
                        }
                        let needle = logs.search.trim().to_lowercase();
                        if logs
                            .matching
                            .as_ref()
                            .is_none_or(|(made, _)| *made != needle)
                        {
                            let picked = (0..logs.body.len())
                                .filter(|&at| {
                                    needle.is_empty()
                                        || logs.body[at].to_lowercase().contains(&needle)
                                })
                                .collect();
                            logs.matching = Some((needle.clone(), picked));
                        }
                        let lines: Vec<&String> = logs
                            .matching
                            .as_ref()
                            .map(|(_, picked)| picked.iter().map(|&at| &logs.body[at]).collect())
                            .unwrap_or_default();
                        if logs.searching.is_some() {
                            ui.label(
                                egui::RichText::new(tr("Searching the logs…"))
                                    .small()
                                    .color(muted_on_face()),
                            );
                        }
                        if !needle.is_empty() {
                            ui.label(
                                egui::RichText::new(trf(
                                    "{0} with \u{201c}{1}\u{201d}",
                                    &[
                                        &super::plural(lines.len(), "{0} line", "{0} lines"),
                                        &logs.search.trim(),
                                    ],
                                ))
                                .small()
                                .color(muted_on_face()),
                            );
                        }
                        // Only the rows in sight are laid out; a long log
                        // costs no more per frame than a short one.
                        let row = ui.text_style_height(&egui::TextStyle::Monospace);
                        egui::ScrollArea::both()
                            .id_salt("log-body")
                            .max_height(300.0)
                            .stick_to_bottom(true)
                            .auto_shrink([false, false])
                            .show_rows(ui, row, lines.len(), |ui, range| {
                                for line in &lines[range] {
                                    // One row per line, as `show_rows` counts
                                    // them; long lines scroll sideways.
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(crate::colors::strip(line))
                                                .monospace(),
                                        )
                                        .wrap_mode(egui::TextWrapMode::Extend),
                                    );
                                }
                            });
                    });
                });
            });
        if refresh {
            self.logs.files = settings::list_logs(&self.settings.log_dir);
            let selected = self.logs.selected.clone().filter(|path| path.is_file());
            self.select_log(selected);
        }
        if search {
            self.start_log_search();
        }
        if self.poll_log_search() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        if let Some(path) = pick {
            self.select_log(Some(path));
        }
        if rename && let Some(from) = self.logs.selected.clone() {
            match rename_log(&from, &self.logs.rename) {
                Ok(to) => {
                    self.logs.files = settings::list_logs(&self.settings.log_dir);
                    self.select_log(Some(to));
                    self.logs.note = tr("Renamed.").into();
                }
                Err(err) => self.logs.note = err,
            }
        }
        if delete && let Some(path) = self.logs.selected.clone() {
            match std::fs::remove_file(&path) {
                Ok(()) => {
                    self.logs.files.retain(|item| item != &path);
                    let next = self.logs.files.first().cloned();
                    self.select_log(next);
                    self.logs.note = tr("Deleted.").into();
                }
                Err(err) => self.logs.note = trf("Could not delete: {0}", &[&err]),
            }
        }
        if !open {
            self.logs = LogBrowser::default();
        }
    }
}

/// A log's path below the log folder, such as `Libera/#rust.log`.
fn shown_name(root: &Path, path: &Path) -> String {
    settings::shown_log_name(
        &path
            .strip_prefix(root)
            .unwrap_or(path)
            .display()
            .to_string(),
    )
}

/// Rename a log within its folder. The new name is one file name; `.log`
/// is added when missing, and an existing file is never replaced.
fn rename_log(from: &Path, name: &str) -> Result<PathBuf, String> {
    let name = name.trim();
    if name.is_empty() || name.contains(['/', '\\']) || name.starts_with('.') {
        return Err(tr("Use a plain file name, without folders.").into());
    }
    let name = if name.ends_with(".log") {
        name.to_string()
    } else {
        format!("{name}.log")
    };
    let to = from.with_file_name(name);
    if to == from {
        return Ok(to);
    }
    if to.exists() {
        return Err(tr("A log with that name already exists.").into());
    }
    std::fs::rename(from, &to).map_err(|err| trf("Could not rename: {0}", &[&err]))?;
    Ok(to)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn searching_the_logs_runs_off_the_window_thread() {
        let dir = std::env::temp_dir().join(format!("openirc-logsearch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.log"), "nothing here\n").unwrap();
        std::fs::write(dir.join("b.log"), "the Needle is here\n").unwrap();
        let mut app = crate::ui::tests::app();
        app.logs.files = vec![dir.join("a.log"), dir.join("b.log")];
        app.logs.search = "needle".into();
        app.start_log_search();
        let started = std::time::Instant::now();
        while app.poll_log_search() && started.elapsed() < std::time::Duration::from_secs(5) {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(app.logs.hits, Some(vec![dir.join("b.log")]));
        assert_eq!(app.logs.selected, Some(dir.join("b.log")));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn rename_stays_in_the_folder_and_never_overwrites() {
        let dir = std::env::temp_dir().join(format!("openirc-rename-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let from = dir.join("#rust.log");
        std::fs::write(&from, "a\n").unwrap();
        std::fs::write(dir.join("taken.log"), "b\n").unwrap();
        assert!(rename_log(&from, "../escape").is_err());
        assert!(rename_log(&from, "taken").is_err());
        let to = rename_log(&from, "rust-old").unwrap();
        assert_eq!(to, dir.join("rust-old.log"));
        assert!(to.is_file() && !from.exists());
        assert_eq!(shown_name(&dir, &to), "rust-old.log");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
