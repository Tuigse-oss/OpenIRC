//! The OpenIRC application icon, drawn in code.
//!
//! A blue rounded square holding a white speech bubble with a `#`, the IRC
//! channel sign. Drawing it from shapes keeps it sharp at every size and
//! needs no image files or image crates. `build.rs` includes this file to
//! write the Windows `.ico`, so it uses only `std`.

/// Unpremultiplied RGBA pixels for a `size` x `size` icon.
pub fn render(size: u32) -> Vec<u8> {
    const SAMPLES: u32 = 4;
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let mut acc = [0.0f32; 4];
            for sy in 0..SAMPLES {
                for sx in 0..SAMPLES {
                    let u = (x as f32 + (sx as f32 + 0.5) / SAMPLES as f32) / size as f32;
                    let v = (y as f32 + (sy as f32 + 0.5) / SAMPLES as f32) / size as f32;
                    let [r, g, b, a] = shade(u, v);
                    acc[0] += r * a;
                    acc[1] += g * a;
                    acc[2] += b * a;
                    acc[3] += a;
                }
            }
            let n = (SAMPLES * SAMPLES) as f32;
            let alpha = acc[3] / n;
            let color = |channel: f32| {
                if alpha <= 0.0 {
                    0
                } else {
                    ((channel / n / alpha) * 255.0).round().clamp(0.0, 255.0) as u8
                }
            };
            out.extend_from_slice(&[
                color(acc[0]),
                color(acc[1]),
                color(acc[2]),
                (alpha * 255.0).round().clamp(0.0, 255.0) as u8,
            ]);
        }
    }
    out
}

/// A Windows `.ico` file holding the icon at each size in `sizes` (each at
/// most 256), stored as 32-bit images. Used by `build.rs`.
#[allow(dead_code)]
pub fn ico(sizes: &[u32]) -> Vec<u8> {
    let images: Vec<Vec<u8>> = sizes.iter().map(|&size| dib(size)).collect();
    let mut out = Vec::new();
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (size, image) in sizes.iter().zip(&images) {
        let side = if *size >= 256 { 0 } else { *size as u8 };
        out.extend_from_slice(&[side, side, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&(image.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += image.len() as u32;
    }
    for image in images {
        out.extend_from_slice(&image);
    }
    out
}

/// One icon image as a bottom-up BGRA bitmap plus an all-zero AND mask.
fn dib(size: u32) -> Vec<u8> {
    let rgba = render(size);
    let mask_row = size.div_ceil(32) * 4;
    let mut out = Vec::new();
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(size as i32).to_le_bytes());
    out.extend_from_slice(&((size * 2) as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&[0u8; 24]);
    for y in (0..size).rev() {
        for x in 0..size {
            let at = ((y * size + x) * 4) as usize;
            out.extend_from_slice(&[rgba[at + 2], rgba[at + 1], rgba[at], rgba[at + 3]]);
        }
    }
    out.extend(std::iter::repeat_n(0u8, (mask_row * size) as usize));
    out
}

/// Color and coverage at a point of the unit square.
fn shade(u: f32, v: f32) -> [f32; 4] {
    // Rounded square with a top-to-bottom gradient.
    let tile = rounded_box(u - 0.5, v - 0.5, 0.46, 0.46, 0.2);
    if tile > 0.0 {
        return [0.0; 4];
    }
    let top = [0.25, 0.52, 0.93];
    let bottom = [0.10, 0.27, 0.62];
    let mut color = mix(top, bottom, v);
    // A faint highlight along the top edge.
    let glow = (1.0 - (v / 0.35)).clamp(0.0, 1.0) * 0.12;
    color = mix(color, [1.0, 1.0, 1.0], glow);

    // Speech bubble: a rounded box plus a tail at the lower left.
    let bubble = rounded_box(u - 0.5, v - 0.46, 0.31, 0.25, 0.12);
    let tail = triangle(u, v, (0.28, 0.62), (0.44, 0.66), (0.24, 0.82));
    let in_bubble = bubble <= 0.0 || tail <= 0.0;
    if in_bubble {
        color = [1.0, 1.0, 1.0];
        // The `#`: two slanted uprights and two crossbars.
        let stroke = 0.032;
        let hash = [
            segment(u, v, (0.45, 0.30), (0.41, 0.62)),
            segment(u, v, (0.59, 0.30), (0.55, 0.62)),
            segment(u, v, (0.35, 0.40), (0.67, 0.40)),
            segment(u, v, (0.33, 0.52), (0.65, 0.52)),
        ]
        .into_iter()
        .fold(f32::MAX, f32::min);
        if hash <= stroke {
            color = mix(top, bottom, 0.55);
        }
    }
    [color[0], color[1], color[2], 1.0]
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

/// Signed distance to a box of half-size `w` x `h` with corner radius `r`.
fn rounded_box(x: f32, y: f32, w: f32, h: f32, r: f32) -> f32 {
    let qx = x.abs() - w + r;
    let qy = y.abs() - h + r;
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    outside + qx.max(qy).min(0.0) - r
}

/// Distance from a point to a line segment.
fn segment(x: f32, y: f32, a: (f32, f32), b: (f32, f32)) -> f32 {
    let (px, py) = (x - a.0, y - a.1);
    let (bx, by) = (b.0 - a.0, b.1 - a.1);
    let t = ((px * bx + py * by) / (bx * bx + by * by)).clamp(0.0, 1.0);
    ((px - bx * t).powi(2) + (py - by * t).powi(2)).sqrt()
}

/// Negative inside the triangle `a b c`, positive outside.
fn triangle(x: f32, y: f32, a: (f32, f32), b: (f32, f32), c: (f32, f32)) -> f32 {
    let side = |p: (f32, f32), q: (f32, f32)| (q.0 - p.0) * (y - p.1) - (q.1 - p.1) * (x - p.0);
    let (d1, d2, d3) = (side(a, b), side(b, c), side(c, a));
    let negative = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let positive = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    if negative && positive { 1.0 } else { -1.0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_has_transparent_corners_and_an_opaque_middle() {
        let size = 64;
        let rgba = render(size);
        assert_eq!(rgba.len(), (size * size * 4) as usize);
        assert_eq!(rgba[3], 0, "top-left corner is transparent");
        let middle = ((size / 2 * size + size / 2) * 4) as usize;
        assert_eq!(rgba[middle + 3], 255);
    }

    #[test]
    fn ico_header_lists_every_size() {
        let file = ico(&[16, 32, 256]);
        assert_eq!(&file[0..4], &[0, 0, 1, 0]);
        assert_eq!(u16::from_le_bytes([file[4], file[5]]), 3);
        assert_eq!(file[6], 16);
        assert_eq!(file[6 + 32], 0, "256 is stored as 0");
    }
}
