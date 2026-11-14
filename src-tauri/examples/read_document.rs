//! Reads image files as known documents on this machine and prints what was found: for seeing why a card or a
//! certificate did not read. Every capital letter and digit is printed as a dot, so names, numbers and dates do not
//! show; `--show` prints them as read.
//!
//!     cargo run --example read_document -- --set <set folder> [--show] [--lines] [--lines-json] [--together] <image>...
//!
//! The set folder is a document config set as the relay keeps it. `--lines` also prints each text line with its
//! place and how sure the reader was; `--lines-json` prints the page's lines as a `tests/lines/` file of a set
//! instead (values and all: for public specimens); `--together` reads the images as the sides of one document.

use blankey_lib::doctypes::{self, ocr, Page, Set};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| args.iter().any(|a| a == name);
    let (show, lines, together, json) = (flag("--show"), flag("--lines"), flag("--together"), flag("--lines-json"));
    let folder = args.iter().position(|a| a == "--set").and_then(|at| args.get(at + 1));
    let set = match folder.map(|dir| Set::from_dir(std::path::Path::new(dir))) {
        Some(Ok(set)) => set,
        Some(Err(e)) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
        None => {
            eprintln!("Name the document config set: --set <folder>");
            std::process::exit(1);
        }
    };
    let masked = |text: &str| -> String {
        if show {
            return text.to_string();
        }
        text.chars().map(|c| if c.is_uppercase() || c.is_ascii_digit() { '•' } else { c }).collect()
    };
    let Some(dir) = ocr::model_dir() else {
        eprintln!("The OCR models are missing: run scripts/fetch-ocr.sh");
        std::process::exit(1);
    };
    let engine = ocr::Engine::load(&dir).expect("OCR models");
    let mut pages: Vec<Page> = Vec::new();
    for path in args.iter().filter(|a| !a.starts_with("--") && Some(*a) != folder) {
        let started = std::time::Instant::now();
        let page =
            match std::fs::read(path).map_err(|e| e.to_string()).and_then(|content| engine.read(&content, &set.ocr)) {
                Ok(page) => page,
                Err(error) => {
                    println!("\n{path}: {error}");
                    continue;
                }
            };
        if json {
            println!("{}", serde_json::to_string_pretty(&page).expect("lines"));
            continue;
        }
        println!("\n{path}: {} x {}, {} lines in {:.1?}", page.width, page.height, page.lines.len(), started.elapsed());
        if lines {
            for line in &page.lines {
                let [x0, y0, _, y1] = line.rect;
                println!("  y {y0:5.0} x {x0:5.0} h {:3.0}  {:.2}  {}", y1 - y0, line.confidence, masked(&line.text));
            }
        }
        if !together {
            report(&set, &[page], &masked);
        } else {
            pages.push(page);
        }
    }
    if together {
        report(&set, &pages, &masked);
    }
}

fn report(set: &Set, pages: &[Page], masked: &dyn Fn(&str) -> String) {
    let Some(reading) = doctypes::read(set, pages, "en") else {
        println!("  not a known document");
        return;
    };
    println!(
        "  {} {:?}: {} fields, number {}",
        reading.doctype,
        reading.sides,
        reading.fields.len(),
        if reading.number.is_some() { "read" } else { "not read" }
    );
    for field in &reading.fields {
        println!("    {:<22} {:<10} {}", field.key, format!("{:?}", field.status), masked(&field.value));
    }
}
