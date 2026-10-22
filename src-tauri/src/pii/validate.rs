//! Checksums of the identifiers T1 recognizes. EGN, IBAN and email live in `fieldtypes`.

use chrono::NaiveDate;

pub use crate::fieldtypes::{valid_email, valid_iban};

const LNCH_WEIGHTS: [u32; 9] = [21, 19, 17, 13, 11, 9, 7, 3, 1];
const EIK9_WEIGHTS: [[u32; 8]; 2] = [[1, 2, 3, 4, 5, 6, 7, 8], [3, 4, 5, 6, 7, 8, 9, 10]];
const EIK13_WEIGHTS: [[u32; 4]; 2] = [[2, 7, 3, 5], [4, 9, 5, 7]];

fn digits(value: &str) -> Option<Vec<u32>> {
    value.chars().map(|c| c.to_digit(10)).collect()
}

/// The EGN checksum holds and its first six digits are a real birth date.
pub fn valid_egn(value: &str) -> bool {
    crate::fieldtypes::valid_egn(value) && egn_birth_date(value).is_some()
}

pub fn egn_birth_date(value: &str) -> Option<NaiveDate> {
    let d = digits(value.get(..6)?)?;
    let (yy, mm, dd) = (d[0] * 10 + d[1], d[2] * 10 + d[3], d[4] * 10 + d[5]);
    let (century, month) = match mm {
        1..=12 => (1900, mm),
        21..=32 => (1800, mm - 20),
        41..=52 => (2000, mm - 40),
        _ => return None,
    };
    NaiveDate::from_ymd_opt((century + yy) as i32, month, dd)
}

pub fn valid_lnch(value: &str) -> bool {
    let Some(d) = digits(value).filter(|d| d.len() == 10) else { return false };
    d[..9].iter().zip(LNCH_WEIGHTS).map(|(d, w)| d * w).sum::<u32>() % 10 == d[9]
}

fn eik_digit<const N: usize>(d: &[u32], weights: &[[u32; N]; 2]) -> u32 {
    for w in weights {
        let sum = d.iter().zip(w).map(|(d, w)| d * w).sum::<u32>() % 11;
        if sum != 10 {
            return sum;
        }
    }
    0
}

/// A 9-digit EIK/BULSTAT, or a 13-digit one for a branch.
pub fn valid_eik(value: &str) -> bool {
    let Some(d) = digits(value) else { return false };
    match d.len() {
        9 => eik_digit(&d[..8], &EIK9_WEIGHTS) == d[8],
        13 => eik_digit(&d[..8], &EIK9_WEIGHTS) == d[8] && eik_digit(&d[8..12], &EIK13_WEIGHTS) == d[12],
        _ => false,
    }
}

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
