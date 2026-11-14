use serde_json::Value;

use crate::i18n::mark;

pub use blankey_doctypes::values::{egn_checksum as valid_egn, is_type, parse_date, valid_lnch, TYPES};

pub const TRUE_VALUES: &[&str] = &["1", "true", "да", "yes", "x"];

/// Why an `egn` field refuses a value; it takes an EGN or a ЛНЧ. The collection form runs the same check
/// (public/collect/validate.js on the relay).
fn egn_problem(value: &str) -> Option<&'static str> {
    if blankey_doctypes::values::ten_digits(value).is_none() {
        return Some(mark("An EGN or LNCh has 10 digits"));
    }
    (!valid_egn(value) && !valid_lnch(value)).then_some(mark("Not a valid EGN or LNCh: check for a mistyped digit"))
}

pub fn valid_iban(value: &str) -> bool {
    let iban: String = value.replace(' ', "").to_uppercase();
    let bytes = iban.as_bytes();
    let shape_ok = (14..=34).contains(&bytes.len())
        && bytes[..2].iter().all(u8::is_ascii_uppercase)
        && bytes[2..4].iter().all(u8::is_ascii_digit)
        && bytes[4..].iter().all(u8::is_ascii_alphanumeric);
    if !shape_ok {
        return false;
    }
    let rearranged = iban[4..].chars().chain(iban[..4].chars());
    let mut remainder: u32 = 0;
    for ch in rearranged {
        let n = ch.to_digit(36).unwrap();
        remainder = if n >= 10 { (remainder * 100 + n) % 97 } else { (remainder * 10 + n) % 97 };
    }
    remainder == 1
}

pub fn valid_email(value: &str) -> bool {
    let Some((local, domain)) = value.split_once('@') else { return false };
    let clean = |s: &str| !s.is_empty() && !s.contains(char::is_whitespace) && !s.contains('@');
    clean(local) && clean(domain) && domain.split_once('.').is_some_and(|(a, b)| !a.is_empty() && !b.is_empty())
}

/// An error message, or None when the value is acceptable.
pub fn validate(field_type: &str, value: &str) -> Option<&'static str> {
    if value.is_empty() {
        return None;
    }
    match field_type {
        "number" => value.replace(' ', "").replace(',', ".").parse::<f64>().is_err().then_some(mark("Invalid number")),
        "date" => parse_date(value).is_none().then_some(mark("Date must be DD.MM.YYYY")),
        "egn" => egn_problem(value),
        "iban" => (!valid_iban(value)).then_some(mark("Invalid IBAN")),
        "email" => (!valid_email(value)).then_some(mark("Invalid email")),
        _ => None,
    }
}

pub fn is_true(value: &str) -> bool {
    TRUE_VALUES.contains(&value.trim().to_lowercase().as_str())
}

/// The stored string as templates see it: numbers become int / float, bools become bool.
pub fn coerce(field_type: &str, value: &str) -> Value {
    match field_type {
        "number" => {
            let cleaned = value.replace(' ', "").replace(',', ".");
            match cleaned.parse::<rust_decimal::Decimal>() {
                Ok(n) if n.fract().is_zero() => i64::try_from(n.trunc())
                    .map(Value::from)
                    .unwrap_or_else(|_| Value::from(cleaned.parse::<f64>().unwrap_or_default())),
                Ok(_) => cleaned.parse::<f64>().map(Value::from).unwrap_or_else(|_| Value::from(value)),
                Err(_) => Value::from(value),
            }
        }
        "bool" => Value::Bool(is_true(value)),
        _ => Value::from(value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn egn() {
        assert!(valid_egn("7501020018"));
        assert!(!valid_egn("7501020019"));
        assert!(!valid_egn("75010200"));
        assert!(valid_lnch("7111042925"));
        assert!(!valid_lnch("7111042922"));
    }

    #[test]
    fn egn_fields_take_an_egn_or_a_lnch_and_say_why_not() {
        assert_eq!(validate("egn", "7501020018"), None);
        assert_eq!(validate("egn", "7111042925"), None);
        assert_eq!(validate("egn", "9208128911"), Some("Not a valid EGN or LNCh: check for a mistyped digit"));
        assert_eq!(validate("egn", "75010200"), Some("An EGN or LNCh has 10 digits"));
        assert_eq!(validate("egn", "75010200a8"), Some("An EGN or LNCh has 10 digits"));
    }

    #[test]
    fn iban() {
        assert!(valid_iban("BG80 BNBG 9661 1020 3456 78"));
        assert!(!valid_iban("BG80BNBG96611020345679"));
    }

    #[test]
    fn misc() {
        assert_eq!(validate("date", "31.12.2024"), None);
        assert!(validate("date", "2024-12-31").is_some());
        assert_eq!(validate("number", "1 234,50"), None);
        assert!(validate("email", "a@b").is_some());
        assert_eq!(validate("email", "a@b.bg"), None);
    }

    #[test]
    fn coercion() {
        assert_eq!(coerce("number", "1 500"), Value::from(1500));
        assert_eq!(coerce("number", "1 500,50"), Value::from(1500.5));
        assert_eq!(coerce("bool", "Да"), Value::Bool(true));
        assert_eq!(coerce("bool", "no"), Value::Bool(false));
        assert_eq!(coerce("text", "x"), Value::from("x"));
    }
}
