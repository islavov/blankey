//! Local detection of personal data in document text: T1 patterns with checksums, T2 per-language labels.
//! See docs/pii-detection-rfc.md.

mod corrections;
mod document;
mod grammar;
mod redact;
mod resolve;
mod t1;
mod t2;
mod types;
mod validate;

#[cfg(test)]
mod tests;

pub use corrections::Corrections;
pub use document::{Document, EntityView, PageText};
pub use redact::{guard, mask_png};
pub use types::{Analysis, Block, Detection, Entity, Kind, Occ};

/// Detections at or above this are redacted.
pub const REDACT: f32 = 0.4;

pub fn detect(text: &str) -> Vec<Detection> {
    let mut hits = t1::scan(text);
    let more = t2::scan(text, &mut hits);
    hits.extend(more);
    resolve::resolve(hits)
}

pub fn analyze(blocks: &[Block], corrections: &Corrections) -> Analysis {
    let per_block = blocks.iter().map(|b| corrections.apply(&b.text, detect(&b.text))).collect();
    let mut a = resolve::group(blocks, per_block);
    resolve::propagate(blocks, &mut a);
    a
}

/// One line of text with its personal values and checksummed IDs masked, for text that does not come from a
/// Document (library titles).
pub fn redact_line(text: &str) -> String {
    let mut out = text.to_string();
    for d in detect(text).iter().rev().filter(|d| d.kind.personal() && d.score >= REDACT) {
        let masked = format!("⟦{}⟧", redact::shape(&text[d.start..d.end]));
        out.replace_range(d.start..d.end, &masked);
    }
    guard(&out).into_owned()
}
