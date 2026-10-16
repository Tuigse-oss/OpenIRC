//! The window's main area: topic bar, Find bar, transcript, nickname list,
//! and the input line with history, completion, and formatting keys.

use crate::i18n::{tr, trf};
use eframe::egui;

use super::chrome::NickSide;
use super::{
    Kind, LineAction, NICK_ACTIONS, OpenIrcApp, TextLook, bare_nick, color_hilite, color_ink,
    color_paper, field, line_galley, link_clicks, muted_color, palette_color, paper, same_nick,
    text_on, topic_line,
};

impl OpenIrcApp {
    pub(super) fn client(&mut self, ui: &mut egui::Ui) {
        egui::CentralPanel::default_margins().show(ui, |ui| {
            let Some(active) = self.windows.get(self.active).map(|window| window.kind) else {
                return;
            };
            let font = self.view_settings().font_size as f32;
            let title = self.windows[self.active].title.clone();
            let mut actions: Vec<LineAction> = Vec::new();

            egui::Panel::top("client-head")
                .frame(egui::Frame::NONE)
                .show_separator_line(false)
                .show(ui, |ui| {
                    self.head_bar(ui, active, font, &mut actions);
                    ui.add_space(3.0);
                });

            egui::Panel::bottom("client-input")
                .frame(egui::Frame::NONE)
                .show_separator_line(false)
                .show(ui, |ui| {
                    ui.add_space(3.0);
                    self.input_line(ui, font);
                });

            let side = NickSide::from_setting(&self.settings.nicklist);
            if active == Kind::Channel && side != NickSide::Hidden {
                let panel = match side {
                    NickSide::Left => egui::Panel::left("client-nicks"),
                    _ => egui::Panel::right("client-nicks"),
                };
                panel
                    .frame(egui::Frame::NONE)
                    .resizable(true)
                    .default_size(160.0)
                    .min_size(100.0)
                    .show_separator_line(false)
                    .show(ui, |ui| {
                        ui.add_space(0.0);
                        let margin = if side == NickSide::Left {
                            egui::Margin {
                                right: 3,
                                ..Default::default()
                            }
                        } else {
                            egui::Margin {
                                left: 3,
                                ..Default::default()
                            }
                        };
                        egui::Frame::NONE.outer_margin(margin).show(ui, |ui| {
                            paper().show(ui, |ui| {
                                ui.set_min_size(ui.available_size());
                                self.nick_list(ui, font, &mut actions);
                            });
                        });
                    });
            }

            egui::CentralPanel::no_frame().show(ui, |ui| {
                paper().show(ui, |ui| {
                    ui.set_min_size(ui.available_size());
                    let started = self.profile.start();
                    self.transcript(ui, font, &mut actions);
                    self.profile.stop("ui.transcript", started);
                });
            });

            for action in actions {
                self.run_line_action(&title, action);
            }
        });
        self.paste_dialog(ui.ctx());
    }

    /// Topic and channel details for channels; a title bar for other
    /// windows; then the Find bar.
    fn head_bar(
        &mut self,
        ui: &mut egui::Ui,
        kind: Kind,
        font: f32,
        actions: &mut Vec<LineAction>,
    ) {
        let window = &self.windows[self.active];
        let title = window.title.clone();
        let modes = window.modes.clone();
        let users = window.nicks.len();
        let topic = window.topic.clone();
        let title_font = egui::FontId::new(font + 1.0, egui::FontFamily::Name("bold".into()));
        let mut edit_channel = false;
        paper().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(&title)
                        .font(title_font)
                        .color(color_ink()),
                );
                match kind {
                    Kind::Channel => {
                        let mut details = Vec::new();
                        // Only the letters: a channel key is not shown.
                        let letters = modes.split_whitespace().next().unwrap_or("");
                        if !letters.is_empty() {
                            details.push(letters.to_string());
                        }
                        details.push(super::plural(users, "{0} user", "{0} users"));
                        ui.label(egui::RichText::new(details.join("  ·  ")).color(muted_color()));
                    }
                    Kind::Query => {
                        ui.label(
                            egui::RichText::new(tr("private conversation")).color(muted_color()),
                        );
                    }
                    Kind::Status => {
                        ui.label(egui::RichText::new(tr("server messages")).color(muted_color()));
                    }
                    Kind::List | Kind::Dcc => {}
                }
            });
            if kind == Kind::Channel {
                let topic = topic
                    .filter(|topic| !topic.is_empty())
                    .unwrap_or_else(|| tr("No topic is set.").into());
                let line = topic_line(&topic);
                let width = ui.available_width();
                let look = TextLook {
                    font,
                    mono: false,
                    stamp: "",
                };
                let laid = line_galley(ui, &line, look, width, self.spell());
                let (rect, response) =
                    ui.allocate_exact_size(laid.galley.size(), egui::Sense::click());
                ui.painter()
                    .galley(rect.min, laid.galley.clone(), color_ink());
                link_clicks(ui, &response, rect, &laid, actions);
                if response.double_clicked() {
                    edit_channel = true;
                }
                response.on_hover_text(tr("Double-click to edit the topic and modes"));
            }
        });
        if edit_channel {
            self.open_channel_central();
        }
        ui.add_space(3.0);
        ui.horizontal(|ui| {
            ui.label(tr("Find"));
            ui.add(
                field(&mut self.find_text)
                    .id(egui::Id::new("find-field"))
                    .hint_text(tr("Ctrl+F"))
                    .desired_width(180.0),
            );
            if !self.find_text.is_empty() {
                let count = self.find_matches().map_or(0, |found| found.len());
                ui.label(trf("{0} matching lines", &[&count]));
                if ui.small_button(tr("Clear")).clicked() {
                    self.find_text.clear();
                }
            }
        });
    }

    fn nick_list(&mut self, ui: &mut egui::Ui, font: f32, actions: &mut Vec<LineAction>) {
        let nicks = &self.windows[self.active].nicks;
        let session = self
            .sessions
            .iter()
            .find(|session| Some(session.id) == self.windows[self.active].server);
        let ops = nicks
            .iter()
            .filter(|nick| nick.starts_with(['@', '~', '&']))
            .count();
        let voices = nicks
            .iter()
            .filter(|nick| {
                nick.chars()
                    .take_while(|ch| "@+%&~".contains(*ch))
                    .any(|ch| ch == '+' || ch == '%')
            })
            .count();
        ui.label(
            egui::RichText::new(super::plural(nicks.len(), "{0} user", "{0} users"))
                .font(egui::FontId::new(
                    font,
                    egui::FontFamily::Name("bold".into()),
                ))
                .color(color_ink()),
        );
        ui.label(
            egui::RichText::new(trf(
                "{0}, {1} voiced",
                &[&super::plural(ops, "{0} op", "{0} ops"), &voices],
            ))
            .small()
            .color(muted_color()),
        );
        ui.add_space(2.0);
        let dragging = ui.ctx().input(|input| !input.raw.hovered_files.is_empty());
        let nick_popups = self.popups_for("nicklist");
        let mut drop_target = None;
        egui::ScrollArea::vertical()
            .id_salt("nicks")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let mut last_rank = None;
                for nick in nicks {
                    let rank = super::nick_rank(nick);
                    if last_rank.is_some_and(|last| last != rank) {
                        ui.add_space(3.0);
                        let (rect, _) = ui.allocate_exact_size(
                            egui::vec2(ui.available_width(), 1.0),
                            egui::Sense::hover(),
                        );
                        ui.painter()
                            .rect_filled(rect, 0.0, muted_color().gamma_multiply(0.4));
                        ui.add_space(3.0);
                    }
                    last_rank = Some(rank);
                    let bare = bare_nick(nick);
                    // Only the highest status shows; the full set is in the tooltip.
                    let prefixes: String = nick
                        .chars()
                        .take_while(|ch| "@+%&~".contains(*ch))
                        .collect();
                    let prefix: String = if self.settings.nicklist_hide_badges {
                        String::new()
                    } else {
                        prefixes.chars().take(1).collect()
                    };
                    let selected = self
                        .selected_nick
                        .as_deref()
                        .is_some_and(|picked| same_nick(picked, &bare));
                    let width = ui.available_width();
                    let height = font + 7.0;
                    let (rect, response) =
                        ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click());
                    // Rows scrolled out of sight keep their place but skip
                    // the drawing and text layout, which a big channel's
                    // list would otherwise pay for on every frame.
                    if !ui.is_rect_visible(rect) {
                        continue;
                    }
                    if selected {
                        ui.painter()
                            .rect_filled(rect, egui::CornerRadius::same(2), color_hilite());
                    } else if response.hovered() {
                        ui.painter().rect_filled(
                            rect,
                            egui::CornerRadius::same(2),
                            color_hilite().gamma_multiply(0.12),
                        );
                    }
                    let key = super::nick_key(&bare);
                    let away = session.and_then(|session| session.away_nicks.get(&key));
                    let account = session.and_then(|session| session.accounts.get(&key));
                    let name_color = if selected {
                        text_on(color_hilite(), egui::Color32::WHITE)
                    } else if away.is_some() {
                        super::tone_color(self.nick_tone_with(&bare, &prefixes)).gamma_multiply(0.5)
                    } else {
                        super::tone_color(self.nick_tone_with(&bare, &prefixes))
                    };
                    let badge_color = if selected {
                        name_color
                    } else {
                        prefix_color(&prefix)
                    };
                    let font_id = egui::FontId::proportional(font);
                    ui.painter().text(
                        egui::pos2(rect.left() + 4.0, rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        if prefix.is_empty() { " " } else { &prefix },
                        egui::FontId::new(font, egui::FontFamily::Name("bold".into())),
                        badge_color,
                    );
                    let name_left = rect.left() + 4.0 + font * 0.85;
                    let room = rect.right() - name_left - 2.0;
                    let cut = ui
                        .painter()
                        .layout_no_wrap(bare.clone(), font_id.clone(), name_color)
                        .size()
                        .x
                        > room;
                    let name = super::fitted_text(ui.painter(), &bare, font_id, name_color, room);
                    ui.painter().galley(
                        egui::pos2(name_left, rect.center().y - name.size().y / 2.0),
                        name,
                        name_color,
                    );
                    // The whole nick when it is cut short or has several modes.
                    let mut tip = if cut || prefixes.chars().count() > 1 {
                        format!(
                            "{prefixes}{bare} — {}",
                            tr("double-click for a query, right-click for more")
                        )
                    } else {
                        tr("Double-click for a query. Right-click for more.").to_string()
                    };
                    if let Some(account) = account {
                        tip.push_str(&format!("\n{}", trf("Logged in as {0}", &[&account])));
                    }
                    if let Some(reason) = away {
                        tip.push_str(&format!("\n{}", trf("Away: {0}", &[&reason])));
                    }
                    let response = response.on_hover_text(tip);
                    if dragging && response.contains_pointer() {
                        drop_target = Some(bare.clone());
                    }
                    if response.double_clicked() {
                        actions.push(LineAction::NickDouble(bare.clone()));
                    } else if response.clicked() || response.secondary_clicked() {
                        actions.push(LineAction::Select(bare.clone()));
                    }
                    response.context_menu(|ui| {
                        super::chrome::menu_title(ui, &bare);
                        ui.separator();
                        for (index, (label, command)) in NICK_ACTIONS.iter().enumerate() {
                            if matches!(index, 2 | 8) {
                                ui.separator();
                            }
                            if super::chrome::menu_row(ui, tr(label), false, None, true).clicked() {
                                actions.push(LineAction::Nick(command, bare.clone()));
                                ui.close();
                            }
                        }
                        if !nick_popups.is_empty() {
                            ui.separator();
                        }
                        for popup in &nick_popups {
                            if super::chrome::menu_row(ui, &popup.label, false, None, true)
                                .clicked()
                            {
                                actions.push(LineAction::Popup {
                                    body: popup.body.clone(),
                                    nick: bare.clone(),
                                    text: String::new(),
                                });
                                ui.close();
                            }
                        }
                    });
                }
            });
        if drop_target.is_some() {
            self.drop_nick = drop_target;
        }
    }

    /// The transcript of the active window. Only lines inside the visible
    /// part are laid out and painted; the height of each line is measured
    /// once and kept until the width, font, or timestamp format changes.
    fn transcript(&mut self, ui: &mut egui::Ui, font: f32, actions: &mut Vec<LineAction>) {
        let mono_here = self.window_mono(self.active);
        let found = self.find_matches();
        let stamp_format = if self.settings.timestamps {
            self.settings.timestamp_format.clone()
        } else {
            String::new()
        };
        let look = TextLook {
            font,
            mono: mono_here,
            stamp: &stamp_format,
        };
        let shading = self.settings.line_shading;
        let width = (ui.available_width() - 16.0).max(40.0);
        let key = look.key(width);
        let window = &self.windows[self.active];
        let marker = if self.settings.line_marker && found.is_none() {
            window.marker
        } else {
            None
        };
        let shown: Vec<usize> = match found {
            Some(found) => found.to_vec(),
            None => (0..window.lines.len()).collect(),
        };
        let spell = self.spell();
        let menu_word = &self.menu_word;
        let popups = &self.popups_for("line");
        let spacing = 1.0;
        // Exact heights for lines measured with this layout key. Other lines
        // use their last height (or one row) until they are measured: lines
        // on screen are measured as they are painted, and up to `BUDGET`
        // more per frame from the newest back, so a resize stays smooth.
        const BUDGET: usize = 128;
        let row = ui.fonts_mut(|fonts| fonts.row_height(&look.font_id(false, false)));
        let mut budget = BUDGET;
        let mut stale = false;
        let mut heights = vec![0.0; shown.len()];
        for (slot, &index) in shown.iter().enumerate().rev() {
            let line = &window.lines[index];
            let height = match line.measured.get() {
                (stored, height) if stored == key => height,
                _ if budget > 0 => {
                    budget -= 1;
                    let height = line_galley(ui, line, look, width, spell).galley.size().y;
                    line.measured.set((key, height));
                    height
                }
                (stored, height) => {
                    stale = true;
                    if stored == 0 { row } else { height }
                }
            };
            heights[slot] = height + spacing;
        }
        if stale {
            ui.ctx().request_repaint();
        }
        let marker_gap = 6.0;
        let total: f32 =
            heights.iter().sum::<f32>() + if marker.is_some() { marker_gap } else { 0.0 };
        let window_title = window.title.clone();
        egui::ScrollArea::vertical()
            .id_salt(("transcript", self.active, window_title))
            .stick_to_bottom(true)
            .auto_shrink([false, false])
            .show_viewport(ui, |ui, viewport| {
                ui.set_height(total);
                ui.set_width(width);
                let origin = ui.max_rect().min;
                // Empty space below the last line takes double-clicks too.
                let below = egui::Rect::from_min_max(
                    origin + egui::vec2(0.0, total),
                    egui::pos2(origin.x + width, origin.y + viewport.max.y.max(total)),
                );
                if below.height() > 0.0
                    && ui
                        .interact(
                            below,
                            egui::Id::new(("below", self.active)),
                            egui::Sense::click(),
                        )
                        .double_clicked()
                {
                    actions.push(LineAction::WindowDouble);
                }
                let mut top = 0.0;
                for (slot, &index) in shown.iter().enumerate() {
                    if marker == Some(index) {
                        let y = origin.y + top + marker_gap / 2.0;
                        let color = super::tone_color(super::Tone::Attention).gamma_multiply(0.7);
                        ui.painter().hline(
                            origin.x..=origin.x + width,
                            y,
                            egui::Stroke::new(1.5, color),
                        );
                        top += marker_gap;
                    }
                    let height = heights[slot];
                    if top + height >= viewport.min.y && top <= viewport.max.y {
                        let line = &window.lines[index];
                        let laid = line_galley(ui, line, look, width, spell);
                        line.measured.set((key, laid.galley.size().y));
                        let rect = egui::Rect::from_min_size(
                            origin + egui::vec2(0.0, top),
                            egui::vec2(width, laid.galley.size().y + spacing),
                        );
                        if shading && index % 2 == 1 {
                            ui.painter().rect_filled(
                                rect.expand2(egui::vec2(4.0, 0.0)),
                                0.0,
                                super::mix(color_paper(), color_ink(), 0.05),
                            );
                        }
                        let response = ui.interact(
                            rect,
                            egui::Id::new(("line", self.active, index)),
                            egui::Sense::click(),
                        );
                        ui.painter()
                            .galley(rect.min, laid.galley.clone(), color_ink());
                        link_clicks(ui, &response, rect, &laid, actions);
                        if response.double_clicked() {
                            actions.push(LineAction::WindowDouble);
                        }
                        if response.secondary_clicked() {
                            // The word under the pointer, for a web search.
                            let word = response.interact_pointer_pos().and_then(|pointer| {
                                let galley = &laid.galley;
                                let cursor = galley.cursor_from_pos(pointer - rect.min);
                                let text = galley.text();
                                let byte = text
                                    .char_indices()
                                    .nth(cursor.index.0)
                                    .map_or(text.len(), |(byte, _)| byte);
                                crate::spell::word_at(text, byte)
                                    .map(|(from, to)| text[from..to].to_string())
                            });
                            actions.push(LineAction::MenuWord(word));
                        }
                        let nick = line.nick.clone();
                        let plain = line.plain();
                        response.context_menu(|ui| {
                            use super::chrome::{menu_row, menu_title};
                            if let Some(nick) = &nick {
                                menu_title(ui, nick);
                                ui.separator();
                                if menu_row(ui, tr("Query"), false, None, true).clicked() {
                                    actions.push(LineAction::Nick("query", nick.clone()));
                                    ui.close();
                                }
                                if menu_row(ui, tr("Whois"), false, None, true).clicked() {
                                    actions.push(LineAction::Nick("whois", nick.clone()));
                                    ui.close();
                                }
                                ui.separator();
                            }
                            if menu_row(ui, tr("Copy line"), false, None, true).clicked() {
                                actions.push(LineAction::Copy(plain.clone()));
                                ui.close();
                            }
                            if let Some(word) = menu_word
                                && menu_row(
                                    ui,
                                    &trf("Search the web for \u{201c}{0}\u{201d}", &[&short(word)]),
                                    false,
                                    None,
                                    true,
                                )
                                .clicked()
                            {
                                actions.push(LineAction::Search(word.clone()));
                                ui.close();
                            }
                            if !popups.is_empty() {
                                ui.separator();
                            }
                            for popup in popups {
                                if menu_row(ui, &popup.label, false, None, true).clicked() {
                                    actions.push(LineAction::Popup {
                                        body: popup.body.clone(),
                                        nick: nick.clone().unwrap_or_default(),
                                        text: plain.clone(),
                                    });
                                    ui.close();
                                }
                            }
                            ui.separator();
                            if menu_row(ui, tr("Fixed-width font here"), mono_here, None, true)
                                .clicked()
                            {
                                actions.push(LineAction::ToggleMono);
                                ui.close();
                            }
                        });
                    }
                    top += height;
                }
            });
    }

    /// The input line. While it has focus: Up/Down walk the history, Tab
    /// completes nicknames, Ctrl+B/U/I/R/O insert text style codes, Ctrl+K
    /// inserts a color code and opens the palette, and a paste of several
    /// lines asks before sending.
    fn input_line(&mut self, ui: &mut egui::Ui, font: f32) {
        let id = egui::Id::new("input-line");
        let focused = ui.memory(|memory| memory.has_focus(id));
        let multiline = self.settings.multiline_input;
        // In the multi-line input Enter sends and Shift+Enter starts a line.
        let send_now = focused
            && multiline
            && ui.ctx().input_mut(|input| {
                // Take a plain Enter only; `consume_key` would also take
                // Shift+Enter, which must stay a line break.
                let plain = input.events.iter().position(|event| {
                    matches!(
                        event,
                        egui::Event::Key {
                            key: egui::Key::Enter,
                            pressed: true,
                            modifiers,
                            ..
                        } if !modifiers.shift && !modifiers.alt && !modifiers.command
                    )
                });
                plain.map(|at| input.events.remove(at)).is_some()
            });
        if focused {
            self.input_keys(ui.ctx(), id);
        }
        let connected = self.active_connected();
        let hint = if connected {
            match self.windows.get(self.active).map(|window| window.kind) {
                Some(Kind::Channel | Kind::Query) => {
                    tr("Message — Tab completes nicks, Ctrl+K for colors")
                }
                _ => tr("Type a /command"),
            }
        } else {
            tr("Connect before sending (/help lists commands)")
        };
        if let Some(typing) = self.typing_label() {
            ui.label(
                egui::RichText::new(typing)
                    .small()
                    .italics()
                    .color(muted_color()),
            );
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs(1));
        }
        let spell_on = self.settings.spell_input && !self.dictionary.is_empty();
        let spell = super::Spell {
            words: &self.dictionary,
            skip: &self.spell_skip,
        };
        let output = paper()
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let side = font + 4.0;
                    let (rect, palette) =
                        ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::click());
                    if palette.hovered() {
                        ui.painter().rect_filled(
                            rect,
                            egui::CornerRadius::same(3),
                            color_hilite().gamma_multiply(0.15),
                        );
                    }
                    super::icons::paint(
                        ui.painter(),
                        rect.shrink(2.0),
                        super::icons::Icon::Colors,
                        color_ink(),
                    );
                    let palette = palette.on_hover_text(tr("Insert a color (Ctrl+K)"));
                    if palette.clicked() {
                        self.color_picker = !self.color_picker;
                    }
                    let edit = if multiline {
                        super::field_multi(&mut self.input)
                            .desired_rows(1)
                            .return_key(egui::KeyboardShortcut::new(
                                egui::Modifiers::SHIFT,
                                egui::Key::Enter,
                            ))
                    } else {
                        field(&mut self.input)
                    };
                    let edit = edit
                        .id(id)
                        .lock_focus(true)
                        .font(egui::FontId::proportional(font))
                        .desired_width(f32::INFINITY)
                        .hint_text(hint);
                    // Unknown words are underlined once they are finished
                    // (followed by a space or a mark).
                    let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap: f32| {
                        let text = text.as_str();
                        let plain =
                            egui::TextFormat::simple(egui::FontId::proportional(font), color_ink());
                        let mut marked = plain.clone();
                        marked.underline =
                            egui::Stroke::new(1.0, super::tone_color(super::Tone::Attention));
                        let mut job = egui::text::LayoutJob::default();
                        let mut at = 0;
                        for (from, to) in
                            crate::spell::miss_ranges(text, spell.words, spell.skip, true)
                        {
                            job.append(&text[at..from], 0.0, plain.clone());
                            job.append(&text[from..to], 0.0, marked.clone());
                            at = to;
                        }
                        job.append(&text[at..], 0.0, plain);
                        job.wrap.max_width = wrap;
                        ui.fonts_mut(|fonts| fonts.layout_job(job))
                    };
                    if spell_on {
                        edit.layouter(&mut layouter).show(ui)
                    } else {
                        edit.show(ui)
                    }
                })
                .inner
            })
            .inner;
        let galley = output.galley.clone();
        let galley_pos = output.galley_pos;
        let response = output.response.response;
        if spell_on && response.secondary_clicked() {
            self.input_word = response.interact_pointer_pos().and_then(|pointer| {
                let cursor = galley.cursor_from_pos(pointer - galley_pos);
                let byte = self
                    .input
                    .char_indices()
                    .nth(cursor.index.0)
                    .map_or(self.input.len(), |(byte, _)| byte);
                let (from, to) = crate::spell::word_at(&self.input, byte)?;
                let missed = crate::spell::miss_ranges(
                    &self.input,
                    &self.dictionary,
                    &self.spell_skip,
                    false,
                )
                .contains(&(from, to));
                missed.then(|| {
                    let found =
                        crate::spell::suggestions(&self.input[from..to], &self.dictionary, 6);
                    (from, to, found)
                })
            });
        }
        if self.input_word.is_some() {
            let mut replace = None;
            let mut add = None;
            response.context_menu(|ui| {
                use super::chrome::{menu_row, menu_title};
                let Some((from, to, found)) = &self.input_word else {
                    return;
                };
                let word = self.input.get(*from..*to).unwrap_or_default().to_string();
                menu_title(ui, &word);
                ui.separator();
                if found.is_empty() {
                    ui.label(egui::RichText::new(tr("No suggestions")).color(muted_color()));
                }
                for suggestion in found {
                    if menu_row(ui, suggestion, false, None, true).clicked() {
                        replace = Some(suggestion.clone());
                        ui.close();
                    }
                }
                ui.separator();
                if menu_row(ui, tr("Add to the dictionary"), false, None, true).clicked() {
                    add = Some(word);
                    ui.close();
                }
            });
            if let Some((from, to, _)) = self.input_word.clone() {
                if let Some(suggestion) = replace
                    && self.input.is_char_boundary(from)
                    && self.input.is_char_boundary(to)
                    && to <= self.input.len()
                {
                    // Keep a capital first letter if the word had one.
                    let capital = self.input[from..].starts_with(char::is_uppercase);
                    let mut text = suggestion;
                    if capital {
                        let mut chars = text.chars();
                        if let Some(first) = chars.next() {
                            text = first.to_uppercase().chain(chars).collect();
                        }
                    }
                    self.input.replace_range(from..to, &text);
                    self.input_word = None;
                }
                if let Some(word) = add {
                    self.add_to_dictionary(&word);
                    self.input_word = None;
                }
            }
            if !response.context_menu_opened() {
                self.input_word = None;
            }
        }
        if response.changed() {
            self.note_draft();
            let typed = self.input.clone();
            self.want_fonts(&typed);
            self.completion = None;
            self.send_typing();
            self.user_active();
        }
        let entered = if multiline {
            send_now
        } else {
            response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter))
        };
        // Text put in by recall, completion or the color palette starts a
        // draft too.
        self.note_draft();
        if entered {
            self.enter_line();
            response.request_focus();
        }
        if self.color_picker {
            self.color_palette(ui, &response, id);
            // A color code put in now starts a draft at once.
            self.note_draft();
        }
    }

    fn input_keys(&mut self, ctx: &egui::Context, id: egui::Id) {
        use egui::{Key, Modifiers};
        let mut insert = None;
        let (up, down, tab, paste) = ctx.input_mut(|input| {
            for (key, code) in [
                (Key::B, '\u{2}'),
                (Key::U, '\u{1f}'),
                (Key::I, '\u{1d}'),
                (Key::R, '\u{16}'),
                (Key::O, '\u{f}'),
                (Key::K, '\u{3}'),
            ] {
                if input.consume_key(Modifiers::COMMAND, key) {
                    insert = Some(code);
                }
            }
            let paste = input.events.iter().find_map(|event| match event {
                egui::Event::Paste(text) if text.contains('\n') => Some(text.clone()),
                _ => None,
            });
            if paste.is_some() {
                input.events.retain(
                    |event| !matches!(event, egui::Event::Paste(text) if text.contains('\n')),
                );
            }
            let walk = !self.input.contains('\n');
            (
                walk && input.consume_key(Modifiers::NONE, Key::ArrowUp),
                walk && input.consume_key(Modifiers::NONE, Key::ArrowDown),
                input.consume_key(Modifiers::NONE, Key::Tab),
                paste,
            )
        });
        let mut state = egui::text_edit::TextEditState::load(ctx, id).unwrap_or_default();
        let cursor = state
            .cursor
            .char_range()
            .map_or(self.input.chars().count(), |range| range.primary.index.0);
        let place = |state: &mut egui::text_edit::TextEditState, at: usize| {
            state
                .cursor
                .set_char_range(Some(egui::text::CCursorRange::one(
                    egui::text::CCursor::new(at),
                )));
        };
        if let Some(code) = insert {
            let mut chars: Vec<char> = self.input.chars().collect();
            let at = cursor.min(chars.len());
            chars.insert(at, code);
            self.input = chars.into_iter().collect();
            place(&mut state, at + 1);
            if code == '\u{3}' {
                self.color_picker = true;
            }
        }
        if up && let Some(text) = self.history.up(&self.input) {
            let end = text.chars().count();
            self.input = text;
            place(&mut state, end);
        }
        if down && let Some(text) = self.history.down() {
            let end = text.chars().count();
            self.input = text;
            place(&mut state, end);
        }
        if tab {
            let mut names: Vec<String> = self
                .windows
                .get(self.active)
                .map(|window| window.nicks.clone())
                .unwrap_or_default();
            if let Some(window) = self.windows.get(self.active)
                && window.kind == Kind::Query
            {
                names.push(window.title.clone());
            }
            // Every open channel, and commands for a `/` at the start.
            for window in &self.windows {
                if window.kind == Kind::Channel && !names.contains(&window.title) {
                    names.push(window.title.clone());
                }
            }
            if self.input.starts_with('/') {
                names.extend(super::input::COMMANDS.iter().map(|name| format!("/{name}")));
            }
            let current = self.input.clone();
            if let Some((text, at)) =
                super::input::complete(&current, cursor, &names, ": ", &mut self.completion)
            {
                self.input = text;
                place(&mut state, at);
            }
        }
        state.store(ctx, id);
        if let Some(text) = paste {
            let lines: Vec<String> = text
                .lines()
                .map(str::to_string)
                .filter(|line| !line.trim().is_empty())
                .collect();
            let limit = self.settings.paste_confirm as usize;
            if limit > 0 && lines.len() >= limit {
                self.hold_paste(lines, None);
            } else {
                self.run_typed(lines, None);
            }
        }
    }

    fn color_palette(&mut self, ui: &mut egui::Ui, anchor: &egui::Response, id: egui::Id) {
        let mut pick = None;
        let mut close = false;
        egui::Area::new(egui::Id::new("color-palette"))
            .order(egui::Order::Foreground)
            .fixed_pos(anchor.rect.left_top() - egui::vec2(0.0, 74.0))
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(tr("Color"));
                        if ui.small_button("✕").clicked() {
                            close = true;
                        }
                    });
                    egui::Grid::new("palette-grid")
                        .spacing([3.0, 3.0])
                        .show(ui, |ui| {
                            for index in 0u8..16 {
                                let (rect, response) = ui.allocate_exact_size(
                                    egui::vec2(20.0, 18.0),
                                    egui::Sense::click(),
                                );
                                ui.painter().rect(
                                    rect,
                                    egui::CornerRadius::same(2),
                                    palette_color(index),
                                    egui::Stroke::new(1.0, super::color_shadow()),
                                    egui::StrokeKind::Inside,
                                );
                                ui.painter().text(
                                    rect.center(),
                                    egui::Align2::CENTER_CENTER,
                                    index.to_string(),
                                    egui::FontId::proportional(10.0),
                                    text_on(palette_color(index), egui::Color32::BLACK),
                                );
                                if response
                                    .on_hover_text(trf("Color {0}", &[&index]))
                                    .clicked()
                                {
                                    pick = Some(index);
                                }
                                if index == 7 {
                                    ui.end_row();
                                }
                            }
                        });
                });
            });
        if let Some(index) = pick {
            let mut state = egui::text_edit::TextEditState::load(ui.ctx(), id).unwrap_or_default();
            let mut chars: Vec<char> = self.input.chars().collect();
            let mut at = state
                .cursor
                .char_range()
                .map_or(chars.len(), |range| range.primary.index.0)
                .min(chars.len());
            if at == 0 || chars[at - 1] != '\u{3}' {
                chars.insert(at, '\u{3}');
                at += 1;
            }
            for digit in format!("{index:02}").chars() {
                chars.insert(at, digit);
                at += 1;
            }
            self.input = chars.into_iter().collect();
            state
                .cursor
                .set_char_range(Some(egui::text::CCursorRange::one(
                    egui::text::CCursor::new(at),
                )));
            state.store(ui.ctx(), id);
            ui.ctx().memory_mut(|memory| memory.request_focus(id));
            self.color_picker = false;
        }
        if close {
            self.color_picker = false;
        }
    }

    fn paste_dialog(&mut self, ctx: &egui::Context) {
        let Some(lines) = self.paste_pending.as_ref() else {
            return;
        };
        let count = lines.len();
        let preview: String = lines.iter().take(4).cloned().collect::<Vec<_>>().join("\n");
        let mut decision = None;
        egui::Modal::new(egui::Id::new("paste-confirm")).show(ctx, |ui| {
            ui.heading(tr("Send several lines?"));
            ui.label(trf(
                "You pasted {0} lines. Each one is sent as its own message.",
                &[&count],
            ));
            ui.add_space(4.0);
            egui::Frame::new()
                .fill(color_paper())
                .inner_margin(egui::Margin::same(6))
                .show(ui, |ui| {
                    ui.label(egui::RichText::new(&preview).monospace().color(color_ink()));
                    if count > 4 {
                        ui.label(
                            egui::RichText::new(trf("… and {0} more", &[&(count - 4)]))
                                .color(muted_color()),
                        );
                    }
                });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button(trf("Send {0} lines", &[&count])).clicked() {
                    decision = Some(true);
                }
                if ui.button(tr("Cancel")).clicked() {
                    decision = Some(false);
                }
            });
        });
        match decision {
            Some(true) => self.send_paste(),
            Some(false) => {
                self.paste_pending = None;
                self.paste_entry = None;
                self.paste_target = None;
            }
            None => {}
        }
    }
}

/// Color for a nickname's channel status prefix.
fn prefix_color(prefix: &str) -> egui::Color32 {
    let base = match prefix.chars().next() {
        Some('~' | '&') => egui::Color32::from_rgb(170, 60, 200),
        Some('@') => egui::Color32::from_rgb(200, 50, 50),
        Some('%') => egui::Color32::from_rgb(210, 120, 20),
        Some('+') => egui::Color32::from_rgb(30, 140, 60),
        _ => color_ink(),
    };
    super::readable(base, color_paper(), color_ink())
}

/// A word cut to a menu-friendly length.
fn short(word: &str) -> String {
    const MOST: usize = 24;
    if word.chars().count() <= MOST {
        word.to_string()
    } else {
        word.chars().take(MOST - 1).chain(['…']).collect()
    }
}
