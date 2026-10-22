use serde::Serialize;

/// What a detected span is. `class` decides whether it is redacted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Person,
    /// A value the user marked to hide.
    Marked,
    Org,
    Address,
    Egn,
    Lnch,
    Eik,
    VatId,
    Iban,
    Bic,
    IdCard,
    IdIssueDate,
    IdIssuer,
    Passport,
    BirthDate,
    Email,
    Phone,
    Postcode,
    Plate,
    Ssn,
    Nino,
    Date,
    Money,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    /// Always redacted.
    Personal,
    /// Suggested as a template var, never redacted.
    Variable,
}

pub const KINDS: &[Kind] = &[
    Kind::Person,
    Kind::Marked,
    Kind::Org,
    Kind::Address,
    Kind::Egn,
    Kind::Lnch,
    Kind::Eik,
    Kind::VatId,
    Kind::Iban,
    Kind::Bic,
    Kind::IdCard,
    Kind::IdIssueDate,
    Kind::IdIssuer,
    Kind::Passport,
    Kind::BirthDate,
    Kind::Email,
    Kind::Phone,
    Kind::Postcode,
    Kind::Plate,
    Kind::Ssn,
    Kind::Nino,
    Kind::Date,
    Kind::Money,
];

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Person => "person",
            Kind::Marked => "marked",
            Kind::Org => "org",
            Kind::Address => "address",
            Kind::Egn => "egn",
            Kind::Lnch => "lnch",
            Kind::Eik => "eik",
            Kind::VatId => "vat_id",
            Kind::Iban => "iban",
            Kind::Bic => "bic",
            Kind::IdCard => "id_card",
            Kind::IdIssueDate => "id_issue_date",
            Kind::IdIssuer => "id_issuer",
            Kind::Passport => "passport",
            Kind::BirthDate => "birth_date",
            Kind::Email => "email",
            Kind::Phone => "phone",
            Kind::Postcode => "postcode",
            Kind::Plate => "plate",
            Kind::Ssn => "ssn",
            Kind::Nino => "nino",
            Kind::Date => "date",
            Kind::Money => "money",
        }
    }

    pub fn from_name(name: &str) -> Option<Kind> {
        KINDS.iter().copied().find(|k| k.name() == name)
    }

    pub fn class(self) -> Class {
        match self {
            Kind::Date | Kind::Money => Class::Variable,
            _ => Class::Personal,
        }
    }

    pub fn personal(self) -> bool {
        self.class() == Class::Personal
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    /// A T1 recognizer by id.
    Pattern(&'static str),
    /// A T2 label, as written in the document.
    Anchor(String),
    /// A company name found by its legal-form suffix.
    Suffix,
    /// An address found by its street and city tokens.
    Chain,
    /// Another occurrence of a value detected elsewhere in the document.
    Echo,
    /// Marked by the user.
    User,
}

/// One hit in one block. Offsets are byte offsets into the block text.
#[derive(Clone, Debug)]
pub struct Detection {
    pub kind: Kind,
    pub start: usize,
    pub end: usize,
    pub score: f32,
    pub source: Source,
}

impl Detection {
    pub fn width(&self) -> usize {
        self.end - self.start
    }

    pub fn overlaps(&self, other: &Detection) -> bool {
        self.start < other.end && other.start < self.end
    }

    pub fn label(&self) -> Option<&str> {
        match &self.source {
            Source::Anchor(label) => Some(label),
            _ => None,
        }
    }
}

/// The unit the detector reads: a docx paragraph or a PDF page. `pieces` are the byte ranges of its runs (docx)
/// or lines (PDF), so spans map back to them.
#[derive(Clone, Debug, Default)]
pub struct Block {
    pub text: String,
    pub pieces: Vec<(usize, usize)>,
}

impl Block {
    pub fn plain(text: &str) -> Block {
        Block { text: text.to_string(), pieces: vec![(0, text.len())] }
    }

    /// More than one piece holds part of the span.
    pub fn crosses(&self, start: usize, end: usize) -> bool {
        self.pieces.iter().filter(|(s, e)| *s < end && start < *e).count() > 1
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Occ {
    pub block: usize,
    pub start: usize,
    pub end: usize,
}

/// A distinct value across the document: every detection whose normalized value is equal.
#[derive(Clone, Debug)]
pub struct Entity {
    /// "pii:1": numbered by first appearance
    pub id: String,
    pub kind: Kind,
    pub norm: String,
    pub surfaces: Vec<String>,
    pub occurrences: Vec<Occ>,
    pub score: f32,
    pub crosses: bool,
}

/// Detections per block plus the entities they form.
#[derive(Debug, Default)]
pub struct Analysis {
    pub per_block: Vec<Vec<Detection>>,
    /// Index into `entities` for each detection of `per_block`.
    pub entity_of: Vec<Vec<usize>>,
    pub entities: Vec<Entity>,
}

impl Analysis {
    pub fn entity(&self, id: &str) -> Option<&Entity> {
        self.entities.iter().find(|e| e.id == id)
    }
}
