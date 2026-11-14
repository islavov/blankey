//! Which document of a set a file is. Every document's `match` is tried against what a page shows, which is worked
//! out once per page; only the one picked is then read in full. So a set may hold many documents.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use super::locate::{anchor, slack};
use super::set::{canonical, Config, Match, Set, Side};
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

    fn holds(&self, wanted: &Match, known: &[String]) -> bool {
        let says = |phrase: &String| {
            let needle = fold::skeleton(phrase);
            self.skeletons.iter().any(|line| fold::find(line, &needle, slack(needle.len())).is_some())
        };
        let has = |code: &String| self.has_code(known, code);
        wanted.text_all.iter().all(says)
            && (wanted.text_any.is_empty() || wanted.text_any.iter().any(says))
            && wanted.codes_all.iter().all(has)
            && (wanted.codes_any.is_empty() || wanted.codes_any.iter().any(has))
            && !wanted.codes_none.iter().any(has)
            && wanted.mrz.as_ref().is_none_or(|state| self.zone.as_ref().is_some_and(|(found, _)| found == state))
    }
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
type Sides<'s> = Vec<(usize, &'s Side)>;

/// The document the pages are, with the side each page is. Among the documents with a side on
/// some page, the one with the most pages, then the highest priority, then the most asked for, then the first in
/// the set.
pub fn pick<'s>(set: &'s Set, pages: &[Page]) -> Option<(&'s Config, Sides<'s>)> {
    let facts: Vec<PageFacts> = pages.iter().map(PageFacts::new).collect();
    let mut best: Option<((usize, i32, usize), &Config, Sides)> = None;
    for config in &set.documents {
        let side_of =
            |page: &PageFacts| config.sides.iter().find(|side| page.holds(&side.matches, &config.known_codes));
        let sides: Sides = facts.iter().enumerate().filter_map(|(i, page)| Some((i, side_of(page)?))).collect();
        if sides.is_empty() {
            continue;
        }
        let asked = sides.iter().map(|(_, side)| conditions(&side.matches)).sum();
        let score = (sides.len(), config.priority, asked);
        if best.as_ref().is_none_or(|(held, _, _)| score > *held) {
            best = Some((score, config, sides));
        }
    }
    best.map(|(_, config, sides)| (config, sides))
}
