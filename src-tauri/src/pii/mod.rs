//! Local detection of personal data in document text: T1 patterns with checksums, T2 per-language labels.
//! See docs/pii-detection-rfc.md.

mod document;
mod grammar;
mod keys;
mod marks;
mod redact;
mod resolve;
mod t1;
mod t2;
mod types;
mod validate;

#[cfg(test)]
mod tests;

pub use document::{Document, EntityView, PageText};
pub use marks::{At, Draft, Mark};
pub use redact::{guard, mask_png};
pub use types::{Analysis, Block, Detection, Entity, Kind, Occ};

/// Detections at or above this are redacted.
pub const REDACT: f32 = 0.4;
/// Entities at or above this are suggested as template vars.
pub const SUGGEST: f32 = 0.7;

pub fn detect(text: &str) -> Vec<Detection> {
    let mut hits = t1::scan(text);
    let more = t2::scan(text, &mut hits);
    hits.extend(more);
    resolve::resolve(hits)
}

pub fn analyze(blocks: &[Block]) -> Analysis {
    let per_block = blocks.iter().map(|b| detect(&b.text)).collect();
    let mut a = resolve::group(blocks, per_block);
    resolve::propagate(blocks, &mut a);
    resolve::assign_roles(blocks, &mut a);
    a
}

/// One line of text with its personal values and checksummed IDs masked, for text that does not come from a
/// Document (library titles).
pub fn redact_line(text: &str) -> String {
    let mut out = text.to_string();
    for d in detect(text).iter().rev().filter(|d| d.kind.personal() && d.score >= REDACT) {
        out.replace_range(d.start..d.end, &format!("⟦{}⟧", d.kind.name()));
    }
    guard(&out).into_owned()
}
