//! The machine-readable zone of an ID card (ICAO 9303, size TD1): three lines of 30 characters in a fixed layout,
//! where the document number, the dates and the whole zone each carry a check digit. A misread is caught, not kept.

use std::sync::LazyLock;

use regex::Regex;

use super::patterns::digit;
use super::{fold, Line};
use crate::values::{egn_birth_date, valid_egn};

/// One value of the zone. `checked` is whether its check digit holds; None when it has none.
pub struct Part {
    pub name: &'static str,
    pub value: String,
    pub checked: Option<bool>,
    /// the line it is on, among the page's lines
    pub line: usize,
}

pub struct Mrz {
    /// the state that issued the document, as its first line names it: "BGR"
    pub state: String,
    pub parts: Vec<Part>,
    /// the page's lines the zone takes
    pub lines: Vec<usize>,
}

static SECOND: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[0-9OQDISBZG]{7}[MFX<][0-9OQDISBZG]{7}[A-Z<]{3}").unwrap());
/// The first line as a whole: capitals, digits and fillers, ending in a run of fillers.
static FIRST: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Z0-9<]{16,34}$").unwrap());

/// The text as the zone prints it: capitals, digits and the filler, which OCR reads as all kinds of brackets.
fn clean(text: &str) -> String {
    let spaced = fold::latin(text).to_uppercase();
    spaced.chars().filter(|c| !c.is_whitespace()).map(|c| if c.is_alphanumeric() { c } else { '<' }).collect()
}

fn digits(text: &str) -> String {
    text.chars().map(digit).collect()
}

/// The check digit of ICAO 9303: weights 7, 3, 1 over digits, letters as 10..35 and the filler as 0.
fn check(data: &str, expected: char) -> bool {
    let value = |c: char| match c {
        '0'..='9' => Some(c as u32 - '0' as u32),
        'A'..='Z' => Some(c as u32 - 'A' as u32 + 10),
        '<' => Some(0),
        _ => None,
    };
    let sum: Option<u32> = data.chars().zip([7, 3, 1].into_iter().cycle()).map(|(c, w)| value(c).map(|v| v * w)).sum();
    sum.is_some_and(|sum| digit(expected).to_digit(10) == Some(sum % 10))
}

/// YYMMDD as DD.MM.YYYY in the given century.
fn date(yymmdd: &str, century: u32) -> Option<String> {
    let n = |range: std::ops::Range<usize>| yymmdd.get(range)?.parse::<u32>().ok();
    let (yy, mm, dd) = (n(0..2)?, n(2..4)?, n(4..6)?);
    chrono::NaiveDate::from_ymd_opt((century + yy) as i32, mm, dd).map(|d| d.format("%d.%m.%Y").to_string())
}

/// The zone on a page, if it has one. `lines` are the page's lines; a zone needs its first two.
pub fn find(lines: &[Line]) -> Option<Mrz> {
    let cleaned: Vec<String> = lines.iter().map(|l| clean(&l.text)).collect();
    let middle = |l: &Line| (l.rect[1] + l.rect[3]) / 2.0;
    let second =
        (0..lines.len()).find(|&i| (28..=32).contains(&cleaned[i].chars().count()) && SECOND.is_match(&cleaned[i]))?;
    let above = (0..lines.len()).filter(|&i| {
        middle(&lines[i]) < middle(&lines[second]) && cleaned[i].contains("<<") && FIRST.is_match(&cleaned[i])
    });
    let first = above.max_by(|&a, &b| middle(&lines[a]).total_cmp(&middle(&lines[b])))?;
    let height = lines[second].rect[3] - lines[second].rect[1];
    let below = (0..lines.len()).filter(|&i| {
        middle(&lines[i]) > middle(&lines[second]) && middle(&lines[i]) - middle(&lines[second]) < 3.0 * height
    });
    let third = below.min_by(|&a, &b| middle(&lines[a]).total_cmp(&middle(&lines[b])));

    // OCR may lose or garble where the line starts ("1DG" for "IDBGR"). Its end is sound: the number and its
    // check digit come right before the run of fillers. Where a line has more after the number, the number is
    // looked for after three letters of a state instead.
    let raw = &cleaned[first];
    let repaired = |number: &str| -> String { number.chars().take(2).chain(digits(number).chars().skip(2)).collect() };
    let holds = |at: usize| {
        let number = raw.get(at + 3..at + 12).map(repaired)?;
        raw.get(at + 12..at + 13).and_then(|digit| digit.chars().next()).filter(|digit| check(&number, *digit))
    };
    let from_end = raw.trim_end_matches('<').len().checked_sub(13).filter(|&at| holds(at).is_some());
    let stated = |at: usize| raw.get(at..at + 3).is_some_and(|state| state.chars().all(|c| c.is_ascii_uppercase()));
    let start = from_end.or((0..=4).find(|&at| stated(at) && holds(at).is_some())).unwrap_or(2);
    // the nationality in the second line stands in for a state that did not read
    let named = raw.get(start..start + 3).filter(|_| stated(start)).map(str::to_string);
    let state = named.or(cleaned[second].get(15..18).map(str::to_string)).unwrap_or_default();
    let mut mrz =
        Mrz { state, parts: Vec::new(), lines: [Some(first), Some(second), third].into_iter().flatten().collect() };
    let one: String = "ID".chars().chain(raw.chars().skip(start)).chain(std::iter::repeat('<')).take(30).collect();
    let two = &cleaned[second];
    if two.chars().count() != 30 || !two.is_ascii() || !one.is_ascii() {
        return Some(mrz);
    }
    let mut part = |name, value: String, checked, line| mrz.parts.push(Part { name, value, checked, line });

    // the number: two letters and seven digits, or nine digits on older cards
    let number: String = one[5..7].chars().chain(digits(&one[7..14]).chars()).collect();
    part(
        "document_number",
        number.trim_matches('<').to_string(),
        Some(check(&number, one.as_bytes()[14] as char)),
        first,
    );

    let (birth, expiry) = (digits(&two[0..6]), digits(&two[8..14]));
    let personal = digits(two[18..29].trim_matches('<'));
    let egn = valid_egn(&personal);
    let composite = format!("{}{}{}{}{}", number, &one[14..30], digits(&two[0..7]), digits(&two[8..15]), &two[18..29]);
    let whole = check(&composite, two.as_bytes()[29] as char);
    // the EGN says the century of birth; without it, a year still ahead is the last century's
    let born = egn_birth_date(&personal).filter(|_| egn).map(|d| d.format("%d.%m.%Y").to_string()).or_else(|| {
        let this_year = chrono::Local::now().format("%y").to_string().parse::<u32>().unwrap_or(99);
        let yy = birth.get(0..2)?.parse::<u32>().ok()?;
        date(&birth, if yy > this_year { 1900 } else { 2000 })
    });
    if let Some(born) = born {
        part("birth_date", born, Some(check(&birth, two.as_bytes()[6] as char)), second);
    }
    if let Some(expires) = date(&expiry, 2000) {
        part("expiry_date", expires, Some(check(&expiry, two.as_bytes()[14] as char)), second);
    }
    if matches!(two.as_bytes()[7], b'M' | b'F') {
        part("sex", (two.as_bytes()[7] as char).to_string(), Some(whole), second);
    }
    // read right when the zone's check holds, but only a real EGN is kept
    if !personal.is_empty() {
        part("personal_number", personal, Some(egn), second);
    }

    // names have no check digit: surname, then the given names after a double filler
    if let Some(third) =
        third.filter(|&i| cleaned[i].chars().all(|c| c.is_ascii_uppercase() || c == '<') && cleaned[i].contains("<<"))
    {
        let names = cleaned[third].trim_matches('<').to_string();
        let (surname, given) = names.split_once("<<").unwrap_or((&names, ""));
        let mut given = given.split('<').filter(|name| !name.is_empty());
        part("surname", surname.replace('<', " "), None, third);
        if let Some(name) = given.next() {
            part("first_name", name.to_string(), None, third);
        }
        let rest: Vec<&str> = given.collect();
        if !rest.is_empty() {
            part("middle_name", rest.join(" "), None, third);
        }
    }
    Some(mrz)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(texts: &[&str]) -> Vec<Line> {
        texts
            .iter()
            .enumerate()
            .map(|(i, text)| Line {
                text: text.to_string(),
                rect: [30.0, 270.0 + 32.0 * i as f64, 600.0, 290.0 + 32.0 * i as f64],
                confidence: 0.9,
            })
            .collect()
    }

    fn part<'a>(mrz: &'a Mrz, name: &str) -> (&'a str, Option<bool>) {
        let part = mrz.parts.iter().find(|p| p.name == name).unwrap();
        (&part.value, part.checked)
    }

    // the specimen card the Ministry of Interior publishes
    const SPECIMEN: [&str; 3] =
        ["IDBGRAA00000000<<<<<<<<<<<<<<<", "9508015F3406175BGR9508010133<0", "IVANOVA<<SLAVINA<GEORGIEVA<<<<"];

    #[test]
    fn reads_the_zone_and_checks_its_digits() {
        let mrz = find(&lines(&SPECIMEN)).unwrap();
        assert_eq!(mrz.state, "BGR");
        assert_eq!(part(&mrz, "document_number"), ("AA0000000", Some(true)));
        assert_eq!(part(&mrz, "birth_date"), ("01.08.1995", Some(true)));
        assert_eq!(part(&mrz, "expiry_date"), ("17.06.2034", Some(true)));
        assert_eq!(part(&mrz, "sex"), ("F", Some(true)));
        assert_eq!(part(&mrz, "personal_number"), ("9508010133", Some(false)), "the specimen's EGN is not a real one");
        assert_eq!(part(&mrz, "surname"), ("IVANOVA", None));
        assert_eq!(part(&mrz, "first_name"), ("SLAVINA", None));
        assert_eq!(part(&mrz, "middle_name"), ("GEORGIEVA", None));
    }

    #[test]
    fn a_letter_read_for_a_digit_is_repaired_and_a_wrong_digit_is_caught() {
        let repaired =
            find(&lines(&["IDBGRAAOOOOOOOO<<<<<<<<<<<<<<<", "95O8O15F34O6175BGR95O8O1O133<O", SPECIMEN[2]])).unwrap();
        assert_eq!(part(&repaired, "birth_date"), ("01.08.1995", Some(true)));
        assert_eq!(part(&repaired, "document_number"), ("AA0000000", Some(true)));

        let wrong = find(&lines(&[SPECIMEN[0], "9508025F3406175BGR9508010133<0", SPECIMEN[2]])).unwrap();
        assert_eq!(part(&wrong, "birth_date").1, Some(false), "the day was misread: its check digit no longer holds");
        assert_eq!(part(&wrong, "expiry_date").1, Some(true));
    }

    #[test]
    fn a_real_egn_in_the_zone_holds_and_gives_the_century_of_birth() {
        let mrz = find(&lines(&[SPECIMEN[0], "9508015F3406175BGR9508010139<2", SPECIMEN[2]])).unwrap();
        assert_eq!(part(&mrz, "personal_number"), ("9508010139", Some(true)));
        assert_eq!(part(&mrz, "birth_date"), ("01.08.1995", Some(true)));
    }

    #[test]
    fn a_first_line_that_lost_its_start_is_lined_up_by_the_numbers_check_digit() {
        // as read from the pre-2024 specimen: "ID" came out as "1", a filler as an angle
        let mrz = find(&lines(&[
            "1BGR6000000002<<<<<<<<<<<<<<<",
            "8508018F1908011BGR8508010133<1",
            "IVANOVA<∠MARITSA<RADNEVA<<<<<",
        ]))
        .unwrap();
        assert_eq!(mrz.state, "BGR");
        assert_eq!(part(&mrz, "document_number"), ("600000000", Some(true)));
        assert_eq!(part(&mrz, "expiry_date"), ("01.08.2019", Some(true)));
        assert_eq!(part(&mrz, "first_name"), ("MARITSA", None));

        let worse = find(&lines(&["1DG6000000002<<<<<<<<<<<<<<<", "8508018F1908011BGR8508010133<1"])).unwrap();
        assert_eq!(worse.state, "BGR", "the state did not read: the nationality stands in");
        assert_eq!(part(&worse, "document_number"), ("600000000", Some(true)));
    }

    #[test]
    fn a_page_without_a_zone_has_none() {
        assert!(find(&lines(&["РЕПУБЛИКА БЪЛГАРИЯ", "ЛИЧНА КАРТА", "Фамилия ИВАНОВА"])).is_none());
    }
}
