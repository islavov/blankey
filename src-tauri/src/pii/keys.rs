//! Suggested profile keys and template var names, shared by every language so templates agree on names.

use super::types::Kind;

pub fn profile_key(kind: Kind) -> Option<&'static str> {
    Some(match kind {
        Kind::Person => "full_name",
        Kind::Representative => "representative",
        Kind::Org => "name",
        Kind::Address => "address",
        Kind::Egn => "egn",
        Kind::Lnch => "lnch",
        Kind::Eik => "eik",
        Kind::VatId => "vat_id",
        Kind::Iban => "iban",
        Kind::Bic => "bic",
        Kind::IdCard => "id_card.number",
        Kind::IdIssueDate => "id_card.issued_on",
        Kind::IdIssuer => "id_card.issuer",
        Kind::Passport => "passport.number",
        Kind::BirthDate => "birth_date",
        Kind::Email => "email",
        Kind::Phone => "phone",
        Kind::Postcode => "address.postcode",
        Kind::Plate => "vehicle.plate",
        Kind::Ssn => "ssn",
        Kind::Nino => "nino",
        Kind::Marked | Kind::Date | Kind::Money => return None,
    })
}

/// "seller" + "id_card.number" -> "seller_id_card_number"
pub fn var(role: &str, key: &str) -> String {
    format!("{role}_{}", key.replace('.', "_"))
}

/// "seller" + "id_card.number" -> "seller.id_card.number"
pub fn binding(role: &str, key: &str) -> String {
    format!("{role}.{key}")
}
