//! The monitor panel along the bottom of the window: highlights, private
//! messages, notices, and server messages from every window and server in
//! one place. A double-click on a line goes to the window it came from.

use crate::i18n::{tr, tr_key};
use eframe::egui;

use super::{Kind, OpenIrcApp, RichLine, color_hilite, line_galley, muted_on_face, paper};

/// Lines kept per tab.
const KEEP: usize = 500;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MonitorTab {
    Highlights,
    Private,
    Notices,
    Server,
}

const TABS: [(MonitorTab, &str); 4] = [
    (MonitorTab::Highlights, tr_key("Highlights")),
    (MonitorTab::Private, tr_key("Private messages")),
    (MonitorTab::Notices, tr_key("Notices")),
    (MonitorTab::Server, tr_key("Server messages")),
];

pub(super) struct MonitorLine {
    line: RichLine,
    /// The window to go to: its server, kind, and title.
    from: Option<(Option<u64>, Kind, String)>,
}

pub(super) struct Monitor {
    tab: MonitorTab,
    lines: [Vec<MonitorLine>; 4],
    /// Unread lines per tab since it was last shown.
    unread: [usize; 4],
}

impl Default for Monitor {
    fn default() -> Self {
        Self {
            tab: MonitorTab::Highlights,
            lines: Default::default(),
            unread: [0; 4],
        }
    }
}

fn slot(tab: MonitorTab) -> usize {
    TABS.iter().position(|(item, _)| *item == tab).unwrap_or(0)
}

impl OpenIrcApp {
    /// Add a line to a monitor tab, labelled with where it came from.
    pub(super) fn monitor_push(
        &mut self,
        tab: MonitorTab,
        mut line: RichLine,
        from: Option<(Kind, String)>,
    ) {
        let server = self.scope();
        // With several servers, say which one.
        let mut place = String::new();
        if self.sessions.len() > 1
            && let Some(session) = self
                .sessions
                .iter()
                .find(|session| Some(session.id) == server)
        {
            place.push_str(session.network.as_deref().unwrap_or(&session.host));
            place.push(' ');
        }
        if let Some((Kind::Channel, title)) = &from {
            place.push_str(title);
            place.push(' ');
        }
        if !place.is_empty() {
            line.spans.insert(
                0,
                super::Piece::new(
                    super::Tone::Event(super::Ev::Info),
                    format!("[{}] ", place.trim_end()),
                ),
            );
        }
        let index = slot(tab);
        let list = &mut self.monitor.lines[index];
        list.push(MonitorLine {
            line,
            from: from.map(|(kind, title)| (server, kind, title)),
        });
        if list.len() > KEEP {
            let extra = list.len() - KEEP;
            list.drain(..extra);
        }
        if !(self.settings.monitor_panel && self.monitor.tab == tab) {
            self.monitor.unread[index] += 1;
        }
    }

    pub(super) fn monitor_panel(&mut self, ui: &mut egui::Ui) {
        let mut go = None;
        let mut clear = false;
        egui::Panel::bottom("monitor")
            .resizable(true)
            .default_size(150.0)
            .min_size(70.0)
            .frame(
                egui::Frame::new()
                    .fill(super::color_face())
                    .inner_margin(egui::Margin::symmetric(4, 3)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    for (tab, label) in TABS {
                        let label = tr(label);
                        let index = slot(tab);
                        let unread = self.monitor.unread[index];
                        let text = if unread > 0 && self.monitor.tab != tab {
                            format!("{label} ({unread})")
                        } else {
                            label.to_string()
                        };
                        let mut rich = egui::RichText::new(text);
                        if unread > 0 && self.monitor.tab != tab {
                            rich = rich.strong();
                        }
                        if ui.selectable_label(self.monitor.tab == tab, rich).clicked() {
                            self.monitor.tab = tab;
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button(tr("Clear")).clicked() {
                            clear = true;
                        }
                        ui.label(
                            egui::RichText::new(tr("F2 hides this panel"))
                                .small()
                                .color(muted_on_face()),
                        );
                    });
                });
                let index = slot(self.monitor.tab);
                self.monitor.unread[index] = 0;
                let font = self.view_settings().font_size as f32;
                let stamp = if self.settings.timestamps {
                    self.settings.timestamp_format.clone()
                } else {
                    String::new()
                };
                let look = super::TextLook {
                    font,
                    mono: false,
                    stamp: &stamp,
                };
                let no_words = std::collections::HashSet::new();
                paper().show(ui, |ui| {
                    ui.set_min_size(ui.available_size());
                    egui::ScrollArea::vertical()
                        .id_salt(("monitor", index))
                        .stick_to_bottom(true)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            let lines = &self.monitor.lines[index];
                            if lines.is_empty() {
                                ui.label(
                                    egui::RichText::new(tr("Nothing yet.")).color(muted_on_face()),
                                );
                            }
                            let width = ui.available_width();
                            for (row, item) in lines.iter().enumerate() {
                                let laid = line_galley(
                                    ui,
                                    &item.line,
                                    look,
                                    width,
                                    super::Spell {
                                        words: &no_words,
                                        skip: &no_words,
                                    },
                                );
                                let (rect, response) = ui
                                    .allocate_exact_size(laid.galley.size(), egui::Sense::click());
                                if response.hovered() {
                                    ui.painter().rect_filled(
                                        rect,
                                        0.0,
                                        color_hilite().gamma_multiply(0.12),
                                    );
                                }
                                ui.painter().galley(
                                    rect.min,
                                    laid.galley.clone(),
                                    super::color_ink(),
                                );
                                let response = if item.from.is_some() {
                                    response.on_hover_text(tr("Double-click to go to this window"))
                                } else {
                                    response
                                };
                                if response.double_clicked() {
                                    go = Some(row);
                                }
                            }
                        });
                });
            });
        let index = slot(self.monitor.tab);
        if clear {
            self.monitor.lines[index].clear();
        }
        if let Some(row) = go
            && let Some((server, kind, title)) = self.monitor.lines[index]
                .get(row)
                .and_then(|item| item.from.clone())
            && let map = self.casemap_of(server)
            && let Some(window) = self.windows.iter().position(|window| {
                window.server == server && window.kind == kind && map.same(&window.title, &title)
            })
        {
            self.activate(window);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pump::Event;

    #[test]
    fn highlights_private_messages_and_notices_are_collected() {
        let mut app = super::super::tests::test_app();
        app.apply_event(Event::Chat {
            channel: true,
            target: "#c".into(),
            from: "ann".into(),
            text: "hey me, look".into(),
            action: false,
        });
        app.apply_event(Event::Chat {
            channel: true,
            target: "#c".into(),
            from: "ann".into(),
            text: "nothing for you".into(),
            action: false,
        });
        app.apply_event(Event::Chat {
            channel: false,
            target: "bob".into(),
            from: "bob".into(),
            text: "psst".into(),
            action: false,
        });
        app.apply_event(Event::Notice {
            from: "NickServ".into(),
            text: "you are identified".into(),
        });
        let count = |tab| app.monitor.lines[slot(tab)].len();
        assert_eq!(
            count(MonitorTab::Highlights),
            1,
            "only the line that names me"
        );
        assert_eq!(count(MonitorTab::Private), 1);
        assert_eq!(count(MonitorTab::Notices), 1);
        let highlight = &app.monitor.lines[slot(MonitorTab::Highlights)][0];
        assert!(
            highlight.line.plain().starts_with("[#c] "),
            "{}",
            highlight.line.plain()
        );
        assert_eq!(
            highlight.from.as_ref().map(|from| from.2.as_str()),
            Some("#c")
        );
        assert_eq!(app.monitor.unread[slot(MonitorTab::Private)], 1);
    }

    #[test]
    fn notify_events_are_shown_and_tracked() {
        let mut app = super::super::tests::test_app();
        app.apply_event(Event::Notify {
            nick: "Ann".into(),
            address: "a@host".into(),
            online: true,
        });
        let status = app.status_index().unwrap();
        let last = app.windows[status].lines.last().unwrap().plain();
        assert_eq!(last, "* Ann (a@host) is online");
        app.apply_event(Event::Notify {
            nick: "Ann".into(),
            address: String::new(),
            online: false,
        });
        assert_eq!(
            app.windows[status].lines.last().unwrap().plain(),
            "* Ann has left IRC"
        );
    }
}
