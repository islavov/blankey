//! Which document of a set a file is. Every document's `match` is tried against what a page shows, which is worked
//! out once per page; only the one picked is then read in full. So a set may hold many documents.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use super::locate::{anchor, slack};
use super::set::{canonical, Config, Match, Set, Side};
use super::trace::{Condition, DocumentSurvey, PageSurvey, SideSurvey, Survey};
use super::{fold, mrz, Page};

/// What a page shows that a `match` can ask about.
struct PageFacts<'a> {
    page: &'a Page,
    skeletons: Vec<Vec<char>>,
    /// the state its machine-readable zone names, and the lines the zone takes
    zone: Option<(String, Vec<usize>)>,
    /// per table of codes (its codes, joined): the ones printed on the page
    codes: RefCell<HashMap<String, HashSet<String>>>,
}

impl<'a> PageFacts<'a> {
    fn new(page: &'a Page) -> PageFacts<'a> {
        PageFacts {
            page,
            skeletons: page.lines.iter().map(|l| fold::skeleton(&l.text)).collect(),
            zone: mrz::find(&page.lines).map(|zone| (zone.state, zone.lines)),
            codes: RefCell::new(HashMap::new()),
        }
    }

    fn has_code(&self, known: &[String], code: &str) -> bool {
        let mut tables = self.codes.borrow_mut();
        let printed = tables.entry(known.join(",")).or_insert_with(|| {
            let in_zone = |line: &usize| self.zone.as_ref().is_some_and(|(_, lines)| lines.contains(line));
            let lines = self.page.lines.iter().enumerate().filter(|(i, _)| !in_zone(i));
            lines.filter_map(|(_, line)| Some(anchor(&line.text, known)?.0)).collect()
        });
        printed.contains(&canonical(code))
    }

    /// Everything the `match` asks, each with whether the page has it.
    fn conditions(&self, wanted: &Match, known: &[String]) -> Vec<Condition> {
        let says = |kind: &'static str, phrase: &String| {
            let needle = fold::skeleton(phrase);
            let line = self.skeletons.iter().position(|line| fold::find(line, &needle, slack(needle.len())).is_some());
            Condition {
                kind,
                wanted: phrase.clone(),
                held: line.is_some(),
                seen: line.map(|at| self.page.lines[at].text.clone()),
            }
        };
        let has = |kind: &'static str, code: &String, printed: bool| Condition {
            kind,
            wanted: code.clone(),
            held: self.has_code(known, code) == printed,
            seen: None,
        };
        let state = self.zone.as_ref().map(|(state, _)| state);
        let mut asked: Vec<Condition> = Vec::new();
        asked.extend(wanted.text_all.iter().map(|phrase| says("text_all", phrase)));
        asked.extend(wanted.text_any.iter().map(|phrase| says("text_any", phrase)));
        asked.extend(wanted.codes_all.iter().map(|code| has("codes_all", code, true)));
        asked.extend(wanted.codes_any.iter().map(|code| has("codes_any", code, true)));
        asked.extend(wanted.codes_none.iter().map(|code| has("codes_none", code, false)));
        asked.extend(wanted.mrz.iter().map(|wanted| Condition {
            kind: "mrz",
            wanted: wanted.clone(),
            held: state == Some(wanted),
            seen: state.cloned(),
        }));
        asked
    }

    fn holds(&self, wanted: &Match, known: &[String]) -> bool {
        holds(&self.conditions(wanted, known))
    }
}

/// Whether the page is the side: every condition holds, and of each `_any` list at least one.
fn holds(conditions: &[Condition]) -> bool {
    let any = |kind: &str| {
        let mut of = conditions.iter().filter(|c| c.kind == kind).peekable();
        of.peek().is_none() || of.any(|c| c.held)
    };
    let one_of = |c: &&Condition| matches!(c.kind, "text_any" | "codes_any");
    conditions.iter().filter(|c| !one_of(c)).all(|c| c.held) && any("text_any") && any("codes_any")
}

/// How much a `match` asks for: of two documents a page could be, the one that asked for more is the surer.
fn conditions(wanted: &Match) -> usize {
    wanted.text_all.len()
        + wanted.codes_all.len()
        + wanted.codes_none.len()
        + usize::from(!wanted.text_any.is_empty())
        + usize::from(!wanted.codes_any.is_empty())
        + usize::from(wanted.mrz.is_some())
}

/// The sides of a document found on the pages: (page index, side).
pub type Sides<'s> = Vec<(usize, &'s Side)>;

/// The side of the document each page is, for the pages that are one.
fn sides_of<'s>(config: &'s Config, facts: &[PageFacts]) -> Sides<'s> {
    let side_of = |page: &PageFacts| config.sides.iter().find(|side| page.holds(&side.matches, &config.known_codes));
    facts.iter().enumerate().filter_map(|(i, page)| Some((i, side_of(page)?))).collect()
}

/// What a document with these sides is ranked by: the pages, its priority, how much its sides asked for.
fn score(config: &Config, sides: &Sides) -> (usize, i32, usize) {
    (sides.len(), config.priority, sides.iter().map(|(_, side)| conditions(&side.matches)).sum())
}

fn choose<'s>(set: &'s Set, facts: &[PageFacts]) -> Option<(&'s Config, Sides<'s>)> {
    let mut best: Option<((usize, i32, usize), &Config, Sides)> = None;
    for config in &set.documents {
        let sides = sides_of(config, facts);
        if sides.is_empty() {
            continue;
        }
        let score = score(config, &sides);
        if best.as_ref().is_none_or(|(held, _, _)| score > *held) {
            best = Some((score, config, sides));
        }
    }
    best.map(|(_, config, sides)| (config, sides))
}

/// The document the pages are, with the side each page is. Among the documents with a side on
/// some page, the one with the most pages, then the highest priority, then the most asked for, then the first in
/// the set.
pub fn pick<'s>(set: &'s Set, pages: &[Page]) -> Option<(&'s Config, Sides<'s>)> {
    let facts: Vec<PageFacts> = pages.iter().map(PageFacts::new).collect();
    choose(set, &facts)
}

/// The sides of one document on the pages, whether or not it is the one picked.
pub fn sides<'s>(config: &'s Config, pages: &[Page]) -> Sides<'s> {
    let facts: Vec<PageFacts> = pages.iter().map(PageFacts::new).collect();
    sides_of(config, &facts)
}

/// Every document of the set against the pages: what each side asked and got, and which document was picked.
pub fn survey(set: &Set, pages: &[Page]) -> Survey {
    let facts: Vec<PageFacts> = pages.iter().map(PageFacts::new).collect();
    let picked = choose(set, &facts).map(|(config, _)| config.id.as_str());
    let documents = set.documents.iter().map(|config| {
        let found = sides_of(config, &facts);
        let tried = facts.iter().enumerate().flat_map(|(page, facts)| {
            config.sides.iter().map(move |side| {
                let conditions = facts.conditions(&side.matches, &config.known_codes);
                SideSurvey { page, side: side.id.clone(), holds: holds(&conditions), conditions }
            })
        });
        DocumentSurvey {
            id: config.id.clone(),
            priority: config.priority,
            sides: tried.collect(),
            score: (!found.is_empty()).then(|| score(config, &found)),
            picked: picked == Some(config.id.as_str()),
        }
    });
    let pages = facts.iter().map(|facts| PageSurvey {
        mrz: facts.zone.as_ref().map(|(state, _)| state.clone()),
        codes: set
            .tables
            .iter()
            .map(|table| {
                // a code with a value of two lines is two fields
                let mut written: Vec<&str> = table.codes.iter().filter_map(|field| field.code.as_deref()).collect();
                written.dedup();
                let known: Vec<String> = written.iter().map(|code| canonical(code)).collect();
                let printed = written.iter().filter(|code| facts.has_code(&known, code)).map(|code| code.to_string());
                (table.id.clone(), printed.collect())
            })
            .collect(),
    });
    Survey { pages: pages.collect(), documents: documents.collect() }
}
