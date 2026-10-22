use super::document::{self, Document};
use super::types::Block;
use super::validate::*;
use super::*;

const EGN: &str = "8503151235";
const LNCH: &str = "1001001001";
/// Valid both as an EGN and as an LNCh.
const BOTH: &str = "8503150075";
const EIK: &str = "121212121";
const IBAN: &str = "BG80 BNBG 9661 1020 3456 78";

/// (kind, text) of every detection above the redaction threshold.
fn found(text: &str) -> Vec<(Kind, &str)> {
    detect(text).into_iter().filter(|d| d.score >= REDACT).map(|d| (d.kind, &text[d.start..d.end])).collect()
}

fn has(text: &str, kind: Kind, value: &str) {
    let all = found(text);
    assert!(all.contains(&(kind, value)), "{kind:?} {value:?} not in {all:?}");
}

#[test]
fn validators() {
    assert!(valid_egn(EGN));
    assert!(!valid_egn("9055121230"), "checksum holds but month 55 is no date");
    assert!(valid_lnch(LNCH) && !valid_lnch("1001001002"));
    assert!(valid_eik(EIK) && !valid_eik("121212122"));
    assert!(valid_vat_bg(&format!("BG{EIK}")));
    assert!(valid_ssn("123-45-6789") && !valid_ssn("666-45-6789") && !valid_ssn("123-00-6789"));
    assert_eq!(egn_birth_date(EGN).map(|d| d.to_string()).as_deref(), Some("1985-03-15"));
}

#[test]
fn t1_patterns_alone() {
    has(&format!("номер {EGN} е"), Kind::Egn, EGN);
    has(&format!("IBAN: {IBAN} BIC"), Kind::Iban, IBAN);
    has("пишете на john.example@example.com днес", Kind::Email, "john.example@example.com");
    has("обадете се на 0888 123 456", Kind::Phone, "0888 123 456");
    has("SSN 123-45-6789", Kind::Ssn, "123-45-6789");
    has("на 01.02.2024 г.", Kind::Date, "01.02.2024");
    has("цена 1 250,50 лв.", Kind::Money, "1 250,50");
    assert!(found("номер 123456789 на поръчката").is_empty(), "a bare 9-digit number is no PII");
}

#[test]
fn t2_labels_claim_and_delimit() {
    has(&format!("ЛНЧ {LNCH}"), Kind::Lnch, LNCH);
    has(&format!("ЕИК {EIK}, със седалище"), Kind::Eik, EIK);
    has("л.к. № 645123456, издадена на 12.03.2020 г. от МВР - София", Kind::IdCard, "645123456");
    has("л.к. № 645123456, издадена на 12.03.2020 г. от МВР - София", Kind::IdIssueDate, "12.03.2020");
    has("л.к. № 645123456, издадена на 12.03.2020 г. от МВР - София, с адрес", Kind::IdIssuer, "от МВР - София");
    has("роден на 15 март 1985 г.", Kind::BirthDate, "15 март 1985");
    has("тел. 02 123 4567, факс", Kind::Phone, "02 123 4567");
    has("Mr. John Example signed", Kind::Person, "John Example");
}

#[test]
fn addresses() {
    let text = format!("с постоянен адрес гр. София, ж.к. Младост 1, бл. 12, вх. А, ет. 3, ап. 7, ЕГН {EGN}");
    has(&text, Kind::Address, "гр. София, ж.к. Младост 1, бл. 12, вх. А, ет. 3, ап. 7");
    has("офис на гр. Пловдив, ул. „Примерна“ № 5; телефон", Kind::Address, "гр. Пловдив, ул. „Примерна“ № 5");
    has("residing at 12 Example Street, Springfield and", Kind::Address, "12 Example Street, Springfield");
    has("the office at 12 Example Street, Springfield", Kind::Address, "12 Example Street, Springfield");
}

#[test]
fn people_and_orgs() {
    let text = format!("1. Иван Петров Иванов, ЕГН {EGN}, наричан по-долу ПРОДАВАЧ");
    has(&text, Kind::Person, "Иван Петров Иванов");
    has(&format!("ИВАН ПЕТРОВ ИВАНОВ, с ЕГН {EGN}"), Kind::Person, "ИВАН ПЕТРОВ ИВАНОВ");
    has("представлявано от Мария Георгиева Димитрова, в качеството", Kind::Person, "Мария Георгиева Димитрова");
    has("„Пример Консулт“ ЕООД, ЕИК", Kind::Org, "„Пример Консулт“ ЕООД");
    has("ЕТ „Пример – Иван Петров“ със", Kind::Org, "ЕТ „Пример – Иван Петров“");
    has("Example Trading Ltd. agrees", Kind::Org, "Example Trading Ltd.");
    let text = "ПРОДАВАЧ: Иван Петров Иванов";
    has(text, Kind::Person, "Иван Петров Иванов");
}

#[test]
fn labels_beat_bare_patterns() {
    assert!(valid_egn(BOTH) && valid_lnch(BOTH));
    let d = detect(&format!("ЛНЧ {BOTH}"));
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].kind, Kind::Lnch, "the label decides between two valid readings");
    let d = detect(&format!("ЕГН {EGN}"));
    assert!(d[0].kind == Kind::Egn && d[0].score > 0.95);
    let d = detect("ЕГН 85031512X9");
    assert!(d.iter().any(|d| d.kind == Kind::Egn && d.score >= REDACT && d.score < 0.7), "unchecked: {d:?}");
}

fn analyze_plain(blocks: &[Block]) -> Analysis {
    analyze(blocks, &Corrections::default())
}

fn blocks(texts: &[&str]) -> Vec<Block> {
    texts.iter().map(|t| Block::plain(t)).collect()
}

#[test]
fn one_value_is_one_entity() {
    let a = analyze_plain(&blocks(&[
        "1. „Пример Инвест“ ЕООД, ЕИК 121212121, със седалище и адрес на управление: гр. София, район Лозенец, ул. „Примерна“ № 5, представлявано от управителя Иван Петров Иванов, наричано по-долу „ЗАЕМОДАТЕЛ“,",
        "2. Иван Петров Иванов, ЕГН .........., л.к. № .........., издадена на .......... от МВР – .........., в качеството си на физическо лице, наричан по-долу „ЗАЕМАТЕЛ“,",
        "ПРИМЕР ИНВЕСТ ЕООД",
        "ИВАН ПЕТРОВ ИВАНОВ",
    ]));
    let named: Vec<_> = a.entities.iter().filter(|e| e.norm == "иван петров иванов").collect();
    assert_eq!(named.len(), 1, "manager, borrower and signature are one value: {named:?}");
    assert_eq!(named[0].occurrences.len(), 3);
    let address: Vec<_> = a.entities.iter().filter(|e| e.kind == Kind::Address).collect();
    assert_eq!(address.len(), 1, "район keeps the address whole: {address:?}");
    assert!(address[0].surfaces[0].ends_with("№ 5"));
    assert!(!a.entities.iter().any(|e| e.kind == Kind::IdIssuer), "a blank after the label is no value");
    assert_eq!(a.entities.iter().filter(|e| e.kind == Kind::Org).count(), 1, "quotes and case do not split a company");
    let ids: Vec<&str> = a.entities.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids[..2], ["pii:1", "pii:2"], "numbered by first appearance: {ids:?}");
}

#[test]
fn entities_merge_spellings() {
    let a = analyze_plain(&blocks(&[
        &format!("ЕГН {EGN}"),
        "ЕГН 850315 1235",
        "Иван Петров Иванов, ЕГН 8503151235",
        "ИВАН ПЕТРОВ ИВАНОВ:",
    ]));
    let egn: Vec<_> = a.entities.iter().filter(|e| e.kind == Kind::Egn).collect();
    assert_eq!(egn.len(), 1);
    assert_eq!(egn[0].id, "pii:1");
    assert_eq!(egn[0].occurrences.len(), 3);
    assert_eq!(egn[0].surfaces, [EGN, "850315 1235"]);
}

// -- Document ----------------------------------------------------------------------------------------------

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde_json::json;

use crate::templates::docx as docx_file;

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn paragraph_xml(runs: &[&str]) -> String {
    let runs: String =
        runs.iter().map(|r| format!(r#"<w:r><w:t xml:space="preserve">{}</w:t></w:r>"#, escape(r))).collect();
    format!("<w:p>{runs}</w:p>")
}

/// A .docx whose body is the given paragraphs, each a list of runs, built on the fixture's package.
fn docx_with(dir: &Path, paragraphs: &[&[&str]]) -> PathBuf {
    docx_body(dir, &paragraphs.iter().map(|runs| paragraph_xml(runs)).collect::<String>())
}

fn docx_body(dir: &Path, body: &str) -> PathBuf {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/filled.docx");
    let mut archive = zip::ZipArchive::new(std::fs::File::open(fixture).unwrap()).unwrap();
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}</w:body></w:document>"#
    );
    let path = dir.join("doc.docx");
    let mut writer = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).unwrap();
        let name = entry.name().to_string();
        let mut data = Vec::new();
        entry.read_to_end(&mut data).unwrap();
        writer.start_file(name.clone(), zip::write::SimpleFileOptions::default()).unwrap();
        writer.write_all(if name == "word/document.xml" { xml.as_bytes() } else { &data }).unwrap();
    }
    writer.finish().unwrap();
    path
}

fn seller() -> String {
    format!("1. Иван Петров Иванов, ЕГН {EGN}, с адрес гр. София, ул. „Примерна“ № 5, наричан по-долу ПРОДАВАЧ.")
}

fn all_text(path: &Path) -> String {
    docx_file::inspect(path).unwrap().into_iter().map(|p| p.text).collect::<Vec<_>>().join("\n")
}

#[test]
fn redacted_paragraphs_hide_values() {
    let dir = tempfile::tempdir().unwrap();
    let seller = seller();
    let path = docx_with(dir.path(), &[&[&seller], &["ПРОДАВАЧ: ", "Иван Петров Иванов"]]);
    let doc = Document::docx(&path, &Corrections::default()).unwrap();
    let out = serde_json::to_string(&json!({"p": doc.redacted_paragraphs(), "pii": doc.entities()})).unwrap();
    for value in [EGN, "Иван", "Примерна"] {
        assert!(!out.contains(value), "{value} leaked: {out}");
    }
    let paragraphs = doc.redacted_paragraphs();
    assert!(paragraphs[0].text.contains("ЕГН ⟦pii:2 ##########⟧"), "{}", paragraphs[0].text);
    assert_eq!(paragraphs[1].runs, ["ПРОДАВАЧ: ", "⟦pii:1 Xxxx Xxxxxx Xxxxxx⟧"], "the shape of the value stays");
    let egn = doc.entities().into_iter().find(|e| e.id == "pii:2").unwrap();
    assert_eq!((egn.length, egn.occurrences), (10, 1));
    let out = serde_json::to_string(&doc.entities()).unwrap();
    assert!(!out.contains("kind") && !out.contains("role"), "only the assistant names what a value is: {out}");
}

#[test]
fn placeholders_resolve_to_tokenize_finds() {
    let dir = tempfile::tempdir().unwrap();
    let seller = seller();
    let path = docx_with(dir.path(), &[&[&seller], &["ПРОДАВАЧ: ", "Иван Петров Иванов"]]);
    let doc = Document::docx(&path, &Corrections::default()).unwrap();
    let finds = doc
        .resolve_finds(vec![
            json!({"find": "⟦pii:1 Xxxx Xxxxxx Xxxxxx⟧", "var": "seller_name"}),
            json!({"find": "ЕГН ⟦pii:2⟧", "var": "seller_egn_line"}),
            json!({"find": "ПРОДАВАЧ", "var": "role", "occurrence": 2}),
        ])
        .unwrap();
    assert_eq!(finds[0], json!({"find": "Иван Петров Иванов", "var": "seller_name"}));
    assert_eq!(finds[1], json!({"find": format!("ЕГН {EGN}"), "var": "seller_egn_line"}));
    assert_eq!(finds[2], json!({"find": "ПРОДАВАЧ", "var": "role", "occurrence": 2}), "plain finds pass unchanged");
    assert!(doc.resolve_finds(vec![json!({"find": "⟦pii:9⟧", "var": "x"})]).is_err());
    let target = dir.path().join("t.docx");
    docx_file::tokenize(&path, &target, &finds).unwrap();
    assert!(!all_text(&target).contains(EGN));
}

#[test]
fn values_across_runs_are_redacted_in_the_first_run() {
    let dir = tempfile::tempdir().unwrap();
    let path = docx_with(dir.path(), &[&["ЕГН 8503", "151235"]]);
    let doc = Document::docx(&path, &Corrections::default()).unwrap();
    assert!(doc.entities()[0].crosses_runs);
    assert_eq!(doc.redacted_paragraphs()[0].runs, ["ЕГН ⟦pii:1 ##########⟧", ""]);
    let html = doc.html(&[]).unwrap();
    assert_eq!(
        html.matches(r#"data-e="pii:1" data-n="0""#).count(),
        2,
        "one place over two runs keeps one number: {html}"
    );
}

#[test]
fn snap_and_guard() {
    let text = "Наемател: Иван Петров, гр. София";
    let s = text.find("ван").unwrap();
    let (a, b) = document::snap(text, s - 1, s + "ван Пет".len());
    assert_eq!(&text[a..b], "Иван Петров");
    let (a, b) = document::snap(text, text.find(':').unwrap(), text.find("Иван").unwrap() + 4);
    assert_eq!(&text[a..b], "Иван");
    assert_eq!(guard(&format!("id {EGN} and {IBAN}")), "id ⟦##########⟧ and ⟦XX## XXXX #### #### #### ##⟧");
    assert_eq!(guard("ticket 1730000000"), "ticket 1730000000");
    assert_eq!(redact_line(&format!("Договор с ЕГН {EGN}")), "Договор с ЕГН ⟦##########⟧");
}

fn pdf_with(dir: &Path, lines: &[&str]) -> Vec<u8> {
    let body: String = lines.iter().map(|l| format!("{l}\n\n")).collect();
    std::fs::write(dir.join("main.typ"), format!("#set page(width: 210mm, height: 297mm)\n{body}")).unwrap();
    crate::templates::typst::compile(dir, "{}", false).unwrap()
}

#[test]
fn pdf_pages_redact_lines_and_place_masks() {
    let dir = tempfile::tempdir().unwrap();
    let pdf = pdf_with(dir.path(), &[&format!("Наемател: Иван Петров Иванов, ЕГН {EGN}"), "Срок: 12 месеца"]);
    let doc = Document::pdf(&pdf, &Corrections::default()).unwrap();
    let pages = serde_json::to_string(&doc.redacted_pages()).unwrap();
    assert!(!pages.contains(EGN) && !pages.contains("Иван") && pages.contains("Срок"), "{pages}");
    let egn = doc.entities().into_iter().find(|e| e.id == "pii:2").unwrap();
    assert_eq!(egn.places.len(), 1);
    assert_eq!(doc.masks(1).len(), 2, "the name and the EGN");
    let png = crate::render::preview::grid_page(&pdf, 1, 80.0).unwrap();
    let masked = mask_png(&png, 841.9, &doc.masks(1), 80.0).unwrap();
    assert_ne!(png, masked);
}

#[test]
fn html_maps_runs_back_to_blocks() {
    let dir = tempfile::tempdir().unwrap();
    let table = format!("<w:tbl><w:tr><w:tc>{}</w:tc></w:tr></w:tbl>", paragraph_xml(&["ЕГН ", EGN]));
    let body = format!("{}{table}{}", paragraph_xml(&["Наемател: Иван Петров"]), paragraph_xml(&["Край"]));
    let path = docx_body(dir.path(), &body);
    let doc = Document::docx(&path, &Corrections::default()).unwrap();
    // read_body puts the table paragraph after both body paragraphs
    assert_eq!(doc.blocks()[2].text, format!("ЕГН {EGN}"));
    let html = doc.html(&[]).unwrap();
    let table_at = html.find("<table>").unwrap();
    let mark_at = html.find(r#"<mark class="pii" data-e="pii:2" data-n="0">"#).unwrap();
    assert!(mark_at > table_at, "the table paragraph renders in document order: {html}");
    assert!(html.contains(r#"<span data-b="2" data-o="4">"#), "the run after \"ЕГН \" starts at 4: {html}");
    assert!(html.contains(r#"<span data-b="1" data-o="0">Край</span>"#), "{html}");
    assert!(html.find("Край").unwrap() > table_at);
}

#[test]
fn corrections_hide_and_show() {
    let text = format!("Наемател: Иван Петров, ЕГН {EGN}, клубна карта 77-ABC, стая 12");
    let mut c = Corrections::default();
    c.hide("клубна карта 77-ABC", true);
    c.show("Иван Петров");
    let a = analyze(&blocks(&[&text, "КЛУБНА КАРТА 77-ABC"]), &c);
    let kinds: Vec<(Kind, usize)> = a.entities.iter().map(|e| (e.kind, e.occurrences.len())).collect();
    assert!(kinds.contains(&(Kind::Marked, 2)), "hidden in any letter case: {kinds:?}");
    assert!(!kinds.iter().any(|(k, _)| *k == Kind::Person), "shown: {kinds:?}");
    assert!(kinds.contains(&(Kind::Egn, 1)));

    c.show("КЛУБНА КАРТА 77-ABC");
    assert!(c.hide.is_empty() && c.show == ["Иван Петров", "КЛУБНА КАРТА 77-ABC"], "shown values stay listed: {c:?}");
    c.hide("Иван Петров", false);
    assert!(c.hide.is_empty() && c.show == ["КЛУБНА КАРТА 77-ABC"], "a detected value only stops being shown");
    c.forget("клубна карта 77-abc");
    assert!(c.is_empty());
}

#[test]
fn review_selection_and_hidden_list() {
    let dir = tempfile::tempdir().unwrap();
    let path = docx_with(dir.path(), &[&["Членска карта: ", "НК-4471", " от 2020 г."]]);
    let doc = Document::docx(&path, &Corrections::default()).unwrap();
    assert!(doc.hidden().is_empty());
    let text = &doc.blocks()[0].text;
    let at = text.find("НК-").unwrap();
    let utf16 = |byte: usize| text[..byte].encode_utf16().count();
    assert_eq!(doc.selection(0, utf16(at + 2), utf16(at + 6)).as_deref(), Some("НК-4471"));
    let mut c = Corrections::default();
    c.hide("НК-4471", true);
    let doc = Document::docx(&path, &c).unwrap();
    assert_eq!(doc.hidden()[0]["id"], "pii:1");
    assert_eq!(doc.redacted_paragraphs()[0].runs, ["Членска карта: ", "⟦pii:1 XX-####⟧", " от 2020 г."]);
    assert!(doc.html(&[]).unwrap().contains(r#"<mark class="pii" data-e="pii:1" data-n="0">НК-4471</mark>"#));
}

#[test]
fn review_places_are_numbered_and_located() {
    let dir = tempfile::tempdir().unwrap();
    let path = docx_with(
        dir.path(),
        &[&[&format!("ЕГН {EGN}")], &["Стая 12, карта 77-ABC"], &[&format!("отново {EGN}, карта 77-ABC")]],
    );
    let mut c = Corrections::default();
    c.show("77-ABC");
    let doc = Document::docx(&path, &c).unwrap();
    let html = doc.html(&c.show).unwrap();
    assert!(html.contains(r#"data-e="pii:1" data-n="1""#), "{html}");
    assert!(html.contains(r#"class="pii shown" data-v="0" data-n="1""#), "{html}");
    assert_eq!(doc.hidden()[0]["first"], json!([0, 7]));
    assert_eq!(doc.first("77-ABC"), Some((1, "Стая 12, карта ".len())));
}

#[test]
fn corrections_ignore_quotes() {
    let texts = ["1. „ПРИМЕР ИНВЕСТ“ ЕООД, ЕИК 121212121", "ПРИМЕР ИНВЕСТ ЕООД"];
    // a selection that left out the opening quote still hides every spelling
    let mut c = Corrections::default();
    c.hide("ПРИМЕР ИНВЕСТ“ ЕООД", true);
    let a = analyze(&blocks(&texts), &c);
    let company: Vec<_> = a.entities.iter().filter(|e| e.norm == "пример инвест еоод").collect();
    assert_eq!(company.len(), 1, "{company:?}");
    assert_eq!(company[0].occurrences.len(), 2);
    assert!(
        company[0].surfaces.contains(&"„ПРИМЕР ИНВЕСТ“ ЕООД".to_string()),
        "the quotes are hidden too: {company:?}"
    );
    // showing one spelling shows them all
    let mut c = Corrections::default();
    c.show("ПРИМЕР ИНВЕСТ ЕООД");
    let a = analyze(&blocks(&texts), &c);
    assert!(!a.entities.iter().any(|e| e.norm == "пример инвест еоод"), "{:?}", a.entities);
    // and the two are one correction
    c.hide("„ПРИМЕР ИНВЕСТ“ ЕООД", false);
    assert!(c.is_empty());
}

#[test]
fn removing_values() {
    let text = format!("Наемател: Иван Петров, ЕГН {EGN}");
    let mut c = Corrections::default();
    c.hide("стая 12", true);
    c.remove("СТАЯ 12");
    assert!(c.is_empty(), "an added value is forgotten");
    c.remove("Иван Петров");
    assert_eq!(c.ignore, ["Иван Петров"], "a detected value is dismissed");
    let a = analyze(&blocks(&[&text]), &c);
    assert!(!a.entities.iter().any(|e| e.norm == "иван петров"));
    c.hide("Иван Петров", false);
    assert!(c.is_empty(), "hiding it again undoes the dismissal");
}

#[test]
fn capitals_are_names_only_from_three_letters() {
    assert!(
        found("решение на ЕДНОЛИЧНИЯ СОБСТВЕНИК НА КАПИТАЛА от днес").is_empty(),
        "{:?}",
        found("решение на ЕДНОЛИЧНИЯ СОБСТВЕНИК НА КАПИТАЛА от днес")
    );
    has("Собственик: ИВАН ПЕТРОВ", Kind::Person, "ИВАН ПЕТРОВ");
}
