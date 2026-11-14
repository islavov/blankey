//! What a file is, told from its content: every file that enters the vault goes through `recognize`.
//!
//! This is the identity stage: cheap, synchronous, and always run. It hashes the content and tells its kind,
//! whatever the file is called, so the lists, the tools and the assistant agree on what a file is. The hash is how a
//! file is known again later (it is the same SHA-256 the template folders and the PII corrections go by).
//!
//! Content stages (is it a template already made, does it hold personal data at a glance, what an image shows) are
//! slower and come later as their own functions: each takes a `Recognized` and keys what it finds by its `sha256`.

use std::path::Path;

use sha2::{Digest, Sha256};

/// How an OLE compound file starts, and the name of the stream a .doc keeps its text in (UTF-16).
const OLE: &[u8] = &[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
const WORD_STREAM: &[u8] = b"W\0o\0r\0d\0D\0o\0c\0u\0m\0e\0n\0t\0";
/// A PDF may start with a few stray bytes; its header is within this many.
const PDF_HEADER_WITHIN: usize = 1024;
const HEIC_BRANDS: [&[u8]; 3] = [b"heic", b"heix", b"mif1"];

/// What a file is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Pdf,
    Docx,
    /// A Word 97-2003 file (.doc): kept and opened like any file, but tools cannot read it.
    OldWord,
    Png,
    Jpeg,
    Gif,
    Webp,
    Heic,
    Unknown,
}

const KINDS: [Kind; 8] =
    [Kind::Pdf, Kind::Docx, Kind::OldWord, Kind::Png, Kind::Jpeg, Kind::Gif, Kind::Webp, Kind::Heic];

impl Kind {
    pub fn media_type(self) -> &'static str {
        match self {
            Kind::Pdf => "application/pdf",
            Kind::Docx => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            Kind::OldWord => "application/msword",
            Kind::Png => "image/png",
            Kind::Jpeg => "image/jpeg",
            Kind::Gif => "image/gif",
            Kind::Webp => "image/webp",
            Kind::Heic => "image/heic",
            Kind::Unknown => "application/octet-stream",
        }
    }

    /// The extension a file of this kind is written with.
    pub fn extension(self) -> Option<&'static str> {
        match self {
            Kind::Pdf => Some("pdf"),
            Kind::Docx => Some("docx"),
            Kind::OldWord => Some("doc"),
            Kind::Png => Some("png"),
            Kind::Jpeg => Some("jpg"),
            Kind::Gif => Some("gif"),
            Kind::Webp => Some("webp"),
            Kind::Heic => Some("heic"),
            Kind::Unknown => None,
        }
    }

    /// A document the tools can read.
    pub fn readable(self) -> bool {
        matches!(self, Kind::Pdf | Kind::Docx)
    }

    pub fn is_image(self) -> bool {
        matches!(self, Kind::Png | Kind::Jpeg | Kind::Gif | Kind::Webp | Kind::Heic)
    }

    /// The kind a stored media type stands for.
    pub fn from_media_type(media_type: &str) -> Kind {
        KINDS.into_iter().find(|kind| kind.media_type() == media_type).unwrap_or(Kind::Unknown)
    }

    /// What the name says, for content that tells nothing.
    fn from_name(name: &str) -> Kind {
        let extension = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
        match extension.as_str() {
            "jpeg" => Kind::Jpeg,
            other => KINDS.into_iter().find(|kind| kind.extension() == Some(other)).unwrap_or(Kind::Unknown),
        }
    }

    /// What the content is, when it shows.
    fn from_content(content: &[u8]) -> Option<Kind> {
        let brand = content.get(4..12).filter(|b| b.starts_with(b"ftyp")).map(|b| &b[4..]);
        if contains(&content[..content.len().min(PDF_HEADER_WITHIN)], b"%PDF-") {
            Some(Kind::Pdf)
        } else if content.starts_with(b"PK\x03\x04") && contains(content, b"word/document.xml") {
            Some(Kind::Docx)
        } else if content.starts_with(OLE) && contains(content, WORD_STREAM) {
            Some(Kind::OldWord)
        } else if content.starts_with(b"\x89PNG\r\n\x1a\n") {
            Some(Kind::Png)
        } else if content.starts_with(&[0xFF, 0xD8, 0xFF]) {
            Some(Kind::Jpeg)
        } else if content.starts_with(b"GIF8") {
            Some(Kind::Gif)
        } else if content.starts_with(b"RIFF") && content.get(8..12) == Some(b"WEBP") {
            Some(Kind::Webp)
        } else if brand.is_some_and(|brand| HEIC_BRANDS.contains(&brand)) {
            Some(Kind::Heic)
        } else {
            None
        }
    }
}

fn contains(content: &[u8], needle: &[u8]) -> bool {
    content.windows(needle.len()).any(|window| window == needle)
}

/// What recognition knows about one file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recognized {
    /// SHA-256 of the content, lowercase hex
    pub sha256: String,
    pub size: u64,
    pub kind: Kind,
}

/// Recognizes a file by its content; `name` decides only when the content tells nothing.
pub fn recognize(name: &str, content: &[u8]) -> Recognized {
    Recognized {
        sha256: data_encoding::HEXLOWER.encode(&Sha256::digest(content)),
        size: content.len() as u64,
        kind: Kind::from_content(content).unwrap_or_else(|| Kind::from_name(name)),
    }
}

/// `recognize` for a file on disk.
pub fn recognize_file(path: &Path) -> std::io::Result<Recognized> {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    Ok(recognize(name, &std::fs::read(path)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::tools::tests::fixture;

    fn old_word() -> Vec<u8> {
        [OLE, &[0; 64], WORD_STREAM].concat()
    }

    #[test]
    fn tells_the_kind_from_the_content_whatever_the_name() {
        let kind = |name: &str, content: &[u8]| recognize(name, content).kind;
        assert_eq!(kind("scan.bin", b"%PDF-1.7\n"), Kind::Pdf);
        assert_eq!(kind("scan.bin", b"\n\n%PDF-1.4"), Kind::Pdf, "a header after stray bytes");
        assert_eq!(kind("contract.docx", &old_word()), Kind::OldWord, "a .doc renamed to .docx");
        assert_eq!(kind("photo", b"\x89PNG\r\n\x1a\n...."), Kind::Png);
        assert_eq!(kind("photo.png", &[0xFF, 0xD8, 0xFF, 0xE0]), Kind::Jpeg);
        assert_eq!(kind("a", b"GIF89a"), Kind::Gif);
        assert_eq!(kind("a", b"RIFF\x10\0\0\0WEBPVP8 "), Kind::Webp);
        assert_eq!(kind("a", b"\0\0\0\x18ftypheic\0\0\0\0"), Kind::Heic);
        assert_eq!(kind("a.mp4", b"\0\0\0\x18ftypisom\0\0\0\0"), Kind::Unknown);
    }

    #[test]
    fn recognizes_real_documents() {
        let docx = std::fs::read(fixture("filled.docx")).unwrap();
        assert_eq!(recognize("contract.doc", &docx).kind, Kind::Docx, "a docx named .doc");
        let pdf = recognize_file(&fixture("form.pdf")).unwrap();
        assert_eq!(pdf.kind, Kind::Pdf);
        assert_eq!(pdf.size, std::fs::metadata(fixture("form.pdf")).unwrap().len());
        assert_eq!(pdf.sha256, crate::library::hash_file(&fixture("form.pdf")).unwrap(), "the hash the folders use");
    }

    #[test]
    fn the_name_decides_only_when_the_content_tells_nothing() {
        let kind = |name: &str| recognize(name, b"plain text").kind;
        assert_eq!(kind("deed.PDF"), Kind::Pdf);
        assert_eq!(kind("letter.doc"), Kind::OldWord, "an RTF or HTML file saved as .doc");
        assert_eq!(kind("card.jpeg"), Kind::Jpeg);
        assert_eq!(kind("notes.txt"), Kind::Unknown);
        assert_eq!(kind("notes"), Kind::Unknown);
    }

    #[test]
    fn hashes_the_content() {
        let seen = recognize("a.txt", b"abc");
        assert_eq!(seen.sha256, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(seen.size, 3);
    }

    #[test]
    fn kinds_and_media_types_go_both_ways() {
        for kind in KINDS {
            assert_eq!(Kind::from_media_type(kind.media_type()), kind);
        }
        assert_eq!(Kind::from_media_type("text/plain"), Kind::Unknown);
        assert!(Kind::Docx.readable() && !Kind::OldWord.readable() && !Kind::Jpeg.readable());
        assert!(Kind::Heic.is_image() && !Kind::Pdf.is_image());
    }
}
