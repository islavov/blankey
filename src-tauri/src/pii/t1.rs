//! T1: identifiers found by shape and checksum alone. A recognizer that scores 0 alone still reports its hits,
//! so a T2 label can claim them ("л.к. № 645123456").

use std::sync::LazyLock;

use regex::Regex;

use super::types::{Detection, Kind, Source};
use super::validate as v;

pub struct Recognizer {
    pub id: &'static str,
    pub kind: Kind,
    re: Regex,
    /// The capture group that is the value; 0 for the whole match.
    group: usize,
    check: fn(&str) -> bool,
    alone: f32,
    /// On a failed check, retry without the last space-separated token (an IBAN followed by "BIC").
    shrink: bool,
}

fn any(_: &str) -> bool {
    true
}

fn r(id: &'static str, kind: Kind, re: &str, check: fn(&str) -> bool, alone: f32) -> Recognizer {
    Recognizer { id, kind, re: Regex::new(re).unwrap(), group: 0, check, alone, shrink: false }
}

const PLATE: &str = "АВЕКМНОРСТУХABEHKMOPCTXY";

pub static REGISTRY: LazyLock<Vec<Recognizer>> = LazyLock::new(|| {
    vec![
        r("egn", Kind::Egn, r"\b\d{10}\b", v::valid_egn, 0.8),
        r("lnch", Kind::Lnch, r"\b\d{10}\b", v::valid_lnch, 0.0),
        r("eik", Kind::Eik, r"\b(?:\d{13}|\d{9})\b", v::valid_eik, 0.0),
        r("vat_bg", Kind::VatId, r"\bBG\d{9,10}\b", v::valid_vat_bg, 0.95),
        Recognizer {
            shrink: true,
            ..r("iban", Kind::Iban, r"\b[A-Z]{2}\d{2}(?: ?[A-Z0-9]{4}){2,7}(?: ?[A-Z0-9]{1,3})?\b", v::valid_iban, 0.99)
        },
        r("bic", Kind::Bic, r"\b[A-Z]{6}[A-Z0-9]{2}(?:[A-Z0-9]{3})?\b", any, 0.0),
        r("email", Kind::Email, r"[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}", v::valid_email, 0.95),
        r("phone_bg", Kind::Phone, r"(?:\+359|\b00359|\b0)[ \-]?8[789](?:[ \-]?\d){7}\b", any, 0.85),
        r("phone", Kind::Phone, r"\+\d{1,3}(?:[ \-()]{0,2}\d){6,13}\b", v::phone_digits, 0.5),
        r("id_card", Kind::IdCard, r"\b\d{9}\b", any, 0.0),
        r("passport", Kind::Passport, r"\b[A-Z]{0,2}\d{6,9}\b", any, 0.0),
        r("ssn", Kind::Ssn, r"\b\d{3}-\d{2}-\d{4}\b", v::valid_ssn, 0.85),
        r("ssn_plain", Kind::Ssn, r"\b\d{9}\b", v::valid_ssn, 0.0),
        r("nino", Kind::Nino, r"\b[A-CEGHJ-PR-TW-Z]{2} ?\d{2} ?\d{2} ?\d{2} ?[A-D]\b", any, 0.85),
        r("plate_bg", Kind::Plate, &format!(r"\b[{PLATE}]{{1,2}} ?\d{{4}} ?[{PLATE}]{{2}}\b"), any, 0.85),
        r(
            "date",
            Kind::Date,
            r"\b(?:\d{1,2}\.\d{1,2}\.\d{4}|\d{4}-\d{2}-\d{2}|\d{1,2}/\d{1,2}/\d{4})\b",
            v::valid_date,
            0.9,
        ),
        Recognizer {
            group: 1,
            ..r(
                "money",
                Kind::Money,
                r"(\b\d{1,3}(?:[ \u{a0}.,]?\d{3})*(?:[.,]\d{1,2})?)[ \u{a0}]?(?:лв\.?|лева|BGN|EUR|евро|USD|GBP|€|£|\$)",
                any,
                0.9,
            )
        },
        Recognizer { group: 1, ..r("money_pre", Kind::Money, r"[€£$][ \u{a0}]?(\d[\d,.]*\d|\d)", any, 0.9) },
    ]
});

fn checked(r: &Recognizer, text: &str, start: usize, end: usize) -> Option<(usize, usize)> {
    let mut end = end;
    loop {
        if (r.check)(&text[start..end]) {
            return Some((start, end));
        }
        if !r.shrink {
            return None;
        }
        end = start + text[start..end].rfind(' ')?;
    }
}

pub fn scan(text: &str) -> Vec<Detection> {
    let mut out = Vec::new();
    for r in REGISTRY.iter() {
        for caps in r.re.captures_iter(text) {
            let Some(m) = caps.get(r.group) else { continue };
            if let Some((start, end)) = checked(r, text, m.start(), m.end()) {
                out.push(Detection { kind: r.kind, start, end, score: r.alone, source: Source::Pattern(r.id) });
            }
        }
    }
    out
}
