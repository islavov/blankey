//! Checksums of the identifiers T1 recognizes. EGN, ЛНЧ, IBAN and email live in `fieldtypes`.

use chrono::NaiveDate;

pub use crate::fieldtypes::{valid_email, valid_iban, valid_lnch};

pub use blankey_doctypes::values::{valid_egn, valid_eik};

/// A Bulgarian VAT number: BG and a valid EIK or EGN.
pub fn valid_vat_bg(value: &str) -> bool {
    value.strip_prefix("BG").is_some_and(|rest| valid_eik(rest) || valid_egn(rest))
}

pub fn valid_ssn(value: &str) -> bool {
    let d: String = value.chars().filter(char::is_ascii_digit).collect();
    if d.len() != 9 {
        return false;
    }
    let (area, group, serial) = (&d[..3], &d[3..5], &d[5..]);
    area != "000" && area != "666" && !area.starts_with('9') && group != "00" && serial != "0000"
}

pub fn valid_date(value: &str) -> bool {
    let core = value.trim_end_matches(|c: char| !c.is_ascii_digit());
    ["%d.%m.%Y", "%Y-%m-%d", "%d/%m/%Y"].iter().any(|f| NaiveDate::parse_from_str(core, f).is_ok())
}

/// 6 to 15 digits, the range of real phone numbers.
pub fn phone_digits(value: &str) -> bool {
    (6..=15).contains(&value.chars().filter(char::is_ascii_digit).count())
}
