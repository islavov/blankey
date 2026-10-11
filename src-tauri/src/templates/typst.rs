//! Typst templates: `main.typ` reads `json(bytes(sys.inputs.data))`. Fonts: the bundled Noto Sans family, the
//! system fonts and Typst's own; packages come from the local package dirs or Typst Universe.

use std::path::Path;
use std::sync::{Arc, LazyLock};

use typst::diag::{FileResult, SourceDiagnostic, Warned};
use typst::foundations::{Bytes, Datetime, Dict, Duration, IntoValue};
use typst::syntax::{FileId, RootedPath, Source, VirtualPath, VirtualRoot};
use typst::text::{Font, FontBook};
use typst::utils::LazyHash;
use typst::{Library, LibraryExt, World};
use typst_kit::downloader::SystemDownloader;
use typst_kit::files::{FileStore, FsRoot, SystemFiles};
use typst_kit::fonts::{self as kit_fonts, FontStore};
use typst_kit::packages::SystemPackages;

use crate::render::fonts;

static FONTS: LazyLock<Arc<FontStore>> = LazyLock::new(|| {
    let mut store = FontStore::new();
    for data in [fonts::REGULAR, fonts::BOLD, fonts::MONO] {
        for font in Font::iter(Bytes::new(data)) {
            let info = font.info().clone();
            store.push((font, info));
        }
    }
    store.extend(kit_fonts::system());
    store.extend(kit_fonts::embedded());
    Arc::new(store)
});

struct TemplateWorld {
    library: LazyHash<Library>,
    fonts: Arc<FontStore>,
    files: FileStore<SystemFiles>,
    main: FileId,
}

impl World for TemplateWorld {
    fn library(&self) -> &LazyHash<Library> {
        &self.library
    }

    fn book(&self) -> &LazyHash<FontBook> {
        self.fonts.book()
    }

    fn main(&self) -> FileId {
        self.main
    }

    fn source(&self, id: FileId) -> FileResult<Source> {
        self.files.source(id)
    }

    fn file(&self, id: FileId) -> FileResult<Bytes> {
        self.files.file(id)
    }

    fn font(&self, index: usize) -> Option<Font> {
        self.fonts.font(index)
    }

    fn today(&self, offset: Option<Duration>) -> Option<Datetime> {
        let now = chrono::Local::now();
        let date = match offset {
            None => now.date_naive(),
            Some(offset) => (chrono::Utc::now() + chrono::Duration::seconds(offset.seconds() as i64)).date_naive(),
        };
        use chrono::Datelike;
        Datetime::from_ymd(date.year(), date.month() as u8, date.day() as u8)
    }
}

fn describe(diagnostics: &[SourceDiagnostic]) -> String {
    diagnostics
        .iter()
        .map(|d| {
            let mut text = d.message.to_string();
            for hint in &d.hints {
                text.push_str(&format!(" (hint: {})", hint.v));
            }
            text
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// Compiles `main.typ` in `root` to PDF with `data` (JSON text) as `sys.inputs.data`.
pub fn compile(root: &Path, data: &str) -> Result<Vec<u8>, String> {
    let mut inputs = Dict::new();
    inputs.insert("data".into(), data.into_value());
    let library = Library::builder().with_inputs(inputs).build();
    let downloader = SystemDownloader::new(concat!("blankey/", env!("CARGO_PKG_VERSION")));
    let files = SystemFiles::new(FsRoot::new(root.to_path_buf()), SystemPackages::new(downloader));
    let main =
        RootedPath::new(VirtualRoot::Project, VirtualPath::new("/main.typ").map_err(|e| format!("{e:?}"))?).intern();
    let world =
        TemplateWorld { library: LazyHash::new(library), fonts: FONTS.clone(), files: FileStore::new(files), main };
    let Warned { output, .. } = typst::compile::<typst_layout::PagedDocument>(&world);
    let document = output.map_err(|errors| format!("Typst: {}", describe(&errors)))?;
    typst_pdf::pdf(&document, &typst_pdf::PdfOptions::default())
        .map_err(|errors| format!("Typst PDF: {}", describe(&errors)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiles_with_inputs_and_cyrillic() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("main.typ"),
            "#let d = json(bytes(sys.inputs.data))\n#set text(font: \"Noto Sans\")\nИме: #d.name\n",
        )
        .unwrap();
        let pdf = compile(dir.path(), r#"{"name": "Мария Иванова"}"#).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        let text = crate::render::preview::text(&pdf).unwrap();
        assert!(text.contains("Мария Иванова"), "{text}");
    }

    #[test]
    fn reports_errors() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("main.typ"), "#nope()").unwrap();
        assert!(compile(dir.path(), "{}").unwrap_err().contains("unknown variable"));
    }
}
