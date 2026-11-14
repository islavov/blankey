//! What a value of a field type must be: the types there are, and the checksums of the Bulgarian identifiers. The
//! app's forms and its PII recognizer use the same checks.

use chrono::NaiveDate;

pub const TYPES: &[&str] = &["text", "number", "date", "bool", "egn", "iban", "email", "phone"];
const EGN_WEIGHTS: [u32; 9] = [2, 4, 8, 5, 10, 9, 7, 3, 6];
const LNCH_WEIGHTS: [u32; 9] = [21, 19, 17, 13, 11, 9, 7, 3, 1];
const EIK9_WEIGHTS: [[u32; 8]; 2] = [[1, 2, 3, 4, 5, 6, 7, 8], [3, 4, 5, 6, 7, 8, 9, 10]];
const EIK13_WEIGHTS: [[u32; 4]; 2] = [[2, 7, 3, 5], [4, 9, 5, 7]];

pub fn is_type(field_type: &str) -> bool {
    TYPES.contains(&field_type)
}

pub fn parse_date(value: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(value.trim(), "%d.%m.%Y").ok()
}

/// The digits of a 10-digit personal number, EGN or ЛНЧ.
pub fn ten_digits(value: &str) -> Option<Vec<u32>> {
    (value.len() == 10 && value.bytes().all(|b| b.is_ascii_digit()))
        .then(|| value.bytes().map(|b| (b - b'0') as u32).collect())
}

fn weighted(digits: &[u32], weights: [u32; 9]) -> u32 {
    digits[..9].iter().zip(weights).map(|(d, w)| d * w).sum()
}

fn digits(value: &str) -> Option<Vec<u32>> {
    value.chars().map(|c| c.to_digit(10)).collect()
}

/// Ten digits whose last is the EGN check digit; the date in them is not looked at.
pub fn egn_checksum(value: &str) -> bool {
    ten_digits(value).is_some_and(|d| weighted(&d, EGN_WEIGHTS) % 11 % 10 == d[9])
}

/// The EGN checksum holds and its first six digits are a real birth date.
pub fn valid_egn(value: &str) -> bool {
    egn_checksum(value) && egn_birth_date(value).is_some()
}

/// ЛНЧ, the personal number a foreigner has in place of an EGN.
pub fn valid_lnch(value: &str) -> bool {
    ten_digits(value).is_some_and(|d| weighted(&d, LNCH_WEIGHTS) % 10 == d[9])
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
