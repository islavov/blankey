//! The engine against an invented config set (`testdata/example`): a member card read by its labels and a permit
//! read by its printed codes. The sets of real documents and their tests are not part of the app.

use std::collections::BTreeMap;

use super::*;

/// The example set as the relay would serve one: path -> YAML.
pub fn example_set_files() -> BTreeMap<String, String> {
    [
        ("set.yaml", include_str!("testdata/example/set.yaml")),
        ("tables/example_codes.yaml", include_str!("testdata/example/tables/example_codes.yaml")),
        ("documents/example_member_card.yaml", include_str!("testdata/example/documents/example_member_card.yaml")),
        ("documents/example_permit.yaml", include_str!("testdata/example/documents/example_permit.yaml")),
    ]
    .into_iter()
    .map(|(path, text)| (path.to_string(), text.to_string()))
    .collect()
}

fn example() -> Set {
    Set::parse("test", &example_set_files()).unwrap()
}

/// A page of lines, each (text, x, y); a line is 14 high and 9 wide a character.
fn page(lines: &[(&str, f64, f64)]) -> Page {
    let lines = lines.iter().map(|(text, x, y)| Line {
        text: text.to_string(),
        rect: [*x, *y, x + 9.0 * text.chars().count() as f64, y + 14.0],
        confidence: 0.95,
    });
    Page { width: 600.0, height: 400.0, lines: lines.collect(), turned: 0, skew: 0.0 }
}

fn member_card() -> Page {
    page(&[
        ("EXAMPLE LIBRARY", 40.0, 20.0),
        ("Member card", 40.0, 40.0),
        ("Member name JANE EXAMPLE", 40.0, 80.0),
        ("Member no M12O456", 40.0, 100.0),
        ("Personal no 7501020018", 40.0, 120.0),
        ("Born 02.01.1975", 40.0, 140.0),
        ("Town СПРИНГФИЙЛД/SPRINGFIELD", 40.0, 160.0),
        ("Address", 40.0, 180.0),
        ("12 OAK STREET", 40.0, 200.0),
        ("FLOOR 3", 40.0, 220.0),
    ])
}

fn permit() -> Page {
    page(&[
        ("EXAMPLE PERMIT OFFICE", 40.0, 20.0),
        ("(A) P-1234", 40.0, 60.0),
        ("(B) 14.03.2008", 40.0, 80.0),
        ("(C.1)", 40.0, 100.0),
        ("ПРИМЕР ЕООД", 110.0, 100.0),
        ("PRIMER EOOD", 110.0, 111.0),
        ("(D) 1732 kg", 40.0, 140.0),
        ("(E)", 40.0, 160.0),
        ("(a) OWNER", 110.0, 160.0),
    ])
}

fn value<'a>(reading: &'a Reading, key: &str) -> Option<(&'a str, Status)> {
    reading.fields.iter().find(|f| f.key == key).map(|f| (f.value.as_str(), f.status))
}

#[test]
fn a_set_parses_and_leaves_out_what_this_engine_cannot_run() {
    let set = example();
    assert_eq!(
        set.documents.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(),
        ["example_member_card", "example_permit"]
    );
    assert!(set.skipped.is_empty(), "{:?}", set.skipped);
    assert_eq!(set.ocr.zoom, 2, "a setting of a newer engine beside it is ignored");

    let with = |path: &str, text: &str| {
        let mut files = example_set_files();
        files.insert(path.to_string(), text.to_string());
        files
    };
    let member = include_str!("testdata/example/documents/example_member_card.yaml");
    let cases = [
        (member.replace("id: example_member_card", "id: later\nengine: 2"), "later", "engine 2"),
        (
            member.replace("id: example_member_card", "id: other\nlocator_of_a_newer_engine: 1"),
            "documents/zz.yaml",
            "unknown field",
        ),
        (member.replace("id: example_member_card", "id: other").replace("member_no", "no_such"), "other", "pattern"),
        (
            member.replace("id: example_member_card", "id: other").replace("type: date", "type: moon"),
            "other",
            "unknown type",
        ),
        (
            member.replace("id: example_member_card", "id: other").replace("number: member.number", "number: nothing"),
            "other",
            "not a field",
        ),
        (member.replace("id: example_member_card", "id: other").replace("{name}", "{nobody}"), "other", "nobody"),
        (member.replace("id: example_member_card", "id: other").replace("key: town", "key: name"), "other", "twice"),
        (member.replace("id: example_member_card", "id: other\nuses: no_table"), "other", "no table"),
        (member.to_string(), "example_member_card", "twice"),
    ];
    for (text, named, reason) in cases {
        let set = Set::parse("test", &with("documents/zz.yaml", &text)).unwrap();
        assert_eq!(set.documents.len(), 2, "{reason}: the rest of the set works");
        let [(name, why)] = set.skipped.as_slice() else { panic!("{reason}: {:?}", set.skipped) };
        assert_eq!(name, named, "{why}");
        assert!(why.contains(reason), "{why}");
    }

    // a pattern that does not compile takes the documents that use it with it
    let set = Set::parse("test", &with("set.yaml", &example_set_files()["set.yaml"].replace("'P[0-9]{4}'", "'P[0-9'")))
        .unwrap();
    assert_eq!(set.documents.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(), ["example_member_card"]);
    assert_eq!(set.skipped.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>(), ["permit_no", "example_permit"]);

    // the set file itself is all or nothing
    let set_yaml = &example_set_files()["set.yaml"];
    assert!(Set::parse("test", &with("set.yaml", &set_yaml.replace("engine: 1", "engine: 2")))
        .is_err_and(|e| e.contains("engine 2")));
    assert!(
        Set::parse("test", &with("set.yaml", &set_yaml.replace("  zoom: 2\n", ""))).is_err(),
        "every setting is needed"
    );
    assert!(Set::parse("test", &BTreeMap::new()).is_err());
}

#[test]
fn a_document_is_read_by_its_labels() {
    let reading = read(&example(), &[member_card()], "en").unwrap();
    assert_eq!((reading.doctype.as_str(), reading.kind.as_str()), ("example_member_card", "person"));
    assert_eq!(reading.sides, ["front"]);
    assert_eq!(reading.document, Text { en: "Example member card".into(), bg: "Примерна членска карта".into() });
    assert_eq!(value(&reading, "name"), Some(("JANE EXAMPLE", Status::Unchecked)));
    assert_eq!(value(&reading, "member.number"), Some(("M120456", Status::Ok)), "the pattern repairs the letter O");
    assert_eq!(value(&reading, "egn"), Some(("7501020018", Status::Ok)));
    assert_eq!(value(&reading, "birth_date"), Some(("02.01.1975", Status::Ok)));
    assert_eq!(
        value(&reading, "town"),
        Some(("СПРИНГФИЙЛД", Status::Unchecked)),
        "the first half of a bilingual value"
    );
    assert_eq!(value(&reading, "address"), Some(("12 OAK STREET, FLOOR 3", Status::Unchecked)));
    assert_eq!((reading.name.as_str(), reading.number.as_deref()), ("JANE EXAMPLE", Some("M120456")));
    let number = reading.fields.iter().find(|f| f.key == "member.number").unwrap();
    assert_eq!(number.label, "Member number");
    assert_eq!(number.place.map(|(page, _)| page), Some(0), "where it was read goes to the check sheet");

    let bulgarian = read(&example(), &[member_card()], "bg").unwrap();
    assert_eq!(bulgarian.fields.iter().find(|f| f.key == "egn").unwrap().label, "ЕГН");
}

#[test]
fn a_value_that_fails_its_check_is_marked_and_names_nothing() {
    let mut card = member_card();
    for line in &mut card.lines {
        line.text = line.text.replace("02.01.1975", "03.01.1975").replace("M12O456", "M12");
    }
    let reading = read(&example(), &[card], "en").unwrap();
    assert_eq!(value(&reading, "egn"), Some(("7501020018", Status::Failed)), "at odds with the birth date");
    assert_eq!(value(&reading, "birth_date"), Some(("03.01.1975", Status::Failed)));
    assert_eq!(value(&reading, "member.number").map(|(_, status)| status), Some(Status::Failed));
    assert_eq!(reading.number, None);
}

#[test]
fn a_document_is_read_by_its_printed_codes() {
    let reading = read(&example(), &[permit()], "en").unwrap();
    assert_eq!(
        (reading.doctype.as_str(), reading.kind.as_str(), reading.family.as_str()),
        ("example_permit", "vehicle", "example_permit")
    );
    assert_eq!(value(&reading, "permit.number"), Some(("P1234", Status::Ok)));
    assert_eq!(value(&reading, "permit.issued_on"), Some(("14.03.2008", Status::Ok)));
    assert_eq!(value(&reading, "holder.name"), Some(("ПРИМЕР ЕООД", Status::Unchecked)), "the value beside its code");
    assert_eq!(value(&reading, "holder.name_latin"), Some(("PRIMER EOOD", Status::Unchecked)), "the line under it");
    assert_eq!(value(&reading, "weight"), Some(("1732", Status::Ok)));
    assert_eq!(
        value(&reading, "holder.status"),
        Some(("(a) OWNER", Status::Unchecked)),
        "a lettered item is its value"
    );
    assert_eq!(value(&reading, "remarks"), None, "a code of the table this document does not carry");
    let weight = reading.fields.iter().find(|f| f.key == "weight").unwrap();
    assert_eq!(weight.label, "Weight (D)", "the printed code stays on the label");
    assert_eq!(reading.name, "P1234 ПРИМЕР ЕООД");
}

#[test]
fn the_document_is_picked_among_all_that_could_be_it() {
    let set = example();
    assert!(read(
        &set,
        &[page(&[("LEASE AGREEMENT", 40.0, 20.0), ("(A) landlord and (B) tenant, member", 40.0, 60.0)])],
        "en"
    )
    .is_none());
    assert!(read(&set, &[], "en").is_none());

    // a side that is recognized and holds nothing
    let rules = read(
        &set,
        &[page(&[("Rules of the Example Library", 40.0, 20.0), ("Member name is printed overleaf", 40.0, 60.0)])],
        "en",
    )
    .unwrap();
    assert_eq!((rules.doctype.as_str(), rules.sides.as_slice()), ("example_member_card", &["rules".to_string()][..]));
    assert!(rules.fields.is_empty());
    assert_eq!(rules.name, "Example member card");

    // the pages of one file are the sides of one document
    let both = read(&set, &[member_card(), page(&[("Rules of the Example Library", 40.0, 20.0)])], "en").unwrap();
    assert_eq!(both.sides, ["front", "rules"]);

    // a permit with code F printed is another document: `codes_none` keeps this one off it
    let mut other = permit();
    other.lines.push(page(&[("(F) none", 40.0, 180.0)]).lines.remove(0));
    assert!(read(&set, &[other], "en").is_none());

    let member = include_str!("testdata/example/documents/example_member_card.yaml");
    let rival = |id: &str, extra: &str, matches: &str| {
        let text = member
            .replace("id: example_member_card", &format!("id: {id}\n{extra}"))
            .replace("{ text_all: [example library], text_any: [member card, членска карта] }", matches);
        let mut files = example_set_files();
        files.insert(format!("documents/{id}.yaml"), text);
        Set::parse("test", &files).unwrap()
    };
    let picked = |set: &Set| read(set, &[member_card()], "en").unwrap().doctype;
    assert_eq!(
        picked(&rival("aaa_loose", "", "{ text_all: [example library] }")),
        "example_member_card",
        "the one that asks for more"
    );
    assert_eq!(
        picked(&rival("zzz_same", "", "{ text_all: [example library], text_any: [member card] }")),
        "example_member_card",
        "the first of equals"
    );
    assert_eq!(
        picked(&rival("zzz_first", "priority: 5", "{ text_all: [example library] }")),
        "zzz_first",
        "priority goes before both"
    );
}
