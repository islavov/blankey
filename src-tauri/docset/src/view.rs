//! The reading on top of the picture, as one HTML file: every line the reader found, the fields where they were
//! read, and beside it how the document was picked and what each field's locators tried. The file holds the picture
//! and everything read from it, so it is for public specimens.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::Path;

use base64::Engine as _;
use blankey_doctypes::ocr::{self, OcrSettings};
use blankey_doctypes::{self as doctypes, Page};
use serde_json::{json, Value};

use crate::{Sheet, Tool};

const TEMPLATE: &str = include_str!("view.html");
const DEFAULT_PORT: u16 = 4173;

/// The picture as the page shows it: a data URI, and the size a browser lays it out at.
fn picture(path: &Path) -> Option<(String, (f64, f64))> {
    let content = std::fs::read(path).ok()?;
    let (width, height) = ocr::shown_size(&content)?;
    let kind = match path.extension()?.to_string_lossy().to_lowercase().as_str() {
        "png" => "image/png",
        "webp" => "image/webp",
        _ => "image/jpeg",
    };
    let encoded = base64::engine::general_purpose::STANDARD.encode(&content);
    Some((format!("data:{kind};base64,{encoded}"), (width as f64, height as f64)))
}

/// The report's data: the pages with their pictures, and the explained reading with every box moved from the
/// upright, levelled picture the lines were read in to the picture as it is shown.
fn data(tool: &Tool, set: &doctypes::Set, sheets: &[Sheet]) -> Value {
    let pages: Vec<Page> = sheets.iter().map(|sheet| sheet.page.clone()).collect();
    let pictures: Vec<Option<(String, (f64, f64))>> =
        sheets.iter().map(|sheet| sheet.picture.as_deref().and_then(picture)).collect();
    let shown = |page: usize, rect: [f64; 4]| match &pictures[page] {
        Some((_, (width, height))) => ocr::shown_rect(rect, *width, *height, pages[page].turned, pages[page].skew),
        None => rect,
    };
    let mut told = doctypes::explain(set, &pages, "en", tool.option("--document"));
    for (page, scan) in told.scans.iter_mut().enumerate() {
        scan.lines.iter_mut().for_each(|line| line.rect = shown(page, line.rect));
    }
    for tried in told.fields.iter_mut().flat_map(|field| &mut field.tries) {
        let candidates = tried.attempts.iter_mut().flat_map(|attempt| &mut attempt.candidates);
        candidates.for_each(|candidate| candidate.rect = candidate.rect.map(|rect| shown(tried.page, rect)));
    }
    for field in told.reading.iter_mut().flat_map(|reading| &mut reading.fields) {
        field.place = field.place.map(|(page, rect)| (page, shown(page, rect)));
    }
    let described = sheets.iter().zip(&pictures).enumerate().map(|(at, (sheet, picture))| {
        let (width, height) = picture.as_ref().map_or((sheet.page.width, sheet.page.height), |(_, size)| *size);
        let lines: Vec<Value> = sheet
            .page
            .lines
            .iter()
            .map(|line| json!({"text": line.text, "rect": shown(at, line.rect), "confidence": line.confidence}))
            .collect();
        json!({
            "name": sheet.name,
            "width": width,
            "height": height,
            "image": picture.as_ref().map(|(uri, _)| uri),
            "lines": lines,
        })
    });
    json!({ "pages": described.collect::<Vec<_>>(), "explained": told })
}

fn page(data: &Value) -> String {
    // "</" would end the script the data sits in
    TEMPLATE.replace("/*DATA*/", &data.to_string().replace("</", "<\\/"))
}

/// What changes when a file of the set is edited: the names, sizes and times of its YAML files.
fn version(dir: &Path) -> String {
    fn walk(dir: &Path, hasher: &mut DefaultHasher) {
        let mut entries: Vec<_> = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                if path.file_name().is_some_and(|name| name != "tests") {
                    walk(&path, hasher);
                }
            } else if path.extension().is_some_and(|e| e == "yaml") {
                let meta = std::fs::metadata(&path).ok();
                path.hash(hasher);
                meta.as_ref().map(|m| m.len()).hash(hasher);
                meta.and_then(|m| m.modified().ok()).hash(hasher);
            }
        }
    }
    let mut hasher = DefaultHasher::new();
    walk(dir, &mut hasher);
    format!("{:x}", hasher.finish())
}

/// The pages named, read with these settings.
struct Loaded {
    settings: OcrSettings,
    sheets: Vec<Sheet>,
}

/// Serves the report and builds it anew on every load, so the page follows the set as it is edited. The pictures
/// are read again only when the `ocr:` settings changed.
fn watch(tool: &Tool, port: u16) -> Result<i32, String> {
    let listener = TcpListener::bind(("127.0.0.1", port)).map_err(|e| format!("port {port}: {e}"))?;
    println!("http://127.0.0.1:{port}/  (Ctrl-C to stop)");
    let mut loaded: Option<Loaded> = None;
    for stream in listener.incoming().flatten() {
        let mut stream = stream;
        let mut request = String::new();
        if BufReader::new(&stream).read_line(&mut request).is_err() {
            continue;
        }
        let path = request.split_whitespace().nth(1).unwrap_or("/");
        let (kind, body) = if path == "/version" {
            ("text/plain", version(&tool.dir))
        } else if path == "/" {
            let version = version(&tool.dir);
            let built = tool.set().and_then(|set| {
                if loaded.as_ref().is_none_or(|kept| kept.settings != set.ocr) {
                    loaded = Some(Loaded { sheets: tool.sheets(&set.ocr)?, settings: set.ocr.clone() });
                }
                let sheets = &loaded.as_ref().ok_or("nothing read")?.sheets;
                Ok(data(tool, &set, sheets))
            });
            let mut data = built.unwrap_or_else(|error| json!({ "pages": [], "error": error }));
            data["watch"] = json!(true);
            data["version"] = json!(version);
            ("text/html; charset=utf-8", page(&data))
        } else {
            ("text/plain", String::new())
        };
        let head = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: {kind}\r\ncontent-length: {}\r\ncache-control: no-store\r\nconnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(head.as_bytes()).and_then(|_| stream.write_all(body.as_bytes()));
    }
    Ok(0)
}

pub fn command(tool: &Tool) -> Result<i32, String> {
    if tool.flag("--watch") {
        let port = tool.option("--port").map(|port| port.parse::<u16>().map_err(|e| format!("--port: {e}")));
        return watch(tool, port.transpose()?.unwrap_or(DEFAULT_PORT));
    }
    let set = tool.set()?;
    let sheets = tool.sheets(&set.ocr)?;
    let file = match tool.option("-o") {
        Some(file) => std::path::PathBuf::from(file),
        None => {
            let folder = tool.dir.join("tests/.view");
            std::fs::create_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
            let names: Vec<&str> = sheets.iter().map(|sheet| sheet.name.as_str()).collect();
            folder.join(format!("{}.html", names.join("+")))
        }
    };
    std::fs::write(&file, page(&data(tool, &set, &sheets))).map_err(|e| format!("{}: {e}", file.display()))?;
    println!("{}", file.display());
    Ok(0)
}
