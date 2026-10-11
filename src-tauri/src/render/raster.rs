//! Drawing on rendered pages: outlines, grid lines and small labels, then PNG encoding.

use ab_glyph::{Font, FontRef, PxScale, ScaleFont};
use tiny_skia::{Color, Paint, PathBuilder, Pixmap, Rect, Stroke, Transform};

use super::fonts;

pub fn pixmap_from_rgba(width: u32, height: u32, rgba: Vec<u8>) -> Result<Pixmap, String> {
    let size = tiny_skia::IntSize::from_wh(width, height).ok_or("empty page image")?;
    // rendered pages are opaque, so straight and premultiplied alpha agree
    Pixmap::from_vec(rgba, size).ok_or_else(|| "page image has the wrong size".to_string())
}

pub fn png(pixmap: &Pixmap) -> Result<Vec<u8>, String> {
    pixmap.encode_png().map_err(|e| e.to_string())
}

fn paint(r: u8, g: u8, b: u8, a: u8) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color(Color::from_rgba8(r, g, b, a));
    paint.anti_alias = true;
    paint
}

pub fn outline(pixmap: &mut Pixmap, x0: f32, y0: f32, x1: f32, y1: f32, rgba: (u8, u8, u8, u8)) {
    let Some(rect) = Rect::from_ltrb(x0, y0, x1.max(x0 + 1.0), y1.max(y0 + 1.0)) else { return };
    let path = PathBuilder::from_rect(rect);
    let stroke = Stroke { width: 1.0, ..Stroke::default() };
    pixmap.stroke_path(&path, &paint(rgba.0, rgba.1, rgba.2, rgba.3), &stroke, Transform::identity(), None);
}

pub fn line(pixmap: &mut Pixmap, x0: f32, y0: f32, x1: f32, y1: f32, rgba: (u8, u8, u8, u8)) {
    let mut builder = PathBuilder::new();
    builder.move_to(x0, y0);
    builder.line_to(x1, y1);
    let Some(path) = builder.finish() else { return };
    let stroke = Stroke { width: 1.0, ..Stroke::default() };
    pixmap.stroke_path(&path, &paint(rgba.0, rgba.1, rgba.2, rgba.3), &stroke, Transform::identity(), None);
}

/// Small text with its top-left corner at (x, y).
pub fn text(pixmap: &mut Pixmap, x: f32, y: f32, content: &str, px: f32, rgb: (u8, u8, u8)) {
    let font = FontRef::try_from_slice(fonts::REGULAR).expect("bundled font parses");
    let scaled = font.as_scaled(PxScale::from(px));
    let (width, height) = (pixmap.width() as i32, pixmap.height() as i32);
    let mut caret = x;
    let baseline = y + scaled.ascent();
    let data = pixmap.data_mut();
    for ch in content.chars() {
        let id = scaled.glyph_id(ch);
        let glyph = id.with_scale_and_position(px, ab_glyph::point(caret, baseline));
        caret += scaled.h_advance(id);
        let Some(outlined) = font.outline_glyph(glyph) else { continue };
        let bounds = outlined.px_bounds();
        outlined.draw(|gx, gy, coverage| {
            let px_x = bounds.min.x as i32 + gx as i32;
            let px_y = bounds.min.y as i32 + gy as i32;
            if px_x < 0 || px_y < 0 || px_x >= width || px_y >= height || coverage <= 0.0 {
                return;
            }
            let i = ((px_y * width + px_x) * 4) as usize;
            let a = coverage.min(1.0);
            for (channel, value) in [rgb.0, rgb.1, rgb.2].into_iter().enumerate() {
                let old = data[i + channel] as f32;
                data[i + channel] = (old + (value as f32 - old) * a).round() as u8;
            }
        });
    }
}
