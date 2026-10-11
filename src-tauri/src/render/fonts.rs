//! The bundled Noto Sans: metrics for fitting values into form fields, glyph coverage, and a subsetted
//! Type0 font for drawing values onto PDF pages.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use lopdf::{dictionary, Document, Object, ObjectId, Stream};
use ttf_parser::{Face, GlyphId};

pub const REGULAR: &[u8] = include_bytes!("../../assets/fonts/NotoSans-Regular.ttf");
pub const BOLD: &[u8] = include_bytes!("../../assets/fonts/NotoSans-Bold.ttf");
pub const MONO: &[u8] = include_bytes!("../../assets/fonts/NotoSansMono-Regular.ttf");

static FACE: LazyLock<Face<'static>> = LazyLock::new(|| Face::parse(REGULAR, 0).expect("bundled font parses"));

pub fn face() -> &'static Face<'static> {
    &FACE
}

fn glyph(ch: char) -> GlyphId {
    FACE.glyph_index(ch).unwrap_or(GlyphId(0))
}

fn advance(gid: GlyphId) -> f32 {
    FACE.glyph_hor_advance(gid).unwrap_or(0) as f32 / FACE.units_per_em() as f32
}

/// Width of `text` in points at `size`.
pub fn string_width(text: &str, size: f32) -> f32 {
    text.chars().map(|ch| advance(glyph(ch))).sum::<f32>() * size
}

pub fn covers(ch: char) -> bool {
    FACE.glyph_index(ch).is_some()
}

/// Characters the font cannot draw, sorted and without duplicates.
pub fn missing_glyphs(value: &str) -> Vec<String> {
    let mut missing: Vec<char> = value.chars().filter(|ch| !ch.is_whitespace() && !covers(*ch)).collect();
    missing.sort_unstable();
    missing.dedup();
    missing.into_iter().map(String::from).collect()
}

/// Collects the glyphs used on the pages, then embeds a subset of the font.
pub struct EmbeddedFont {
    remapper: subsetter::GlyphRemapper,
    /// new gid -> (old gid, character)
    used: BTreeMap<u16, (u16, char)>,
}

impl Default for EmbeddedFont {
    fn default() -> Self {
        Self::new()
    }
}

impl EmbeddedFont {
    pub fn new() -> Self {
        Self { remapper: subsetter::GlyphRemapper::new(), used: BTreeMap::new() }
    }

    /// The text as 2-byte glyph ids for an Identity-H Tj string.
    pub fn encode(&mut self, text: &str) -> Vec<u8> {
        let mut out = Vec::with_capacity(text.len() * 2);
        for ch in text.chars() {
            let old = glyph(ch).0;
            let new = self.remapper.remap(old);
            self.used.entry(new).or_insert((old, ch));
            out.extend(new.to_be_bytes());
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.used.is_empty()
    }

    /// Adds the Type0 font with its subset to the document.
    pub fn embed(self, doc: &mut Document) -> Result<ObjectId, String> {
        let face = face();
        let scale = 1000.0 / face.units_per_em() as f32;
        let subset = subsetter::subset(REGULAR, 0, &self.remapper).map_err(|e| format!("font subset: {e:?}"))?;
        let base_font = "BLNKEY+NotoSans-Regular";
        // new gids are dense from 0 (.notdef) upwards
        let widths: Vec<Object> = (0..self.remapper.num_gids())
            .map(|gid| {
                let old = self.used.get(&gid).map_or(0, |(old, _)| *old);
                Object::Integer((advance(GlyphId(old)) * 1000.0).round() as i64)
            })
            .collect();

        let file_len = subset.len() as i64;
        let font_file =
            doc.add_object(Stream::new(dictionary! { "Length1" => file_len }, subset).with_compression(true));
        let bbox = face.global_bounding_box();
        let descriptor = doc.add_object(dictionary! {
            "Type" => "FontDescriptor",
            "FontName" => Object::Name(base_font.into()),
            "Flags" => 32,
            "FontBBox" => vec![
                ((bbox.x_min as f32) * scale).into(),
                ((bbox.y_min as f32) * scale).into(),
                ((bbox.x_max as f32) * scale).into(),
                ((bbox.y_max as f32) * scale).into(),
            ],
            "ItalicAngle" => 0,
            "Ascent" => (face.ascender() as f32 * scale).round() as i64,
            "Descent" => (face.descender() as f32 * scale).round() as i64,
            "CapHeight" => (face.capital_height().unwrap_or(face.ascender()) as f32 * scale).round() as i64,
            "StemV" => 80,
            "FontFile2" => font_file,
        });
        let cid_font = doc.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "CIDFontType2",
            "BaseFont" => Object::Name(base_font.into()),
            "CIDSystemInfo" => dictionary! {
                "Registry" => Object::string_literal("Adobe"),
                "Ordering" => Object::string_literal("Identity"),
                "Supplement" => 0,
            },
            "FontDescriptor" => descriptor,
            "W" => vec![Object::Integer(0), Object::Array(widths)],
            "CIDToGIDMap" => "Identity",
        });
        let to_unicode = doc.add_object(Stream::new(dictionary! {}, self.cmap().into_bytes()).with_compression(true));
        Ok(doc.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type0",
            "BaseFont" => Object::Name(base_font.into()),
            "Encoding" => "Identity-H",
            "DescendantFonts" => vec![Object::Reference(cid_font)],
            "ToUnicode" => to_unicode,
        }))
    }

    fn cmap(&self) -> String {
        let entries: Vec<String> = self
            .used
            .iter()
            .filter(|(gid, _)| **gid != 0)
            .map(|(gid, (_, ch))| {
                let utf16: String = ch.encode_utf16(&mut [0; 2]).iter().map(|u| format!("{u:04X}")).collect();
                format!("<{gid:04X}> <{utf16}>")
            })
            .collect();
        let mut body = String::new();
        for chunk in entries.chunks(100) {
            body.push_str(&format!("{} beginbfchar\n{}\nendbfchar\n", chunk.len(), chunk.join("\n")));
        }
        format!(
            "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n\
             /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
             /CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n\
             1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n{body}\
             endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widths_and_coverage() {
        assert!(string_width("Иван", 9.0) > 15.0);
        assert!(string_width("ab", 10.0) > string_width("a", 10.0));
        assert_eq!(missing_glyphs("Ива 漢字 a"), ["字", "漢"]);
    }
}
