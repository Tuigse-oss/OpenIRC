//! The script editor: several script files that run together, with line
//! numbers, find and replace, go to line, and a check for unbalanced
//! brackets and syntax errors. Quick aliases, remotes, and popups have
//! their own tabs.

use crate::i18n::{tr, trf};
use eframe::egui;

use super::{
    Alias, OpenIrcApp, Tone, color_ink, dialog_pos, field, field_multi, muted_on_face, tone_color,
};
use crate::settings::ScriptFile;

/// The editor's working copy. Running handlers keep using the saved
/// scripts until Save accepts these.
pub(super) struct ScriptDraft {
    files: Vec<ScriptFile>,
    current: usize,
    note: String,
    /// The note reports a failure, so it is shown in the warning color.
    bad: bool,
    problems: Vec<String>,
    find: String,
    replace: String,
    goto: String,
    rename: String,
    /// A file on disk to load a script from or save one to.
    path: String,
    tab: Tab,
    /// Select these characters and scroll to them on the next frame.
    select: Option<(usize, usize)>,
}

impl ScriptDraft {
    fn set_note(&mut self, outcome: Result<String, String>) {
        (self.note, self.bad) = match outcome {
            Ok(note) => (note, false),
            Err(note) => (note, true),
        };
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Files,
    Aliases,
    Remotes,
    Popups,
}

const EDITOR_ID: &str = "script-editor";

impl OpenIrcApp {
    pub(super) fn open_scripts(&mut self) {
        if self.script_draft.is_some() {
            return;
        }
        let mut files = self.settings.scripts.clone();
        if files.is_empty() {
            files.push(ScriptFile {
                name: "main".into(),
                text: self.settings.script_text.clone(),
                enabled: true,
            });
        }
        let rename = files[0].name.clone();
        self.script_draft = Some(ScriptDraft {
            files,
            current: 0,
            note: String::new(),
            bad: false,
            problems: Vec::new(),
            find: String::new(),
            replace: String::new(),
            goto: String::new(),
            rename,
            path: crate::settings::config_dir()
                .join("scripts")
                .join("script.txt")
                .display()
                .to_string(),
            tab: Tab::Files,
            select: None,
        });
    }

    pub(super) fn scripts_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut draft) = self.script_draft.take() else {
            return;
        };
        let mut open = true;
        let mut keep = true;
        let max_height = (ctx.content_rect().height() - 80.0).max(260.0);
        egui::Window::new(tr("Scripts"))
            .collapsible(false)
            .resizable([true, true])
            .default_size([760.0, 520.0])
            .max_height(max_height)
            .default_pos(dialog_pos(ctx, 1))
            .open(&mut open)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    for (tab, label) in [
                        (Tab::Files, tr("Script files")),
                        (Tab::Aliases, tr("Quick aliases")),
                        (Tab::Remotes, tr("Remotes")),
                        (Tab::Popups, tr("Popups")),
                    ] {
                        ui.selectable_value(&mut draft.tab, tab, label);
                    }
                });
                ui.separator();
                match draft.tab {
                    Tab::Files => keep = self.script_files_tab(ui, &mut draft),
                    Tab::Aliases => self.aliases_tab(ui),
                    Tab::Remotes => self.remotes_tab(ui),
                    Tab::Popups => self.popups_tab(ui),
                }
            });
        if open && keep {
            self.script_draft = Some(draft);
        }
    }

    /// The file list and editor. Returns false when Close was pressed.
    fn script_files_tab(&mut self, ui: &mut egui::Ui, draft: &mut ScriptDraft) -> bool {
        let mut keep = true;
        let saved_files = if self.settings.scripts.is_empty() {
            vec![ScriptFile {
                name: "main".into(),
                text: self.settings.script_text.clone(),
                enabled: true,
            }]
        } else {
            self.settings.scripts.clone()
        };
        let changed = draft.files != saved_files;
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(170.0);
                ui.label(egui::RichText::new(tr("Files (ticked ones run)")).small());
                let mut pick = None;
                for (index, file) in draft.files.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut file.enabled, "")
                            .on_hover_text(tr("Run this file"));
                        if ui
                            .selectable_label(draft.current == index, &file.name)
                            .clicked()
                        {
                            pick = Some(index);
                        }
                    });
                }
                if let Some(index) = pick {
                    draft.current = index;
                    draft.rename = draft.files[index].name.clone();
                    draft.select = None;
                }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if ui.small_button(tr("New")).clicked() {
                        let name = fresh_name(&draft.files);
                        draft.files.push(ScriptFile {
                            name: name.clone(),
                            text: String::new(),
                            enabled: true,
                        });
                        draft.current = draft.files.len() - 1;
                        draft.rename = name;
                    }
                    if ui
                        .add_enabled(
                            draft.files.len() > 1,
                            egui::Button::new(tr("Delete")).small(),
                        )
                        .on_hover_text(tr("Remove this file when you save"))
                        .clicked()
                    {
                        draft.files.remove(draft.current);
                        draft.current = draft.current.min(draft.files.len() - 1);
                        draft.rename = draft.files[draft.current].name.clone();
                    }
                    if ui
                        .add_enabled(draft.current > 0, egui::Button::new("↑").small())
                        .on_hover_text(tr("Run earlier"))
                        .clicked()
                    {
                        draft.files.swap(draft.current, draft.current - 1);
                        draft.current -= 1;
                    }
                    if ui
                        .add_enabled(
                            draft.current + 1 < draft.files.len(),
                            egui::Button::new("↓").small(),
                        )
                        .on_hover_text(tr("Run later"))
                        .clicked()
                    {
                        draft.files.swap(draft.current, draft.current + 1);
                        draft.current += 1;
                    }
                });
                ui.horizontal(|ui| {
                    ui.add(field(&mut draft.rename).desired_width(100.0));
                    let name = draft.rename.trim().to_string();
                    let taken = draft
                        .files
                        .iter()
                        .enumerate()
                        .any(|(index, file)| index != draft.current && file.name == name);
                    if ui
                        .add_enabled(
                            !name.is_empty() && !taken,
                            egui::Button::new(tr("Rename")).small(),
                        )
                        .clicked()
                    {
                        draft.files[draft.current].name = name;
                    }
                });
                ui.add_space(4.0);
                ui.label(egui::RichText::new(tr("File on disk")).small());
                ui.add(
                    field(&mut draft.path)
                        .hint_text(tr("path to a .txt script"))
                        .desired_width(166.0),
                );
                ui.horizontal(|ui| {
                    if ui
                        .small_button(tr("Load"))
                        .on_hover_text(tr("Add the file on disk as a new script file"))
                        .clicked()
                    {
                        let done = load_script(draft);
                        draft.set_note(done);
                    }
                    if ui
                        .small_button(tr("Save to disk"))
                        .on_hover_text(tr("Write this script file to the file on disk"))
                        .clicked()
                    {
                        let done = export_script(draft);
                        draft.set_note(done);
                    }
                });
            });
            ui.separator();
            ui.vertical(|ui| {
                self.editor_toolbar(ui, draft);
                editor(ui, draft);
            });
        });
        ui.separator();
        ui.horizontal(|ui| {
            if ui
                .add_enabled(changed, egui::Button::new(tr("Save scripts")))
                .clicked()
            {
                draft.problems = check_files(&draft.files);
                if draft.problems.is_empty() {
                    self.settings.scripts = draft.files.clone();
                    self.settings.rebuild_script();
                    self.script_fault.clear();
                    self.persist();
                    draft.set_note(Ok(tr("Scripts saved.").into()));
                } else {
                    draft.set_note(Err(tr("Not saved: fix the problems below first.").into()));
                }
            }
            if ui
                .add_enabled(changed, egui::Button::new(tr("Revert")))
                .clicked()
            {
                draft.files = saved_files.clone();
                draft.current = draft.current.min(draft.files.len() - 1);
                draft.note.clear();
                draft.problems.clear();
            }
            if ui.button(tr("Close")).clicked() {
                keep = false;
            }
            if changed {
                ui.label(egui::RichText::new(tr("Unsaved changes")).color(muted_on_face()));
            }
        });
        if !draft.note.is_empty() {
            let color = if draft.bad {
                tone_color(Tone::Attention)
            } else {
                color_ink()
            };
            ui.colored_label(color, &draft.note);
        }
        for problem in &draft.problems {
            ui.colored_label(tone_color(Tone::Attention), problem);
        }
        if !self.script_fault.is_empty() {
            ui.colored_label(
                tone_color(Tone::Attention),
                trf("Running script error: {0}", &[&self.script_fault]),
            );
        }
        keep
    }

    fn editor_toolbar(&mut self, ui: &mut egui::Ui, draft: &mut ScriptDraft) {
        let ctx = ui.ctx().clone();
        let id = egui::Id::new(EDITOR_ID);
        let cursor = egui::text_edit::TextEditState::load(&ctx, id)
            .and_then(|state| state.cursor.char_range())
            .map(|range| {
                let (a, b) = (range.primary.index.0, range.secondary.index.0);
                (a.min(b), a.max(b))
            });
        ui.horizontal(|ui| {
            ui.label(tr("Find"));
            let find = ui.add(field(&mut draft.find).desired_width(120.0));
            ui.label(tr("Replace with"));
            ui.add(field(&mut draft.replace).desired_width(120.0));
            let text = &draft.files[draft.current].text;
            let enter = find.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
            if ui.button(tr("Next")).clicked() || enter {
                let from = cursor.map_or(0, |(_, end)| end);
                match find_next(text, &draft.find, from) {
                    Some(found) => {
                        draft.select = Some(found);
                        draft.note.clear();
                    }
                    None => {
                        draft.note = trf("\u{201c}{0}\u{201d} is not in this file.", &[&draft.find])
                    }
                }
            }
            if ui.button(tr("Replace")).clicked() {
                let selected = cursor.filter(|(start, end)| {
                    let chosen: String = text.chars().skip(*start).take(end - start).collect();
                    !draft.find.is_empty() && chosen.to_lowercase() == draft.find.to_lowercase()
                });
                let file = &mut draft.files[draft.current];
                match selected {
                    Some((start, end)) => {
                        file.text = splice(&file.text, start, end, &draft.replace);
                        let after = start + draft.replace.chars().count();
                        draft.select = find_next(&file.text, &draft.find, after);
                    }
                    None => draft.select = find_next(&file.text, &draft.find, 0),
                }
            }
            if ui
                .button(tr("All"))
                .on_hover_text(tr("Replace every match"))
                .clicked()
            {
                let file = &mut draft.files[draft.current];
                let (text, count) = replace_all(&file.text, &draft.find, &draft.replace);
                file.text = text;
                draft.note = trf(
                    "Replaced {0}.",
                    &[&super::plural(count, "{0} match", "{0} matches")],
                );
            }
        });
        ui.horizontal(|ui| {
            ui.label(tr("Line"));
            let goto = ui.add(field(&mut draft.goto).desired_width(50.0));
            let enter = goto.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
            if ui.button(tr("Go")).clicked() || enter {
                let text = &draft.files[draft.current].text;
                match draft
                    .goto
                    .trim()
                    .parse::<usize>()
                    .ok()
                    .and_then(|line| line_start(text, line))
                {
                    Some(start) => draft.select = Some((start, start)),
                    None => draft.note = tr("There is no such line.").into(),
                }
            }
            if ui
                .button(tr("Check"))
                .on_hover_text(tr("Look for unbalanced brackets and syntax errors"))
                .clicked()
            {
                draft.problems = check_files(&draft.files);
                draft.note = if draft.problems.is_empty() {
                    tr("No problems found.").into()
                } else {
                    String::new()
                };
            }
            let (line, column) = cursor
                .map(|(start, _)| line_and_column(&draft.files[draft.current].text, start))
                .unwrap_or((1, 1));
            ui.label(
                egui::RichText::new(trf("Line {0}, column {1}", &[&line, &column]))
                    .small()
                    .color(muted_on_face()),
            );
        });
    }

    fn aliases_tab(&mut self, ui: &mut egui::Ui) {
        ui.label(tr(
            "A quick alias runs when you type /name; its body is sent as a command.",
        ));
        ui.horizontal(|ui| {
            ui.label(tr("Name"));
            ui.add(field(&mut self.alias_name).desired_width(100.0));
            ui.label(tr("Body"));
            ui.add(field(&mut self.alias_body).desired_width(280.0));
            if ui.button(tr("Add")).clicked() {
                let name = self.alias_name.trim().trim_start_matches('/').to_string();
                let body = self.alias_body.trim().to_string();
                if !name.is_empty() && !body.is_empty() {
                    self.settings.aliases.push(Alias { name, body });
                    self.alias_name.clear();
                    self.alias_body.clear();
                    self.persist();
                }
            }
        });
        let mut remove = None;
        egui::ScrollArea::vertical()
            .id_salt("aliases")
            .max_height(360.0)
            .show(ui, |ui| {
                for (index, alias) in self.settings.aliases.iter().enumerate() {
                    ui.horizontal(|ui| {
                        if ui.small_button(tr("Remove")).clicked() {
                            remove = Some(index);
                        }
                        ui.label(format!("/{} → {}", alias.name, alias.body));
                    });
                }
                if self.settings.aliases.is_empty() {
                    ui.label(tr("No quick aliases."));
                }
            });
        if let Some(index) = remove {
            self.settings.aliases.remove(index);
            self.persist();
        }
    }

    fn remotes_tab(&mut self, ui: &mut egui::Ui) {
        ui.label(tr("A remote runs its body when an event (text, action, join, notice, ...) matches the mask."));
        ui.horizontal(|ui| {
            ui.label(tr("Event"));
            ui.add(field(&mut self.remote_event).desired_width(70.0));
            ui.label(tr("Mask"));
            ui.add(field(&mut self.remote_mask).desired_width(90.0));
            ui.label(tr("Body"));
            ui.add(field(&mut self.remote_body).desired_width(220.0));
            if ui.button(tr("Add")).clicked() {
                let event = self.remote_event.trim().to_string();
                let body = self.remote_body.trim().to_string();
                if !event.is_empty() && !body.is_empty() {
                    self.settings.remotes.push(crate::script::Remote {
                        event,
                        mask: self.remote_mask.trim().to_string(),
                        body,
                    });
                    self.remote_body.clear();
                    self.persist();
                }
            }
        });
        let mut remove = None;
        egui::ScrollArea::vertical()
            .id_salt("remotes")
            .max_height(360.0)
            .show(ui, |ui| {
                for (index, remote) in self.settings.remotes.iter().enumerate() {
                    ui.horizontal(|ui| {
                        if ui.small_button(tr("Remove")).clicked() {
                            remove = Some(index);
                        }
                        ui.label(format!(
                            "on {} {} {}",
                            remote.event, remote.mask, remote.body
                        ));
                    });
                }
                if self.settings.remotes.is_empty() {
                    ui.label(tr("No remotes."));
                }
            });
        if let Some(index) = remove {
            self.settings.remotes.remove(index);
            self.persist();
        }
    }

    fn popups_tab(&mut self, ui: &mut egui::Ui) {
        ui.label(tr(
            "A popup is an item in a right-click menu. $nick is the person, $chan the channel.",
        ));
        ui.horizontal(|ui| {
            ui.label(tr("Menu"));
            let shown = crate::settings::POPUP_MENUS
                .iter()
                .find(|(key, _)| *key == self.popup_menu)
                .map_or(tr("Transcript line"), |(_, label)| tr(label));
            egui::ComboBox::from_id_salt("popup-menu")
                .selected_text(shown)
                .show_ui(ui, |ui| {
                    for (key, label) in crate::settings::POPUP_MENUS {
                        ui.selectable_value(&mut self.popup_menu, key.to_string(), tr(label));
                    }
                });
            ui.label(tr("Label"));
            ui.add(field(&mut self.popup_label).desired_width(110.0));
            ui.label(tr("Body"));
            ui.add(field(&mut self.popup_body).desired_width(220.0));
            if ui.button(tr("Add")).clicked() {
                let label = self.popup_label.trim().to_string();
                let body = self.popup_body.trim().to_string();
                if !label.is_empty() && !body.is_empty() {
                    self.settings.popups.push(crate::settings::Popup {
                        label,
                        body,
                        menu: self.popup_menu.clone(),
                    });
                    self.popup_label.clear();
                    self.popup_body.clear();
                    self.persist();
                }
            }
        });
        let mut remove = None;
        egui::ScrollArea::vertical()
            .id_salt("popups")
            .max_height(360.0)
            .show(ui, |ui| {
                egui::Grid::new("popup-list")
                    .num_columns(4)
                    .spacing([10.0, 4.0])
                    .striped(true)
                    .show(ui, |ui| {
                        for (index, popup) in self.settings.popups.iter().enumerate() {
                            if ui.small_button(tr("Remove")).clicked() {
                                remove = Some(index);
                            }
                            let menu = crate::settings::POPUP_MENUS
                                .iter()
                                .find(|(key, _)| *key == popup.menu)
                                .map_or("Transcript line", |(_, label)| label);
                            ui.label(egui::RichText::new(tr(menu)).color(muted_on_face()));
                            ui.label(egui::RichText::new(&popup.label).strong());
                            ui.label(&popup.body);
                            ui.end_row();
                        }
                    });
                if self.settings.popups.is_empty() {
                    ui.label(tr("No popups."));
                }
            });
        if let Some(index) = remove {
            self.settings.popups.remove(index);
            self.persist();
        }
    }
}

/// The text box with a line-number gutter, scrolled together.
fn editor(ui: &mut egui::Ui, draft: &mut ScriptDraft) {
    let id = egui::Id::new(EDITOR_ID);
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    let row = ui.fonts_mut(|fonts| fonts.row_height(&font));
    let margin = egui::Margin::symmetric(4, 2);
    let file = &mut draft.files[draft.current];
    let lines = file.text.split('\n').count().max(1);
    let mut scroll = egui::ScrollArea::both()
        .id_salt(("script-scroll", draft.current))
        .max_height(340.0)
        .auto_shrink([false, false]);
    if let Some((start, _)) = draft.select {
        let (line, _) = line_and_column(&file.text, start);
        let target = (line.saturating_sub(4) as f32) * row;
        scroll = scroll.vertical_scroll_offset(target);
    }
    let ink = super::color_ink();
    let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap: f32| {
        let mut job = highlight(text.as_str(), font.clone(), ink);
        job.wrap.max_width = wrap;
        ui.fonts_mut(|fonts| fonts.layout_job(job))
    };
    scroll.show(ui, |ui| {
        ui.horizontal_top(|ui| {
            let numbers: String = (1..=lines).map(|line| format!("{line:>4}\n")).collect();
            ui.add_space(0.0);
            ui.vertical(|ui| {
                ui.add_space(f32::from(margin.top));
                ui.label(
                    egui::RichText::new(numbers.trim_end())
                        .font(font.clone())
                        .color(muted_on_face()),
                );
            });
            ui.add(
                field_multi(&mut file.text)
                    .id(id)
                    .font(egui::TextStyle::Monospace)
                    .code_editor()
                    .layouter(&mut layouter)
                    .margin(margin)
                    .desired_rows(16)
                    .desired_width(f32::INFINITY)
                    .lock_focus(true),
            );
        });
    });
    if let Some((start, end)) = draft.select.take() {
        let mut state = egui::text_edit::TextEditState::load(ui.ctx(), id).unwrap_or_default();
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::two(
                egui::text::CCursor::new(start),
                egui::text::CCursor::new(end),
            )));
        state.store(ui.ctx(), id);
        ui.ctx().memory_mut(|memory| memory.request_focus(id));
    }
}

/// What a piece of script text is, for coloring.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Part {
    Plain,
    Keyword,
    Identifier,
    Variable,
    Comment,
    Bracket,
}

/// Split script text into colored parts: `;` comments, the block words
/// (`on`, `alias`, `if`, …), `$identifiers`, `%variables`, and brackets.
fn script_parts(text: &str) -> Vec<(Part, std::ops::Range<usize>)> {
    const KEYWORDS: [&str; 8] = [
        "on", "alias", "if", "elseif", "else", "while", "return", "halt",
    ];
    let mut parts = Vec::new();
    let mut line_start = 0;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if trimmed.starts_with(';') {
            parts.push((Part::Comment, line_start..line_start + line.len()));
            line_start += line.len();
            continue;
        }
        let bytes = line.as_bytes();
        let mut at = 0;
        while at < bytes.len() {
            let ch = bytes[at];
            let start = at;
            let part = if ch == b'$' || ch == b'%' {
                at += 1;
                while at < bytes.len()
                    && (bytes[at].is_ascii_alphanumeric()
                        || bytes[at] == b'_'
                        || bytes[at] == b'-' && ch == b'$')
                {
                    at += 1;
                }
                if at == start + 1 {
                    Part::Plain
                } else if ch == b'$' {
                    Part::Identifier
                } else {
                    Part::Variable
                }
            } else if ch.is_ascii_alphabetic() {
                while at < bytes.len() && bytes[at].is_ascii_alphanumeric() {
                    at += 1;
                }
                let word = &line[start..at];
                let at_word_start = start == 0 || !bytes[start - 1].is_ascii_alphanumeric();
                if at_word_start && KEYWORDS.iter().any(|key| key.eq_ignore_ascii_case(word)) {
                    Part::Keyword
                } else {
                    Part::Plain
                }
            } else if matches!(ch, b'{' | b'}' | b'(' | b')') {
                at += 1;
                Part::Bracket
            } else {
                // Skip to the next byte that could start something, staying
                // on character boundaries.
                at += 1;
                while at < bytes.len() && !line.is_char_boundary(at) {
                    at += 1;
                }
                Part::Plain
            };
            match parts.last_mut() {
                Some((last, range)) if *last == part && range.end == line_start + start => {
                    range.end = line_start + at;
                }
                _ => parts.push((part, line_start + start..line_start + at)),
            }
        }
        line_start += line.len();
    }
    parts
}

fn highlight(text: &str, font: egui::FontId, ink: egui::Color32) -> egui::text::LayoutJob {
    let dark = super::color_paper().r() < 128;
    let color = |part: Part| match (part, dark) {
        (Part::Plain, _) => ink,
        (Part::Keyword, false) => egui::Color32::from_rgb(0, 60, 170),
        (Part::Keyword, true) => egui::Color32::from_rgb(110, 160, 255),
        (Part::Identifier, false) => egui::Color32::from_rgb(140, 0, 140),
        (Part::Identifier, true) => egui::Color32::from_rgb(220, 140, 230),
        (Part::Variable, false) => egui::Color32::from_rgb(160, 80, 0),
        (Part::Variable, true) => egui::Color32::from_rgb(240, 180, 90),
        (Part::Comment, false) => egui::Color32::from_rgb(0, 120, 0),
        (Part::Comment, true) => egui::Color32::from_rgb(110, 190, 110),
        (Part::Bracket, false) => egui::Color32::from_rgb(120, 120, 120),
        (Part::Bracket, true) => egui::Color32::from_rgb(170, 170, 170),
    };
    let mut job = egui::text::LayoutJob::default();
    for (part, range) in script_parts(text) {
        job.append(
            &text[range],
            0.0,
            egui::TextFormat {
                font_id: font.clone(),
                color: color(part),
                ..Default::default()
            },
        );
    }
    job
}

/// `script`, `script 2`, … whichever is free.
/// Add the script at `draft.path` as a new file in the editor. Returns a
/// note for the user, as an error when it failed.
fn load_script(draft: &mut ScriptDraft) -> Result<String, String> {
    let path = std::path::PathBuf::from(draft.path.trim());
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(err) => return Err(trf("Could not read {0}: {1}", &[&path.display(), &err])),
    };
    let text = crate::message::decode_line(&bytes).replace("\r\n", "\n");
    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().trim().to_string())
        .filter(|stem| !stem.is_empty())
        .unwrap_or_else(|| "loaded".into());
    let mut name = stem.clone();
    let mut number = 2;
    while draft.files.iter().any(|file| file.name == name) {
        name = format!("{stem}{number}");
        number += 1;
    }
    draft.files.push(ScriptFile {
        name: name.clone(),
        text,
        // A script from elsewhere runs only once the user ticks it.
        enabled: false,
    });
    draft.current = draft.files.len() - 1;
    draft.rename = name.clone();
    draft.select = None;
    Ok(trf(
        "Loaded {0} as {1}. Tick it to run it, then Save.",
        &[&file_name(&path), &name],
    ))
}

/// The last part of a path, for short notes (the full path is in the field).
fn file_name(path: &std::path::Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// Write the current script file to `draft.path`.
fn export_script(draft: &ScriptDraft) -> Result<String, String> {
    let path = std::path::PathBuf::from(draft.path.trim());
    let Some(file) = draft.files.get(draft.current) else {
        return Ok(String::new());
    };
    if draft.path.trim().is_empty() {
        return Err(tr("Type a path first.").into());
    }
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
        && let Err(err) = std::fs::create_dir_all(parent)
    {
        return Err(trf("Could not create {0}: {1}", &[&parent.display(), &err]));
    }
    match std::fs::write(&path, &file.text) {
        Ok(()) => Ok(trf("Saved {0} to {1}.", &[&file.name, &file_name(&path)])),
        Err(err) => Err(trf("Could not write {0}: {1}", &[&path.display(), &err])),
    }
}

fn fresh_name(files: &[ScriptFile]) -> String {
    (1..)
        .map(|number| {
            if number == 1 {
                "script".to_string()
            } else {
                format!("script {number}")
            }
        })
        .find(|name| files.iter().all(|file| &file.name != name))
        .unwrap_or_default()
}

/// Problems in the enabled files: brackets first, then the parser's error.
fn check_files(files: &[ScriptFile]) -> Vec<String> {
    let mut problems = Vec::new();
    for file in files.iter().filter(|file| file.enabled) {
        let brackets = bracket_problems(&file.text);
        if brackets.is_empty() {
            if let Err(err) = crate::slang::check(&file.text) {
                problems.push(format!("{}: {err}", file.name));
            }
        } else {
            problems.extend(
                brackets
                    .into_iter()
                    .map(|problem| format!("{}: {problem}", file.name)),
            );
        }
    }
    problems
}

/// Unbalanced `{}` and `()`, with line numbers. Comment lines (starting
/// with `;`) are skipped.
fn bracket_problems(text: &str) -> Vec<String> {
    let mut open: Vec<(char, usize)> = Vec::new();
    let mut problems = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let number = index + 1;
        if line.trim_start().starts_with(';') {
            continue;
        }
        for ch in line.chars() {
            match ch {
                '{' | '(' => open.push((ch, number)),
                '}' | ')' => {
                    let want = if ch == '}' { '{' } else { '(' };
                    match open.last() {
                        Some((last, _)) if *last == want => {
                            open.pop();
                        }
                        Some((last, at)) => problems.push(format!(
                            "line {number}: {ch} does not match the {last} on line {at}"
                        )),
                        None => problems.push(trf(
                            "line {0}: {1} has no opening {2}",
                            &[&number, &ch, &want],
                        )),
                    }
                }
                _ => {}
            }
        }
    }
    for (ch, at) in open.into_iter().rev() {
        problems.push(trf("line {0}: {1} is never closed", &[&at, &ch]));
    }
    problems
}

/// The character index where 1-based `line` starts.
fn line_start(text: &str, line: usize) -> Option<usize> {
    if line == 0 {
        return None;
    }
    if line == 1 {
        return Some(0);
    }
    let mut seen = 1;
    for (index, ch) in text.chars().enumerate() {
        if ch == '\n' {
            seen += 1;
            if seen == line {
                return Some(index + 1);
            }
        }
    }
    None
}

/// 1-based line and column of a character index.
fn line_and_column(text: &str, at: usize) -> (usize, usize) {
    let mut line = 1;
    let mut column = 1;
    for ch in text.chars().take(at) {
        if ch == '\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }
    (line, column)
}

/// The next match of `needle` (ignoring case) at or after character `from`,
/// wrapping to the start; as character indices.
fn find_next(text: &str, needle: &str, from: usize) -> Option<(usize, usize)> {
    if needle.is_empty() {
        return None;
    }
    let hay: Vec<char> = text.chars().flat_map(char::to_lowercase).collect();
    let want: Vec<char> = needle.chars().flat_map(char::to_lowercase).collect();
    // Lowercasing that changes the length would shift indices; match
    // exactly then.
    let (hay, want) = if hay.len() == text.chars().count() && want.len() == needle.chars().count() {
        (hay, want)
    } else {
        (text.chars().collect(), needle.chars().collect())
    };
    let matches_at = |at: usize| hay.get(at..at + want.len()) == Some(&want[..]);
    let len = hay.len();
    (from..len)
        .chain(0..from.min(len))
        .find(|&at| matches_at(at))
        .map(|at| (at, at + want.len()))
}

fn splice(text: &str, start: usize, end: usize, with: &str) -> String {
    let mut out: String = text.chars().take(start).collect();
    out.push_str(with);
    out.extend(text.chars().skip(end));
    out
}

/// Replace every match of `needle` (ignoring case); the count of matches.
fn replace_all(text: &str, needle: &str, with: &str) -> (String, usize) {
    let mut out = text.to_string();
    let mut count = 0;
    let mut from = 0;
    while let Some((start, end)) = find_next(&out, needle, from) {
        if start < from || count > 100_000 {
            break;
        }
        out = splice(&out, start, end, with);
        from = start + with.chars().count();
        count += 1;
    }
    (out, count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_text_is_split_for_coloring() {
        let text = "; note\nalias hi {\n  echo $nick %count $1-\n}\n";
        let parts = script_parts(text);
        let covered: usize = parts.iter().map(|(_, range)| range.len()).sum();
        assert_eq!(covered, text.len(), "every byte is in one part");
        let kinds: Vec<(Part, &str)> = parts
            .iter()
            .filter(|(part, _)| *part != Part::Plain)
            .map(|(part, range)| (*part, &text[range.clone()]))
            .collect();
        assert_eq!(
            kinds,
            vec![
                (Part::Comment, "; note\n"),
                (Part::Keyword, "alias"),
                (Part::Bracket, "{"),
                (Part::Identifier, "$nick"),
                (Part::Variable, "%count"),
                (Part::Identifier, "$1-"),
                (Part::Bracket, "}"),
            ]
        );
        assert_eq!(
            script_parts("héllo $wörld")
                .iter()
                .map(|(_, r)| r.len())
                .sum::<usize>(),
            "héllo $wörld".len()
        );
    }

    #[test]
    fn brackets_are_checked_with_line_numbers() {
        assert!(bracket_problems("alias a {\n  echo $calc((1+2))\n}\n").is_empty());
        assert_eq!(
            bracket_problems("alias a {\n  if (x {\n}\n"),
            vec!["line 2: ( is never closed", "line 1: { is never closed"]
        );
        assert_eq!(
            bracket_problems("alias a {\n  echo (x}\n"),
            vec![
                "line 2: } does not match the ( on line 2",
                "line 2: ( is never closed",
                "line 1: { is never closed"
            ]
        );
        assert_eq!(
            bracket_problems("; a } comment\n}"),
            vec!["line 2: } has no opening {"]
        );
    }

    #[test]
    fn find_replace_and_lines() {
        let text = "Echo one\necho two\nECHO three";
        assert_eq!(find_next(text, "echo", 1), Some((9, 13)));
        assert_eq!(find_next(text, "echo", 20), Some((0, 4)), "wraps around");
        assert_eq!(find_next(text, "missing", 0), None);
        let (replaced, count) = replace_all(text, "echo", "say");
        assert_eq!(replaced, "say one\nsay two\nsay three");
        assert_eq!(count, 3);
        assert_eq!(
            replace_all("aaa", "a", "aa").0,
            "aaaaaa",
            "replacement text is not searched again"
        );
        assert_eq!(line_start(text, 2), Some(9));
        assert_eq!(line_start(text, 4), None);
        assert_eq!(line_and_column(text, 11), (2, 3));
    }

    #[test]
    fn new_files_get_free_names() {
        let file = |name: &str| ScriptFile {
            name: name.into(),
            text: String::new(),
            enabled: true,
        };
        assert_eq!(fresh_name(&[file("main")]), "script");
        assert_eq!(fresh_name(&[file("script"), file("script 2")]), "script 3");
        let problems = check_files(&[ScriptFile {
            name: "bad".into(),
            text: "nonsense".into(),
            enabled: true,
        }]);
        assert_eq!(problems.len(), 1);
        assert!(problems[0].starts_with("bad: line 1:"), "{problems:?}");
    }

    #[test]
    fn scripts_load_from_and_save_to_disk() {
        let folder = std::env::temp_dir().join(format!("openirc-scripts-{}", std::process::id()));
        let path = folder.join("away.txt");
        let mut draft = ScriptDraft {
            files: vec![ScriptFile {
                name: "main".into(),
                text: "alias hi /say hi".into(),
                enabled: true,
            }],
            current: 0,
            note: String::new(),
            bad: false,
            problems: Vec::new(),
            find: String::new(),
            replace: String::new(),
            goto: String::new(),
            rename: "main".into(),
            path: path.display().to_string(),
            tab: Tab::Files,
            select: None,
        };
        let note = export_script(&draft).unwrap();
        assert!(note.starts_with("Saved main"), "{note}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "alias hi /say hi");
        std::fs::write(&path, "alias a /echo a\r\nalias b /echo b\r\n").unwrap();
        let note = load_script(&mut draft).unwrap();
        assert!(note.starts_with("Loaded"), "{note}");
        assert_eq!(draft.files.len(), 2);
        assert_eq!(draft.files[1].name, "away");
        assert_eq!(draft.files[1].text, "alias a /echo a\nalias b /echo b\n");
        assert!(
            !draft.files[1].enabled,
            "a loaded script waits to be ticked"
        );
        assert_eq!(draft.current, 1);
        load_script(&mut draft).unwrap();
        assert_eq!(draft.files[2].name, "away2");
        draft.path = folder.join("missing.txt").display().to_string();
        assert!(
            load_script(&mut draft)
                .unwrap_err()
                .starts_with("Could not read")
        );
        let _ = std::fs::remove_dir_all(&folder);
    }
}
