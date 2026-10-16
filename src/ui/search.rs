//! Search every window at once. Results list the window and the line; a
//! double-click goes to the window with the same text in its Find bar.

use crate::i18n::tr;
use eframe::egui;

use super::{OpenIrcApp, dialog_pos, field, format_stamp, muted_on_face, paper};

/// Results shown at most, so a common word does not build a huge list.
const MAX_RESULTS: usize = 2_000;

#[derive(Default)]
pub(super) struct Search {
    pub open: bool,
    text: String,
    /// Window index and line index of each match, from the last search.
    results: Vec<(usize, usize)>,
    /// The windows' generations when the search ran; any change reruns it.
    seen: Vec<u64>,
    searched: String,
}

impl OpenIrcApp {
    fn run_search(&mut self) {
        let needle = self.search.text.trim().to_lowercase();
        self.search.searched = self.search.text.trim().to_string();
        self.search.results.clear();
        self.search.seen = self
            .windows
            .iter()
            .map(|window| window.generation)
            .collect();
        if needle.is_empty() {
            return;
        }
        'windows: for (window_index, window) in self.windows.iter().enumerate() {
            for (line_index, line) in window.lines.iter().enumerate() {
                if line.plain().to_lowercase().contains(&needle) {
                    self.search.results.push((window_index, line_index));
                    if self.search.results.len() >= MAX_RESULTS {
                        break 'windows;
                    }
                }
            }
        }
    }

    pub(super) fn search_window(&mut self, ctx: &egui::Context) {
        if !self.search.open {
            return;
        }
        // Windows opened, closed, or written to since the last search.
        let current: Vec<u64> = self
            .windows
            .iter()
            .map(|window| window.generation)
            .collect();
        if !self.search.searched.is_empty() && current != self.search.seen {
            self.run_search();
        }
        let mut open = true;
        let mut go = None;
        let mut search = false;
        egui::Window::new(tr("Search all windows"))
            .default_pos(dialog_pos(ctx, 3))
            .default_size([640.0, 360.0])
            .collapsible(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let response = ui.add(
                        field(&mut self.search.text)
                            .hint_text(tr("text to find"))
                            .desired_width(300.0),
                    );
                    if response.lost_focus()
                        && ui.input(|input| input.key_pressed(egui::Key::Enter))
                    {
                        search = true;
                    }
                    if ui.button(tr("Search")).clicked() {
                        search = true;
                    }
                    if !self.search.searched.is_empty() {
                        let count = self.search.results.len();
                        let more = if count >= MAX_RESULTS {
                            " (first ones)"
                        } else {
                            ""
                        };
                        ui.label(
                            egui::RichText::new(format!(
                                "{}{more}",
                                super::plural(count, "{0} match", "{0} matches")
                            ))
                            .color(muted_on_face()),
                        );
                    }
                });
                paper().show(ui, |ui| {
                    ui.set_min_size(ui.available_size());
                    let row = ui.text_style_height(&egui::TextStyle::Body) + 4.0;
                    egui::ScrollArea::vertical()
                        .id_salt("search-results")
                        .auto_shrink([false, false])
                        .show_rows(ui, row, self.search.results.len(), |ui, range| {
                            for &(window_index, line_index) in &self.search.results[range] {
                                let Some(window) = self.windows.get(window_index) else {
                                    continue;
                                };
                                let Some(line) = window.lines.get(line_index) else {
                                    continue;
                                };
                                let stamp = line
                                    .time
                                    .map(|time| format_stamp("HH:MM", time))
                                    .unwrap_or_default();
                                let response = ui
                                    .horizontal(|ui| {
                                        ui.add_sized(
                                            [120.0, row],
                                            egui::Label::new(
                                                egui::RichText::new(&window.title).strong(),
                                            )
                                            .truncate(),
                                        );
                                        ui.label(egui::RichText::new(stamp).color(muted_on_face()));
                                        ui.add(egui::Label::new(line.plain()).truncate());
                                    })
                                    .response
                                    .interact(egui::Sense::click());
                                if response.double_clicked() {
                                    go = Some(window_index);
                                }
                            }
                        });
                });
            });
        if search {
            self.run_search();
        }
        if let Some(window) = go {
            self.activate(window);
            self.find_text = self.search.searched.clone();
            self.find_cache = None;
        }
        if !open {
            self.search.open = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::pump::Event;

    #[test]
    fn search_finds_lines_in_every_window() {
        let mut app = super::super::tests::test_app();
        for (target, text) in [
            ("#a", "deploy now"),
            ("#b", "nothing here"),
            ("#c", "the DEPLOY failed"),
        ] {
            app.apply_event(Event::Chat {
                channel: true,
                target: target.into(),
                from: "ann".into(),
                text: text.into(),
                action: false,
            });
        }
        app.search.text = "deploy".into();
        app.run_search();
        let titles: Vec<&str> = app
            .search
            .results
            .iter()
            .map(|(window, _)| app.windows[*window].title.as_str())
            .collect();
        assert_eq!(titles, vec!["#a", "#c"]);
    }
}
