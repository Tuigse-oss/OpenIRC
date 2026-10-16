//! Options pages for highlight, ignore, nickname color, and automatic mode
//! rules. Each page is a list of rules with an editor for the one selected.

use crate::i18n::{tr, trf};
use eframe::egui;

use super::{color_ink, field, muted_on_face, palette_color, paper, section, text_on};
use crate::rules::{AutoMode, HighlightRule, IGNORE_TYPES, IgnoreRule, MatchOn, NickColorRule};
use crate::settings::Settings;

/// Which rule is selected on each rules page.
#[derive(Default)]
pub(super) struct RuleSelection {
    highlight: Option<usize>,
    ignore: Option<usize>,
    nick_color: Option<usize>,
    auto_mode: Option<usize>,
    /// Minutes typed for an ignore's "remove after", per edit.
    ignore_minutes: u32,
}

/// A list of rules with Add, Remove, Up, and Down. Returns the selection.
fn rule_list<T>(
    ui: &mut egui::Ui,
    id: &str,
    items: &mut Vec<T>,
    selected: &mut Option<usize>,
    summary: impl Fn(&T) -> String,
    new: impl Fn() -> T,
) {
    paper().show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        egui::ScrollArea::vertical()
            .id_salt(id)
            .max_height(130.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                if items.is_empty() {
                    ui.label(egui::RichText::new(tr("No rules yet.")).color(super::muted_color()));
                }
                for (index, item) in items.iter().enumerate() {
                    let chosen = *selected == Some(index);
                    let mut label = summary(item);
                    if label.trim().is_empty() {
                        label = "(new rule; fill it in below)".into();
                    }
                    // Readable on the selection color as well as on paper.
                    let ink = if chosen {
                        super::text_on(super::color_hilite(), egui::Color32::WHITE)
                    } else {
                        color_ink()
                    };
                    let text = egui::RichText::new(label).color(ink);
                    if ui.selectable_label(chosen, text).clicked() {
                        *selected = Some(index);
                    }
                }
            });
    });
    ui.horizontal(|ui| {
        if ui.button(tr("Add")).clicked() {
            items.push(new());
            *selected = Some(items.len() - 1);
        }
        let current = selected.filter(|index| *index < items.len());
        if ui
            .add_enabled(current.is_some(), egui::Button::new(tr("Remove")))
            .clicked()
            && let Some(index) = current
        {
            items.remove(index);
            *selected = None;
        }
        if ui
            .add_enabled(
                current.is_some_and(|index| index > 0),
                egui::Button::new(tr("Up")),
            )
            .clicked()
            && let Some(index) = current
        {
            items.swap(index, index - 1);
            *selected = Some(index - 1);
        }
        if ui
            .add_enabled(
                current.is_some_and(|index| index + 1 < items.len()),
                egui::Button::new(tr("Down")),
            )
            .clicked()
            && let Some(index) = current
        {
            items.swap(index, index + 1);
            *selected = Some(index + 1);
        }
    });
    if selected.is_some_and(|index| index >= items.len()) {
        *selected = None;
    }
}

/// Palette swatches; `None` for the default color when `allow_none`.
fn color_choice(ui: &mut egui::Ui, value: &mut Option<u8>, allow_none: bool) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(2.0, 2.0);
        if allow_none {
            let (rect, response) =
                ui.allocate_exact_size(egui::vec2(52.0, 18.0), egui::Sense::click());
            let selected = value.is_none();
            ui.painter().rect(
                rect,
                egui::CornerRadius::same(2),
                super::color_paper(),
                egui::Stroke::new(
                    if selected { 2.0 } else { 1.0 },
                    if selected {
                        super::color_hilite()
                    } else {
                        super::color_shadow()
                    },
                ),
                egui::StrokeKind::Inside,
            );
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "default",
                egui::FontId::proportional(10.0),
                color_ink(),
            );
            if response.clicked() {
                *value = None;
            }
        }
        for index in 0u8..16 {
            let (rect, response) =
                ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::click());
            let selected = *value == Some(index);
            ui.painter().rect(
                rect,
                egui::CornerRadius::same(2),
                palette_color(index),
                egui::Stroke::new(
                    if selected { 2.0 } else { 1.0 },
                    if selected {
                        super::color_hilite()
                    } else {
                        super::color_shadow()
                    },
                ),
                egui::StrokeKind::Inside,
            );
            if selected {
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "✓",
                    egui::FontId::proportional(11.0),
                    text_on(palette_color(index), egui::Color32::BLACK),
                );
            }
            if response
                .on_hover_text(trf("Color {0}", &[&index]))
                .clicked()
            {
                *value = Some(index);
            }
        }
    });
}

pub(super) fn highlight_page(
    ui: &mut egui::Ui,
    settings: &mut Settings,
    picks: &mut RuleSelection,
) {
    ui.checkbox(
        &mut settings.highlight_nick,
        tr("Highlight lines that mention my nick"),
    );
    section(ui, tr("Highlight rules"));
    rule_list(
        ui,
        "highlight-rules",
        &mut settings.highlights,
        &mut picks.highlight,
        |rule| {
            let mut text = if rule.regex {
                format!("/{}/", rule.words)
            } else {
                rule.words.clone()
            };
            if !rule.sources.trim().is_empty() {
                text.push_str(&trf("  in {0}", &[&rule.sources]));
            }
            text
        },
        || HighlightRule::words(""),
    );
    let Some(rule) = picks
        .highlight
        .and_then(|index| settings.highlights.get_mut(index))
    else {
        ui.label(egui::RichText::new(tr("Pick a rule to edit it.")).color(muted_on_face()));
        return;
    };
    super::form_grid("highlight-editor")
        .num_columns(2)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            ui.label(if rule.regex {
                tr("Regular expression")
            } else {
                "Words"
            });
            ui.add(
                field(&mut rule.words)
                    .hint_text("rust, good night, $me")
                    .desired_width(300.0),
            );
            ui.end_row();
            ui.label(tr("Only in"));
            ui.add(
                field(&mut rule.sources)
                    .hint_text(tr("channels or nicks, blank for all"))
                    .desired_width(300.0),
            );
            ui.end_row();
            ui.label(tr("Look at"));
            ui.horizontal(|ui| {
                for on in MatchOn::ALL {
                    ui.radio_value(&mut rule.on, on, on.label());
                }
            });
            ui.end_row();
            ui.label("");
            ui.horizontal(|ui| {
                ui.checkbox(&mut rule.case, tr("Match case"));
                ui.checkbox(&mut rule.regex, tr("Regular expression"));
            });
            ui.end_row();
            ui.label(tr("Color"));
            color_choice(ui, &mut rule.color, true);
            ui.end_row();
            ui.label(tr("Sound"));
            ui.add(
                field(&mut rule.sound)
                    .hint_text(tr("WAV file, blank for the highlight sound"))
                    .desired_width(300.0),
            );
            ui.end_row();
            ui.label("");
            ui.horizontal(|ui| {
                ui.checkbox(&mut rule.flash, tr("Flash the taskbar"));
                ui.checkbox(&mut rule.notice, tr("Show a notice"));
            });
            ui.end_row();
        });
    if rule.regex
        && let Some(err) = crate::rules::regex_error(&rule.words)
    {
        ui.colored_label(super::tone_color(super::Tone::Attention), err);
    }
}

pub(super) fn ignore_page(ui: &mut egui::Ui, settings: &mut Settings, picks: &mut RuleSelection) {
    section(ui, tr("Ignore rules"));
    ui.label(
        egui::RichText::new(tr("Masks are nick!user@host with * and ?; a bare nick covers all of that nick's addresses."))
            .small(),
    );
    let now = crate::rules::unix_now();
    rule_list(
        ui,
        "ignore-rules",
        &mut settings.ignores,
        &mut picks.ignore,
        |rule| {
            let mut text = format!("{}  [{}]", rule.full_mask(), rule.types);
            if !rule.network.trim().is_empty() {
                text.push_str(&trf(" on {0}", &[&rule.network]));
            }
            if rule.expires > 0 {
                let left = rule.expires.saturating_sub(now);
                text.push_str(&if left == 0 {
                    " (expired)".to_string()
                } else {
                    trf(" ({0} min left)", &[&left.div_ceil(60)])
                });
            }
            text
        },
        || IgnoreRule::all("nick"),
    );
    let Some(rule) = picks
        .ignore
        .and_then(|index| settings.ignores.get_mut(index))
    else {
        ui.label(egui::RichText::new(tr("Pick a rule to edit it.")).color(muted_on_face()));
        return;
    };
    super::form_grid("ignore-editor")
        .num_columns(2)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            ui.label(tr("Mask"));
            ui.add(
                field(&mut rule.mask)
                    .hint_text(tr("nick or nick!user@host"))
                    .desired_width(260.0),
            );
            ui.end_row();
            ui.label(tr("Ignore"));
            ui.vertical(|ui| {
                for (letter, label) in IGNORE_TYPES {
                    let mut on = rule.types.contains(letter);
                    if ui.checkbox(&mut on, label).changed() {
                        if on {
                            rule.types.push(letter);
                        } else {
                            rule.types.retain(|ch| ch != letter);
                        }
                    }
                }
            });
            ui.end_row();
            ui.label(tr("Network"));
            ui.add(
                field(&mut rule.network)
                    .hint_text(tr("blank for every network"))
                    .desired_width(200.0),
            );
            ui.end_row();
            ui.label(tr("Remove after"));
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut picks.ignore_minutes).range(0..=100_000));
                ui.label(tr("minutes"));
                if ui.button(tr("Set")).clicked() {
                    rule.expires = if picks.ignore_minutes == 0 {
                        0
                    } else {
                        now + u64::from(picks.ignore_minutes) * 60
                    };
                }
                ui.label(egui::RichText::new(tr("0 keeps it")).small());
            });
            ui.end_row();
        });
}

pub(super) fn nick_color_page(
    ui: &mut egui::Ui,
    settings: &mut Settings,
    picks: &mut RuleSelection,
) {
    ui.checkbox(
        &mut settings.auto_nick_colors,
        tr("Give each nick its own color when no rule matches"),
    );
    section(ui, tr("Nickname color rules"));
    ui.label(egui::RichText::new(tr("The first matching rule wins.")).small());
    rule_list(
        ui,
        "nick-color-rules",
        &mut settings.nick_colors,
        &mut picks.nick_color,
        |rule| {
            let who = if rule.mask.trim().is_empty() {
                "anyone".to_string()
            } else {
                rule.mask.clone()
            };
            let modes = match rule.modes.trim() {
                "" => String::new(),
                "*" => " with any status".into(),
                "-" => " without status".into(),
                modes => trf(" with {0}", &[&modes]),
            };
            format!("{who}{modes} → {}", trf("color {0}", &[&rule.color]))
        },
        || NickColorRule {
            mask: String::new(),
            modes: "@".into(),
            color: 4,
        },
    );
    let Some(rule) = picks
        .nick_color
        .and_then(|index| settings.nick_colors.get_mut(index))
    else {
        return;
    };
    super::form_grid("nick-color-editor")
        .num_columns(2)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            ui.label(tr("Nick"));
            ui.add(
                field(&mut rule.mask)
                    .hint_text(tr("blank for anyone; * and ? work"))
                    .desired_width(220.0),
            );
            ui.end_row();
            ui.label(tr("Status"));
            ui.add(
                field(&mut rule.modes)
                    .hint_text(tr("@ for ops, + voice, * any, - none"))
                    .desired_width(220.0),
            );
            ui.end_row();
            ui.label(tr("Color"));
            let mut color = Some(rule.color);
            color_choice(ui, &mut color, false);
            if let Some(color) = color {
                rule.color = color;
            }
            ui.end_row();
        });
}

pub(super) fn auto_mode_page(
    ui: &mut egui::Ui,
    settings: &mut Settings,
    picks: &mut RuleSelection,
) {
    section(ui, tr("Automatic op, halfop, and voice"));
    ui.label(
        egui::RichText::new(tr(
            "Applied when someone matching joins a channel where you are an operator.",
        ))
        .small(),
    );
    rule_list(
        ui,
        "auto-mode-rules",
        &mut settings.auto_modes,
        &mut picks.auto_mode,
        |rule| {
            let mode = match rule.mode {
                'o' => "op",
                'h' => "halfop",
                _ => "voice",
            };
            let channels = if rule.channels.trim().is_empty() {
                "all channels"
            } else {
                rule.channels.as_str()
            };
            format!(
                "{} {} in {channels}{}",
                mode,
                crate::rules::full_mask(&rule.mask),
                if rule.protect { ", protected" } else { "" }
            )
        },
        || AutoMode {
            mask: "nick".into(),
            channels: String::new(),
            network: String::new(),
            mode: 'v',
            protect: false,
        },
    );
    if let Some(rule) = picks
        .auto_mode
        .and_then(|index| settings.auto_modes.get_mut(index))
    {
        super::form_grid("auto-mode-editor")
            .num_columns(2)
            .spacing([8.0, 4.0])
            .show(ui, |ui| {
                ui.label(tr("Mask"));
                ui.add(
                    field(&mut rule.mask)
                        .hint_text(tr("nick or nick!user@host"))
                        .desired_width(240.0),
                );
                ui.end_row();
                ui.label(tr("Channels"));
                ui.add(
                    field(&mut rule.channels)
                        .hint_text(tr("blank for all"))
                        .desired_width(240.0),
                );
                ui.end_row();
                ui.label(tr("Network"));
                ui.add(
                    field(&mut rule.network)
                        .hint_text(tr("blank for all"))
                        .desired_width(200.0),
                );
                ui.end_row();
                ui.label(tr("Give"));
                ui.horizontal(|ui| {
                    ui.radio_value(&mut rule.mode, 'o', tr("op"));
                    ui.radio_value(&mut rule.mode, 'h', tr("halfop"));
                    ui.radio_value(&mut rule.mode, 'v', tr("voice"));
                });
                ui.end_row();
                ui.label("");
                ui.checkbox(
                    &mut rule.protect,
                    tr("Give it back if someone takes it away"),
                );
                ui.end_row();
            });
    }
    ui.checkbox(
        &mut settings.auto_mode_delay,
        tr("Wait 2 to 6 seconds first, in case another operator does it"),
    );
}
