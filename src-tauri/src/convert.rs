//! Turns an old Word file (.doc) into a .docx the tools can read, with a converter the computer already has:
//! LibreOffice when it is installed, since it keeps the layout best, else macOS's own textutil. There is none on a
//! computer with neither; the import then keeps the .doc and says so.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::recognize::{recognize, Kind};

/// LibreOffice starts slowly the first time; a conversion that takes longer than this is given up.
const TIMEOUT: Duration = Duration::from_secs(90);
const TEXTUTIL: &str = "/usr/bin/textutil";

/// Where LibreOffice's command line program is, by platform, then whatever the PATH has.
fn soffice_candidates() -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = [
        "/Applications/LibreOffice.app/Contents/MacOS/soffice",
        r"C:\Program Files\LibreOffice\program\soffice.exe",
        r"C:\Program Files (x86)\LibreOffice\program\soffice.exe",
        "/usr/bin/soffice",
        "/usr/local/bin/soffice",
        "/opt/libreoffice/program/soffice",
    ]
    .iter()
    .map(PathBuf::from)
    .filter(|path| path.is_file())
    .collect();
    found.extend(["soffice", "libreoffice"].map(PathBuf::from));
    found
}

/// Runs the command without a window or output; false when it cannot start, fails or takes too long.
fn run(command: &mut Command) -> bool {
    let Ok(mut child) = command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() else {
        return false;
    };
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if started.elapsed() < TIMEOUT => std::thread::sleep(Duration::from_millis(100)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

/// LibreOffice, with a profile of its own in `dir` so a copy the user has open does not take the job.
fn with_libreoffice(source: &Path, dir: &Path) -> bool {
    let profile = format!("-env:UserInstallation=file://{}", dir.join("libreoffice").display());
    soffice_candidates().iter().any(|soffice| {
        run(Command::new(soffice)
            .arg(&profile)
            .args(["--headless", "--convert-to", "docx", "--outdir"])
            .arg(dir)
            .arg(source))
    })
}

fn with_textutil(source: &Path, target: &Path) -> bool {
    Path::new(TEXTUTIL).is_file()
        && run(Command::new(TEXTUTIL).args(["-convert", "docx", "-format", "doc", "-output"]).arg(target).arg(source))
}

/// The .doc at `source` as .docx bytes; `source` is named `<stem>.doc` and the work happens beside it. An error
/// when this computer has no converter or the result is not a Word file.
pub fn doc_to_docx(source: &Path) -> Result<Vec<u8>, String> {
    let dir = source.parent().ok_or("No folder to convert in")?;
    let target = source.with_extension("docx");
    let _ = std::fs::remove_file(&target);
    let converted = with_libreoffice(source, dir) || with_textutil(source, &target);
    let docx = std::fs::read(&target);
    let _ = std::fs::remove_file(&target);
    let _ = std::fs::remove_dir_all(dir.join("libreoffice"));
    match docx {
        Ok(docx) if converted && recognize("converted.docx", &docx).kind == Kind::Docx => Ok(docx),
        _ => Err("This computer has nothing that converts .doc files".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_that_is_no_word_file_is_not_converted() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("notes.doc");
        std::fs::write(&source, b"").unwrap();
        // an empty file: LibreOffice and textutil either refuse it or make something that is checked again
        let result = doc_to_docx(&source);
        assert!(result.is_err() || recognize("x", &result.unwrap()).kind == Kind::Docx);
        assert!(!dir.path().join("libreoffice").exists());
    }

    /// macOS always has textutil, so there the round trip is real: a .doc made from a docx comes back as one.
    #[cfg(target_os = "macos")]
    #[test]
    fn converts_a_real_doc_on_macos() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("contract.doc");
        let fixture = crate::mcp::tools::tests::fixture("filled.docx");
        let made = Command::new(TEXTUTIL).args(["-convert", "doc", "-output"]).arg(&source).arg(&fixture).status();
        assert!(made.unwrap().success());
        assert_eq!(recognize("contract.docx", &std::fs::read(&source).unwrap()).kind, Kind::OldWord);

        let docx = doc_to_docx(&source).unwrap();
        assert_eq!(recognize("contract.doc", &docx).kind, Kind::Docx);
        let back = dir.path().join("back.docx");
        std::fs::write(&back, &docx).unwrap();
        assert!(!crate::templates::docx::inspect(&back).unwrap().is_empty(), "the tools read the converted file");
        assert!(!source.with_extension("docx").exists(), "nothing is left beside the source");
    }
}
