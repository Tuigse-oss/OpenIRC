//! Short notices in the corner of the window for things worth seeing while
//! another window is active: highlights, private messages, invites, and
//! file offers. Clicking one opens the window it is about.

use std::time::{Duration, Instant};

use eframe::egui;

use super::{Kind, OpenIrcApp, color_hilite, color_ink, color_paper, muted_color};

const SHOW_FOR: Duration = Duration::from_secs(7);
const MAX: usize = 4;

pub(super) struct Toast {
    title: String,
    body: String,
    /// The window to open on click: kind, title, and server.
    target: Option<(Kind, String, Option<u64>)>,
    shown: Instant,
}

impl OpenIrcApp {
    /// Queue a notice. `target` is the window index it is about, if any.
    pub(super) fn tip(
        &mut self,
        title: impl Into<String>,
        body: impl Into<String>,
        target: Option<usize>,
    ) {
        if !self.settings.tips {
            return;
        }
        let target = target
            .and_then(|index| self.windows.get(index))
            .map(|window| (window.kind, window.title.clone(), window.server));
        self.toasts.push(Toast {
            title: title.into(),
            body: crate::colors::strip(&body.into()),
            target,
            shown: Instant::now(),
        });
        if self.toasts.len() > MAX {
            self.toasts.remove(0);
        }
    }

    pub(super) fn toasts(&mut self, ctx: &egui::Context) {
        self.toasts.retain(|toast| toast.shown.elapsed() < SHOW_FOR);
        if self.toasts.is_empty() {
            return;
        }
        let screen = ctx.content_rect();
        let mut open = None;
        let mut dismiss = None;
        let mut bottom = screen.bottom() - 36.0;
        for (index, toast) in self.toasts.iter().enumerate().rev() {
            let age = toast.shown.elapsed().as_secs_f32();
            let fade = ((SHOW_FOR.as_secs_f32() - age) / 0.6).clamp(0.0, 1.0);
            let area = egui::Area::new(egui::Id::new(("toast", index, toast.shown)))
                .order(egui::Order::Foreground)
                .pivot(egui::Align2::RIGHT_BOTTOM)
                .fixed_pos(egui::pos2(screen.right() - 14.0, bottom))
                .sense(egui::Sense::click())
                .show(ctx, |ui| {
                    ui.set_opacity(fade);
                    egui::Frame::new()
                        .fill(color_paper())
                        .stroke(egui::Stroke::new(1.0, super::color_shadow()))
                        .corner_radius(egui::CornerRadius::same(6))
                        .shadow(egui::Shadow {
                            offset: [0, 3],
                            blur: 10,
                            spread: 0,
                            color: egui::Color32::from_black_alpha(60),
                        })
                        .inner_margin(egui::Margin::symmetric(12, 8))
                        .show(ui, |ui| {
                            ui.set_max_width(320.0);
                            ui.horizontal(|ui| {
                                let (bar, _) = ui.allocate_exact_size(
                                    egui::vec2(3.0, 34.0),
                                    egui::Sense::hover(),
                                );
                                ui.painter().rect_filled(
                                    bar,
                                    egui::CornerRadius::same(1),
                                    color_hilite(),
                                );
                                ui.vertical(|ui| {
                                    ui.label(
                                        egui::RichText::new(&toast.title)
                                            .strong()
                                            .color(color_ink()),
                                    );
                                    ui.label(egui::RichText::new(&toast.body).color(muted_color()));
                                });
                            });
                        })
                        .response
                });
            let response = area.response;
            if response.clicked() {
                open = Some(index);
            }
            if response.secondary_clicked() {
                dismiss = Some(index);
            }
            bottom = response.rect.top() - 8.0;
        }
        if let Some(index) = open {
            let toast = self.toasts.remove(index);
            if let Some((kind, title, server)) = toast.target
                && let map = self.casemap_of(server)
                && let Some(window) = self.windows.iter().position(|window| {
                    window.kind == kind
                        && window.server == server
                        && map.same(&window.title, &title)
                })
            {
                self.activate(window);
            }
        } else if let Some(index) = dismiss {
            self.toasts.remove(index);
        }
        ctx.request_repaint_after(Duration::from_millis(250));
    }
}
