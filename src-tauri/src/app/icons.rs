//! Vector-drawn tray icon: a document with a padlock (open when the vault is unlocked) and a dot for pending
//! requests. Monochrome on macOS (a template image the menu bar tints), coloured elsewhere.

use tiny_skia::{
    BlendMode, Color, FillRule, LineCap, LineJoin, Paint, Path, PathBuilder, Pixmap, Rect, Stroke, Transform,
};

const ACCENT: (u8, u8, u8) = (0x2f, 0x7d, 0x5b);
const ACCENT_LOCKED: (u8, u8, u8) = (0x56, 0x60, 0x6b);
const BADGE: (u8, u8, u8) = (0xe5, 0x48, 0x4d);

fn paint(rgb: (u8, u8, u8), alpha: u8) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color(Color::from_rgba8(rgb.0, rgb.1, rgb.2, alpha));
    paint.anti_alias = true;
    paint
}

fn clear_paint() -> Paint<'static> {
    Paint { blend_mode: BlendMode::Clear, anti_alias: true, ..Paint::default() }
}

fn stroke(width: f32) -> Stroke {
    Stroke { width, line_cap: LineCap::Round, line_join: LineJoin::Round, ..Stroke::default() }
}

/// Sheet with a folded top-right corner.
fn document(size: f32) -> Path {
    let (left, top, right, bottom) = (size * 0.14, size * 0.06, size * 0.70, size * 0.86);
    let (fold, radius) = (size * 0.18, size * 0.06);
    let mut pb = PathBuilder::new();
    pb.move_to(left + radius, top);
    pb.line_to(right - fold, top);
    pb.line_to(right, top + fold);
    pb.line_to(right, bottom - radius);
    pb.quad_to(right, bottom, right - radius, bottom);
    pb.line_to(left + radius, bottom);
    pb.quad_to(left, bottom, left, bottom - radius);
    pb.line_to(left, top + radius);
    pb.quad_to(left, top, left + radius, top);
    pb.close();
    pb.finish().expect("document path")
}

fn fold(size: f32) -> Path {
    let (right, top, fold) = (size * 0.70, size * 0.06, size * 0.18);
    let mut pb = PathBuilder::new();
    pb.move_to(right - fold, top);
    pb.line_to(right - fold, top + fold);
    pb.line_to(right, top + fold);
    pb.finish().expect("fold path")
}

fn rounded_rect(rect: Rect, r: f32) -> Path {
    let (l, t, rr, b) = (rect.left(), rect.top(), rect.right(), rect.bottom());
    let mut pb = PathBuilder::new();
    pb.move_to(l + r, t);
    pb.line_to(rr - r, t);
    pb.quad_to(rr, t, rr, t + r);
    pb.line_to(rr, b - r);
    pb.quad_to(rr, b, rr - r, b);
    pb.line_to(l + r, b);
    pb.quad_to(l, b, l, b - r);
    pb.line_to(l, t + r);
    pb.quad_to(l, t, l + r, t);
    pb.close();
    pb.finish().expect("rounded rect")
}

/// (body, shackle)
fn lock(size: f32, locked: bool) -> (Path, Path) {
    let body = Rect::from_xywh(size * 0.50, size * 0.56, size * 0.42, size * 0.34).expect("lock body");
    let body_path = rounded_rect(body, size * 0.06);
    let shackle_w = body.width() * 0.62;
    let shackle_left = body.left() + body.width() / 2.0 - shackle_w / 2.0;
    let radius = shackle_w / 2.0;
    let lift = if locked { 0.0 } else { size * 0.09 };
    let arc_center_y = body.top() - size * 0.20 + radius - lift;
    let mut pb = PathBuilder::new();
    if locked {
        pb.move_to(shackle_left, body.top());
    } else {
        pb.move_to(shackle_left, arc_center_y + size * 0.05);
    }
    pb.line_to(shackle_left, arc_center_y);
    // half circle over the top, approximated with two quarter arcs
    let k = 0.552_284_8 * radius;
    let cx = shackle_left + radius;
    pb.cubic_to(shackle_left, arc_center_y - k, cx - k, arc_center_y - radius, cx, arc_center_y - radius);
    pb.cubic_to(cx + k, arc_center_y - radius, cx + radius, arc_center_y - k, cx + radius, arc_center_y);
    pb.line_to(shackle_left + shackle_w, body.top());
    (body_path, pb.finish().expect("shackle"))
}

fn circle(cx: f32, cy: f32, r: f32) -> Path {
    PathBuilder::from_circle(cx, cy, r).expect("circle")
}

pub fn draw(size: u32, locked: bool, pending: bool, mono: bool) -> Vec<u8> {
    let s = size as f32;
    let mut pixmap = Pixmap::new(size, size).expect("icon size");
    let ink = if mono {
        (0, 0, 0)
    } else if locked {
        ACCENT_LOCKED
    } else {
        ACCENT
    };
    let width = (s * 0.075).max(1.5);
    let t = Transform::identity();

    let doc = document(s);
    if !mono {
        pixmap.fill_path(&doc, &paint((255, 255, 255), 255), FillRule::Winding, t, None);
    }
    pixmap.stroke_path(&doc, &paint(ink, 255), &stroke(width), t, None);
    pixmap.stroke_path(&fold(s), &paint(ink, 255), &stroke(width), t, None);
    for (i, end) in [0.52, 0.44, 0.36].into_iter().enumerate() {
        let y = s * (0.36 + i as f32 * 0.13);
        let mut pb = PathBuilder::new();
        pb.move_to(s * 0.26, y);
        pb.line_to(s * end, y);
        if let Some(line) = pb.finish() {
            pixmap.stroke_path(&line, &paint(ink, 255), &stroke(width * 0.8), t, None);
        }
    }

    let (body, shackle) = lock(s, locked);
    // knock out the area under the lock so it reads clearly over the sheet
    pixmap.stroke_path(&body, &clear_paint(), &stroke(width * 2.2), t, None);
    pixmap.fill_path(&body, &clear_paint(), FillRule::Winding, t, None);
    pixmap.stroke_path(&shackle, &clear_paint(), &stroke(width * 2.2), t, None);
    pixmap.stroke_path(&shackle, &paint(ink, 255), &stroke(width), t, None);
    pixmap.fill_path(&body, &paint(ink, 255), FillRule::Winding, t, None);

    if pending {
        let radius = s * 0.13;
        let (cx, cy) = (s - radius - 1.0, radius + 1.0);
        pixmap.fill_path(&circle(cx, cy, radius + width), &clear_paint(), FillRule::Winding, t, None);
        let colour = if mono { (0, 0, 0) } else { BADGE };
        pixmap.fill_path(&circle(cx, cy, radius), &paint(colour, 255), FillRule::Winding, t, None);
    }
    // tiny-skia stores premultiplied alpha; Tauri takes straight RGBA
    pixmap
        .pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draws_every_state() {
        for (locked, pending, mono) in [(true, false, true), (false, true, false), (true, true, false)] {
            let rgba = draw(44, locked, pending, mono);
            assert_eq!(rgba.len(), 44 * 44 * 4);
            assert!(rgba.chunks(4).any(|p| p[3] > 0));
        }
    }
}
