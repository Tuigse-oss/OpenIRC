//! OpenIRC's own icons for the toolbar, switchbar, and tree, drawn with the
//! egui painter: filled shapes with a dark outline, a light gloss toward the
//! top left, and a gray look when the button is disabled. They scale to any
//! size and need no image files.

use eframe::egui::{self, Color32, Pos2, Rect, Shape, Stroke, Vec2, pos2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Connect,
    Disconnect,
    Join,
    Part,
    ChannelList,
    Options,
    Scripts,
    Address,
    Dcc,
    Favorites,
    Status,
    Channel,
    Query,
    List,
    Chat,
    Colors,
    Notify,
    Urls,
    Logs,
    Transfers,
    Help,
}

/// Paints on a 16 by 16 grid inside a rectangle.
struct Pen<'a> {
    painter: &'a egui::Painter,
    origin: Pos2,
    unit: f32,
    gray: bool,
}

/// Colors used across the set.
const OUTLINE: (u8, u8, u8) = (38, 38, 46);
const GOLD: (u8, u8, u8) = (247, 192, 36);
const BLUE: (u8, u8, u8) = (52, 116, 214);
const GREEN: (u8, u8, u8) = (46, 168, 72);
const RED: (u8, u8, u8) = (214, 52, 44);
const PAPER: (u8, u8, u8) = (250, 250, 244);
const STEEL: (u8, u8, u8) = (150, 162, 178);
const BROWN: (u8, u8, u8) = (160, 98, 46);
const SKIN: (u8, u8, u8) = (244, 196, 150);

/// egui fills a convex polygon only when its points run clockwise on the
/// screen; turn them around when they do not.
fn clockwise(mut points: Vec<Pos2>) -> Vec<Pos2> {
    let area: f32 = points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .map(|(a, b)| a.x * b.y - b.x * a.y)
        .sum();
    if area < 0.0 {
        points.reverse();
    }
    points
}

fn lighten(color: Color32, amount: f32) -> Color32 {
    let up = |value: u8| (value as f32 + (255.0 - value as f32) * amount) as u8;
    Color32::from_rgba_unmultiplied(up(color.r()), up(color.g()), up(color.b()), color.a())
}

fn darken(color: Color32, amount: f32) -> Color32 {
    let down = |value: u8| (value as f32 * (1.0 - amount)) as u8;
    Color32::from_rgba_unmultiplied(down(color.r()), down(color.g()), down(color.b()), color.a())
}

impl Pen<'_> {
    fn at(&self, x: f32, y: f32) -> Pos2 {
        self.origin + Vec2::new(x * self.unit, y * self.unit)
    }

    fn color(&self, (r, g, b): (u8, u8, u8)) -> Color32 {
        if self.gray {
            let luma = (0.3 * r as f32 + 0.59 * g as f32 + 0.11 * b as f32) as u8;
            let soft = lighten(Color32::from_gray(luma), 0.35);
            Color32::from_rgba_unmultiplied(soft.r(), soft.g(), soft.b(), 150)
        } else {
            Color32::from_rgb(r, g, b)
        }
    }

    fn outline(&self) -> Stroke {
        Stroke::new((0.85 * self.unit).max(1.0), self.color(OUTLINE))
    }

    fn points(&self, points: &[(f32, f32)]) -> Vec<Pos2> {
        points.iter().map(|&(x, y)| self.at(x, y)).collect()
    }

    /// A convex shape, filled, with a gloss and no outline.
    fn fill(&self, points: &[(f32, f32)], rgb: (u8, u8, u8)) {
        let fill = self.color(rgb);
        let points = clockwise(self.points(points));
        self.painter
            .add(Shape::convex_polygon(points.clone(), fill, Stroke::NONE));
        self.gloss(&points, fill);
    }

    /// A convex shape, filled and outlined.
    fn shape(&self, points: &[(f32, f32)], rgb: (u8, u8, u8)) {
        self.fill(points, rgb);
        self.painter
            .add(Shape::closed_line(self.points(points), self.outline()));
    }

    /// A lighter, smaller copy of the shape toward the top left.
    fn gloss(&self, points: &[Pos2], fill: Color32) {
        if points.len() < 3 {
            return;
        }
        let center = points
            .iter()
            .fold(Vec2::ZERO, |sum, point| sum + point.to_vec2())
            / points.len() as f32;
        let shift = Vec2::new(-0.5, -0.7) * self.unit;
        let inner: Vec<Pos2> = points
            .iter()
            .map(|point| (center + (point.to_vec2() - center) * 0.5 + shift).to_pos2())
            .collect();
        let shine = lighten(fill, 0.45);
        let shine = Color32::from_rgba_unmultiplied(shine.r(), shine.g(), shine.b(), 150);
        self.painter
            .add(Shape::convex_polygon(inner, shine, Stroke::NONE));
    }

    fn disc(&self, x: f32, y: f32, radius: f32, rgb: (u8, u8, u8)) {
        let fill = self.color(rgb);
        let center = self.at(x, y);
        let radius = radius * self.unit;
        self.painter.circle_filled(center, radius, fill);
        // A darker lower right and a gloss on the upper left give it depth.
        self.painter.circle_filled(
            center + Vec2::splat(radius * 0.18),
            radius * 0.82,
            darken(fill, 0.12),
        );
        self.painter
            .circle_filled(center - Vec2::splat(radius * 0.12), radius * 0.72, fill);
        let shine = lighten(fill, 0.55);
        self.painter.circle_filled(
            center - Vec2::splat(radius * 0.38),
            radius * 0.32,
            Color32::from_rgba_unmultiplied(shine.r(), shine.g(), shine.b(), 170),
        );
        self.painter.circle_stroke(center, radius, self.outline());
    }

    /// A rounded box with a lighter top band.
    fn bevel_box(&self, x0: f32, y0: f32, x1: f32, y1: f32, rgb: (u8, u8, u8)) {
        let fill = self.color(rgb);
        let rect = Rect::from_min_max(self.at(x0, y0), self.at(x1, y1));
        let round = egui::CornerRadius::same((1.5 * self.unit) as u8);
        self.painter.rect_filled(rect, round, fill);
        let lower = Rect::from_min_max(pos2(rect.left(), rect.center().y), rect.right_bottom());
        self.painter.rect_filled(lower, round, darken(fill, 0.1));
        let shine = Rect::from_min_max(
            rect.left_top() + Vec2::splat(self.unit),
            pos2(rect.right() - self.unit, rect.top() + rect.height() * 0.35),
        );
        let light = lighten(fill, 0.4);
        self.painter.rect_filled(
            shine,
            round,
            Color32::from_rgba_unmultiplied(light.r(), light.g(), light.b(), 160),
        );
        self.painter
            .rect_stroke(rect, round, self.outline(), egui::StrokeKind::Middle);
    }

    fn line(&self, a: (f32, f32), b: (f32, f32), width: f32, rgb: (u8, u8, u8)) {
        self.painter.line_segment(
            [self.at(a.0, a.1), self.at(b.0, b.1)],
            Stroke::new(width * self.unit, self.color(rgb)),
        );
    }

    /// A `#` in light strokes over a darker shadow, inside a box.
    fn hash(&self, left: f32, top: f32, size: f32, rgb: (u8, u8, u8)) {
        let s = size / 10.0;
        let strokes = [
            ((3.5, 0.0), (2.5, 10.0)),
            ((7.5, 0.0), (6.5, 10.0)),
            ((0.5, 3.3), (9.5, 3.3)),
            ((0.0, 6.7), (9.0, 6.7)),
        ];
        for ((ax, ay), (bx, by)) in strokes {
            self.line(
                (left + ax * s, top + ay * s),
                (left + bx * s, top + by * s),
                1.6,
                rgb,
            );
        }
    }

    /// A short arrow from `a` to `b` with a filled head.
    fn arrow(&self, a: (f32, f32), b: (f32, f32), rgb: (u8, u8, u8)) {
        let dir = Vec2::new(b.0 - a.0, b.1 - a.1).normalized();
        let side = Vec2::new(-dir.y, dir.x);
        let head = 3.2;
        let base = (b.0 - dir.x * head, b.1 - dir.y * head);
        self.line(a, base, 2.6, OUTLINE);
        self.line(a, base, 1.4, rgb);
        self.shape(
            &[
                b,
                (base.0 + side.x * 2.4, base.1 + side.y * 2.4),
                (base.0 - side.x * 2.4, base.1 - side.y * 2.4),
            ],
            rgb,
        );
    }

    fn bolt(&self, rgb: (u8, u8, u8)) {
        // Two convex halves, then one outline around the whole bolt.
        self.fill(&[(9.5, 0.8), (3.2, 9.0), (7.4, 9.0), (8.6, 6.8)], rgb);
        self.fill(&[(7.4, 9.0), (6.4, 15.2), (12.8, 6.8), (8.6, 6.8)], rgb);
        let outline = self.points(&[
            (9.5, 0.8),
            (3.2, 9.0),
            (7.4, 9.0),
            (6.4, 15.2),
            (12.8, 6.8),
            (8.6, 6.8),
        ]);
        self.painter
            .add(Shape::closed_line(outline, self.outline()));
    }

    fn person(&self, x: f32, body: (u8, u8, u8)) {
        // Shoulders as a wide rounded box, then the head over them.
        self.bevel_box(x - 5.0, 9.5, x + 5.0, 15.2, body);
        self.disc(x, 5.5, 3.6, SKIN);
    }
}

/// Paint a full-color `icon` inside `rect`; `enabled` false grays it.
pub fn paint_color(painter: &egui::Painter, rect: Rect, icon: Icon, enabled: bool) {
    let side = rect.width().min(rect.height());
    let pen = Pen {
        painter,
        origin: rect.center() - Vec2::splat(side / 2.0),
        unit: side / 16.0,
        gray: !enabled,
    };
    match icon {
        Icon::Connect => pen.bolt(GOLD),
        Icon::Disconnect => {
            pen.bolt(STEEL);
            pen.disc(11.5, 11.5, 3.8, RED);
            pen.line((9.6, 13.4), (13.4, 9.6), 1.4, PAPER);
        }
        Icon::Options => {
            let center = (8.0, 8.0);
            for step in 0..8 {
                let angle = step as f32 * std::f32::consts::TAU / 8.0;
                let dir = Vec2::angled(angle);
                let side = Vec2::new(-dir.y, dir.x);
                let point = |out: f32, across: f32| {
                    (
                        center.0 + dir.x * out + side.x * across,
                        center.1 + dir.y * out + side.y * across,
                    )
                };
                pen.shape(
                    &[
                        point(4.5, -1.3),
                        point(7.4, -1.0),
                        point(7.4, 1.0),
                        point(4.5, 1.3),
                    ],
                    STEEL,
                );
            }
            pen.disc(8.0, 8.0, 5.4, STEEL);
            pen.disc(8.0, 8.0, 2.0, (90, 100, 116));
        }
        Icon::Favorites => {
            let center = (8.0, 8.6);
            let at = |step: usize, radius: f32| {
                let angle =
                    -std::f32::consts::FRAC_PI_2 + step as f32 * std::f32::consts::TAU / 10.0;
                (
                    center.0 + angle.cos() * radius,
                    center.1 + angle.sin() * radius,
                )
            };
            // Five points around a pentagon, each a convex triangle.
            let inner: Vec<(f32, f32)> = (0..5).map(|n| at(n * 2 + 1, 3.0)).collect();
            pen.fill(&inner, GOLD);
            for n in 0..5 {
                pen.fill(
                    &[
                        at(n * 2, 7.4),
                        at(n * 2 + 1, 3.0),
                        at((n * 2 + 9) % 10, 3.0),
                    ],
                    GOLD,
                );
            }
            let outline: Vec<(f32, f32)> = (0..10)
                .map(|step| at(step, if step % 2 == 0 { 7.4 } else { 3.0 }))
                .collect();
            pen.painter
                .add(Shape::closed_line(pen.points(&outline), pen.outline()));
        }
        Icon::Join => {
            pen.bevel_box(1.0, 1.5, 12.0, 12.5, BLUE);
            pen.hash(2.6, 2.8, 8.0, PAPER);
            pen.disc(12.0, 12.0, 3.6, GREEN);
            pen.line((10.0, 12.0), (14.0, 12.0), 1.3, PAPER);
            pen.line((12.0, 10.0), (12.0, 14.0), 1.3, PAPER);
        }
        Icon::Part => {
            pen.bevel_box(2.0, 1.0, 9.5, 15.0, BROWN);
            pen.disc(7.8, 8.5, 0.8, GOLD);
            pen.arrow((7.0, 8.0), (15.5, 8.0), RED);
        }
        Icon::ChannelList | Icon::List => {
            pen.bevel_box(2.0, 1.0, 14.0, 15.0, PAPER);
            for y in [4.5, 8.0, 11.5] {
                pen.disc(4.6, y, 1.1, BLUE);
                pen.line((6.8, y), (12.2, y), 1.3, (110, 120, 136));
            }
        }
        Icon::Channel => {
            pen.bevel_box(1.0, 1.0, 15.0, 15.0, BLUE);
            pen.hash(3.0, 3.0, 10.0, PAPER);
        }
        Icon::Scripts => {
            pen.bevel_box(2.0, 1.0, 14.0, 15.0, (252, 238, 180));
            pen.line((6.2, 3.6), (4.6, 5.6), 1.3, GREEN);
            pen.line((4.6, 5.6), (4.6, 7.4), 1.3, GREEN);
            pen.line((4.6, 7.4), (3.6, 8.0), 1.3, GREEN);
            pen.line((3.6, 8.0), (4.6, 8.6), 1.3, GREEN);
            pen.line((4.6, 8.6), (4.6, 10.4), 1.3, GREEN);
            pen.line((4.6, 10.4), (6.2, 12.4), 1.3, GREEN);
            pen.line((9.8, 3.6), (11.4, 5.6), 1.3, GREEN);
            pen.line((11.4, 5.6), (11.4, 7.4), 1.3, GREEN);
            pen.line((11.4, 7.4), (12.4, 8.0), 1.3, GREEN);
            pen.line((12.4, 8.0), (11.4, 8.6), 1.3, GREEN);
            pen.line((11.4, 8.6), (11.4, 10.4), 1.3, GREEN);
            pen.line((11.4, 10.4), (9.8, 12.4), 1.3, GREEN);
        }
        Icon::Address => {
            pen.bevel_box(1.0, 1.0, 15.0, 15.0, RED);
            pen.bevel_box(1.0, 1.0, 3.6, 15.0, (150, 30, 26));
            pen.disc(9.0, 6.0, 2.4, SKIN);
            pen.bevel_box(5.5, 9.0, 12.5, 12.5, PAPER);
        }
        Icon::Query => pen.person(8.0, BLUE),
        Icon::Dcc => {
            pen.bevel_box(1.5, 1.0, 10.5, 14.0, PAPER);
            for y in [4.0, 6.5, 9.0] {
                pen.line((3.5, y), (8.5, y), 1.0, (130, 140, 156));
            }
            pen.arrow((7.0, 11.5), (15.5, 11.5), GREEN);
        }
        Icon::Transfers => {
            pen.arrow((5.0, 14.5), (5.0, 1.5), GREEN);
            pen.arrow((11.0, 1.5), (11.0, 14.5), BLUE);
        }
        Icon::Chat => {
            pen.bevel_box(1.0, 1.5, 11.0, 9.0, BLUE);
            pen.bevel_box(5.0, 6.5, 15.0, 13.0, GREEN);
            pen.shape(&[(7.0, 12.8), (10.0, 12.8), (7.0, 15.3)], GREEN);
        }
        Icon::Status | Icon::Urls => {
            pen.disc(8.0, 8.0, 6.8, BLUE);
            pen.fill(
                &[(4.0, 4.5), (7.5, 3.5), (8.0, 7.0), (5.0, 9.5), (3.0, 7.5)],
                GREEN,
            );
            pen.fill(&[(9.5, 8.5), (12.5, 8.0), (11.5, 12.5), (9.0, 12.0)], GREEN);
            pen.painter.add(egui::epaint::EllipseShape::stroke(
                pen.at(8.0, 8.0),
                Vec2::new(3.0 * pen.unit, 6.8 * pen.unit),
                Stroke::new(0.6 * pen.unit, lighten(pen.color(BLUE), 0.5)),
            ));
            pen.painter
                .circle_stroke(pen.at(8.0, 8.0), 6.8 * pen.unit, pen.outline());
            if icon == Icon::Urls {
                pen.bevel_box(9.0, 10.5, 15.5, 15.5, GOLD);
                pen.line((10.6, 13.0), (13.9, 13.0), 1.2, OUTLINE);
            }
        }
        Icon::Colors => {
            pen.disc(5.5, 5.5, 3.8, RED);
            pen.disc(10.5, 5.5, 3.8, GREEN);
            pen.disc(8.0, 10.2, 3.8, BLUE);
        }
        Icon::Notify => {
            pen.shape(
                &[
                    (8.0, 1.5),
                    (11.6, 4.2),
                    (12.6, 11.0),
                    (3.4, 11.0),
                    (4.4, 4.2),
                ],
                GOLD,
            );
            pen.bevel_box(2.0, 10.5, 14.0, 12.8, GOLD);
            pen.disc(8.0, 14.0, 1.6, BROWN);
        }
        Icon::Logs => {
            pen.bevel_box(3.0, 1.0, 14.5, 15.0, (60, 140, 120));
            pen.bevel_box(5.5, 3.0, 12.5, 7.0, PAPER);
            for y in [3.0, 6.5, 10.0, 13.5] {
                pen.disc(3.0, y, 1.0, STEEL);
            }
        }
        Icon::Help => {
            pen.disc(8.0, 8.0, 7.0, BLUE);
            let mark: Vec<Pos2> = (0..=10)
                .map(|step| {
                    let angle = std::f32::consts::PI * (1.05 + step as f32 * 0.115);
                    pen.at(8.0 + angle.cos() * 2.6, 6.2 + angle.sin() * 2.6)
                })
                .collect();
            pen.painter.add(Shape::line(
                mark,
                Stroke::new(1.7 * pen.unit, pen.color(PAPER)),
            ));
            pen.line((10.4, 7.6), (8.0, 9.4), 1.7, PAPER);
            pen.line((8.0, 9.4), (8.0, 10.6), 1.7, PAPER);
            pen.painter
                .circle_filled(pen.at(8.0, 12.8), 1.0 * pen.unit, pen.color(PAPER));
        }
    }
}

/// Paint `icon` in color; `color` only matters for a disabled look, which
/// is chosen when it is mostly transparent.
pub fn paint(painter: &egui::Painter, rect: Rect, icon: Icon, color: Color32) {
    paint_color(painter, rect, icon, color.a() > 120);
}

/// Raised or sunken edges like a classic toolbar button: light on the
/// top and left, dark on the bottom and right (swapped when sunken).
pub fn bevel(painter: &egui::Painter, rect: Rect, face: Color32, sunken: bool) {
    let light = lighten(face, 0.75);
    let dark = darken(face, 0.38);
    let (top_left, bottom_right) = if sunken { (dark, light) } else { (light, dark) };
    let rect = rect.shrink(0.5);
    let stroke = |color| Stroke::new(1.0, color);
    painter.line_segment([rect.left_bottom(), rect.left_top()], stroke(top_left));
    painter.line_segment([rect.left_top(), rect.right_top()], stroke(top_left));
    painter.line_segment(
        [rect.right_top(), rect.right_bottom()],
        stroke(bottom_right),
    );
    painter.line_segment(
        [rect.right_bottom(), rect.left_bottom()],
        stroke(bottom_right),
    );
}

/// An etched vertical line between toolbar groups.
pub fn etched_separator(ui: &mut egui::Ui, face: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 24.0), egui::Sense::hover());
    let x = rect.center().x.round() + 0.5;
    let painter = ui.painter();
    painter.line_segment(
        [pos2(x, rect.top() + 2.0), pos2(x, rect.bottom() - 2.0)],
        Stroke::new(1.0, darken(face, 0.32)),
    );
    painter.line_segment(
        [
            pos2(x + 1.0, rect.top() + 2.0),
            pos2(x + 1.0, rect.bottom() - 2.0),
        ],
        Stroke::new(1.0, lighten(face, 0.7)),
    );
}

/// The ridged handle at the start of a toolbar band.
pub fn gripper(ui: &mut egui::Ui, face: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(7.0, 24.0), egui::Sense::hover());
    let band = Rect::from_min_max(
        pos2(rect.left() + 2.0, rect.top() + 2.0),
        pos2(rect.left() + 5.0, rect.bottom() - 2.0),
    );
    bevel(ui.painter(), band, face, false);
}

/// A toolbar button: the icon (with a caption if given), flat until the
/// pointer is over it, raised then, and sunken while pressed.
pub fn tool_button(
    ui: &mut egui::Ui,
    icon: Icon,
    caption: &str,
    tip: &str,
    enabled: bool,
) -> egui::Response {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let galley = ui
        .painter()
        .layout_no_wrap(caption.to_string(), font, Color32::PLACEHOLDER);
    let icon_side = 18.0;
    let gap = if caption.is_empty() { 0.0 } else { 5.0 };
    let size = egui::vec2(
        6.0 + icon_side + gap + galley.size().x + 6.0,
        (icon_side + 8.0).max(galley.size().y + 8.0),
    );
    let sense = if enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    let visuals = ui.visuals();
    let face = super::color_face();
    let text_color = if enabled {
        visuals.widgets.inactive.fg_stroke.color
    } else {
        visuals.widgets.inactive.fg_stroke.color.gamma_multiply(0.4)
    };
    let pressed = enabled && response.is_pointer_button_down_on();
    if enabled && (response.hovered() || pressed) {
        let fill = if pressed {
            darken(face, 0.06)
        } else {
            lighten(face, 0.12)
        };
        ui.painter().rect_filled(rect, 0.0, fill);
        bevel(ui.painter(), rect, face, pressed);
    }
    // A pressed button's face moves down and right by a pixel.
    let nudge = if pressed {
        Vec2::splat(1.0)
    } else {
        Vec2::ZERO
    };
    let icon_rect = Rect::from_center_size(
        pos2(rect.left() + 6.0 + icon_side / 2.0, rect.center().y) + nudge,
        Vec2::splat(icon_side),
    );
    paint_color(ui.painter(), icon_rect, icon, enabled);
    if !caption.is_empty() {
        let pos = pos2(
            icon_rect.right() + gap,
            rect.center().y - galley.size().y / 2.0,
        ) + nudge;
        ui.painter().galley(pos, galley, text_color);
    }
    response.on_hover_text(tip)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fills_are_turned_clockwise() {
        let ccw = vec![
            pos2(0.0, 0.0),
            pos2(0.0, 1.0),
            pos2(1.0, 1.0),
            pos2(1.0, 0.0),
        ];
        let fixed = clockwise(ccw);
        assert_eq!(fixed[0], pos2(1.0, 0.0));
        let cw = vec![pos2(0.0, 0.0), pos2(1.0, 0.0), pos2(1.0, 1.0)];
        assert_eq!(clockwise(cw.clone()), cw);
    }
}
