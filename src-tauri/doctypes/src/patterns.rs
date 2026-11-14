//! Named value patterns of a document config set: each finds its value in a piece of text, normalizes it and checks
//! it. The rules come with the set; the steps they are made of, and the checks, are the engine's.

use std::collections::{BTreeMap, HashMap};
use std::sync::LazyLock;

use regex::Regex;
use serde::Deserialize;

use super::fold;
use super::set::Script;
use crate::values::{valid_egn, valid_eik, valid_lnch};

static DIGITS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\d+").unwrap());
static GROUP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\$(\d)").unwrap());

/// What OCR reads in place of a digit.
pub fn digit(c: char) -> char {
    match c {
        'O' | 'Q' | 'D' => '0',
        'I' | 'L' => '1',
        'Z' => '2',
        'S' => '5',
        'G' => '6',
        'B' => '8',
        other => other,
    }
}

#[derive(Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "lowercase")]
enum Check {
    Egn,
    Eik,
    Lnch,
    /// has a letter: a run of digits alone is not it
    Letter,
}

impl Check {
    fn holds(self, value: &str) -> bool {
        match self {
            Check::Egn => valid_egn(value),
            Check::Eik => valid_eik(value),
            Check::Lnch => valid_lnch(value),
            Check::Letter => value.chars().any(char::is_alphabetic),
        }
    }
}

/// A rule as the set writes it. The steps run in the order of the fields.
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct Spec {
    /// look-alike letters of the other alphabet are read as this one, then upper case
    #[serde(default)]
    fold: Option<Script>,
    /// spaces and dashes are dropped
    #[serde(default)]
    squeeze: bool,
    /// characters read as others
    #[serde(default)]
    replace: BTreeMap<char, String>,
    /// a regular expression; its matches are the candidates, left to right
    #[serde(default)]
    find: Option<String>,
    /// instead of `find`: whole runs of digits of one of these lengths
    #[serde(default)]
    runs: Vec<usize>,
    /// the groups of `find` where a letter is a digit misread
    #[serde(default)]
    digits: Vec<usize>,
    /// the value from the groups of `find`, "$1$2"; the whole match when left out
    #[serde(default)]
    value: Option<String>,
    /// what a candidate must pass
    #[serde(default)]
    check: Vec<Check>,
    /// alternatives, tried in order; nothing else is given beside it
    #[serde(default)]
    any: Vec<Spec>,
}

enum Find {
    Match(Regex),
    Runs(Vec<usize>),
}

struct Rule {
    fold: Option<Script>,
    squeeze: bool,
    replace: BTreeMap<char, String>,
    find: Find,
    digits: Vec<usize>,
    value: Option<String>,
    check: Vec<Check>,
}

impl Rule {
    fn compile(spec: Spec) -> Result<Vec<Rule>, String> {
        if !spec.any.is_empty() {
            if spec.find.is_some() || !spec.runs.is_empty() {
                return Err("`any` stands alone".into());
            }
            return spec
                .any
                .into_iter()
                .map(Rule::compile)
                .collect::<Result<Vec<_>, _>>()
                .map(|rules| rules.into_iter().flatten().collect());
        }
        let find = match (spec.find, spec.runs.is_empty()) {
            (Some(find), true) => Find::Match(Regex::new(&find).map_err(|e| e.to_string())?),
            (None, false) => Find::Runs(spec.runs),
            _ => return Err("one of `find` and `runs` is needed".into()),
        };
        Ok(vec![Rule {
            fold: spec.fold,
            squeeze: spec.squeeze,
            replace: spec.replace,
            find,
            digits: spec.digits,
            value: spec.value,
            check: spec.check,
        }])
    }

    fn extract(&self, text: &str) -> Option<String> {
        let mut text = match self.fold {
            Some(Script::Latin) => fold::latin(text).to_uppercase(),
            Some(Script::Cyrillic) => fold::cyrillic(text).to_uppercase(),
            None => text.to_string(),
        };
        if self.squeeze {
            text.retain(|c| !c.is_whitespace() && c != '-');
        }
        if !self.replace.is_empty() {
            text = text.chars().map(|c| self.replace.get(&c).cloned().unwrap_or_else(|| c.to_string())).collect();
        }
        let holds = |value: &String| !value.is_empty() && self.check.iter().all(|check| check.holds(value));
        match &self.find {
            Find::Runs(lengths) => DIGITS
                .find_iter(&text)
                .map(|m| m.as_str().to_string())
                .filter(|run| lengths.contains(&run.len()))
                .find(holds),
            Find::Match(regex) => regex
                .captures_iter(&text)
                .map(|found| {
                    let group = |n: usize| {
                        let part = found.get(n).map_or("", |m| m.as_str());
                        if self.digits.contains(&n) {
                            part.chars().map(digit).collect()
                        } else {
                            part.to_string()
                        }
                    };
                    match &self.value {
                        Some(template) => GROUP
                            .replace_all(template, |c: &regex::Captures| group(c[1].parse().unwrap_or(0)))
                            .into_owned(),
                        None => group(0),
                    }
                })
                .find(holds),
        }
    }
}

/// The patterns of a set, by name.
#[derive(Default)]
pub struct Patterns {
    rules: HashMap<String, Vec<Rule>>,
}

impl Patterns {
    /// Compiles the set's patterns. One that does not compile is left out and named with the reason, so the
    /// documents that use it are left out too and the rest of the set works.
    pub fn compile(specs: BTreeMap<String, serde_yaml::Value>) -> (Patterns, Vec<(String, String)>) {
        let (mut rules, mut dropped) = (HashMap::new(), Vec::new());
        for (name, value) in specs {
            let compiled = serde_yaml::from_value::<Spec>(value).map_err(|e| e.to_string()).and_then(Rule::compile);
            match compiled {
                Ok(rule) => {
                    rules.insert(name, rule);
                }
                Err(reason) => dropped.push((name, reason)),
            }
        }
        (Patterns { rules }, dropped)
    }

    pub fn has(&self, name: &str) -> bool {
        self.rules.contains_key(name)
    }

    /// The first value of the pattern in the text, normalized; None when there is none that holds.
    pub fn extract(&self, name: &str, text: &str) -> Option<String> {
        self.rules.get(name)?.iter().find_map(|rule| rule.extract(text))
    }
}

/// A whole run of ten digits that is a valid EGN: how a field of type `egn` is read when it names no pattern.
pub fn egn(text: &str) -> Option<String> {
    DIGITS.find_iter(text).map(|m| m.as_str().to_string()).find(|run| run.len() == 10 && valid_egn(run))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patterns(yaml: &str) -> (Patterns, Vec<(String, String)>) {
        Patterns::compile(serde_yaml::from_str(yaml).unwrap())
    }

    #[test]
    fn a_rule_folds_squeezes_replaces_finds_repairs_and_checks() {
        let (patterns, dropped) = patterns(
            r#"
            tag: { fold: latin, find: '\b([ABEKMHOPCTYX]{2}) ?(\d{4})\b', value: "$1$2" }
            serial: { fold: latin, squeeze: true, replace: { O: "0" }, find: '[A-Z0-9]{8}', check: [letter] }
            egn: { runs: [10], check: [egn] }
            either: { any: [ { runs: [10], check: [egn] }, { find: '[A-Z]{3}-\d{2}' } ] }
            card: { fold: latin, find: '\b([A-Z]{2})([0-9OIB]{4})\b', digits: [2], value: "$1$2" }
            "#,
        );
        assert!(dropped.is_empty(), "{dropped:?}");
        assert_eq!(patterns.extract("tag", "№ СА 1234").as_deref(), Some("CA1234"), "Cyrillic look-alikes fold");
        assert_eq!(patterns.extract("tag", "1234"), None);
        assert_eq!(patterns.extract("serial", "AB-12 O4 7Z").as_deref(), Some("AB12047Z"));
        assert_eq!(patterns.extract("serial", "1234 5678"), None, "digits alone fail the letter check");
        assert_eq!(patterns.extract("egn", "No 9508010139").as_deref(), Some("9508010139"));
        assert_eq!(patterns.extract("egn", "9508010133"), None, "the checksum fails");
        assert_eq!(patterns.extract("egn", "19508010139"), None, "a longer run is not it");
        assert_eq!(
            patterns.extract("egn", "9508010133 7501020018").as_deref(),
            Some("7501020018"),
            "the first that holds"
        );
        assert_eq!(patterns.extract("either", "ref ABC-12").as_deref(), Some("ABC-12"));
        assert_eq!(patterns.extract("card", "ABO1IB").as_deref(), Some("AB0118"), "letters among the digits");
        assert_eq!(patterns.extract("unknown", "ABC"), None);
        assert_eq!(egn("ЕГН7501020018").as_deref(), Some("7501020018"));
    }

    #[test]
    fn a_rule_that_does_not_compile_is_dropped_alone() {
        let (patterns, dropped) = patterns(
            r#"
            good: { find: '\d{3}' }
            bad_regex: { find: '(' }
            newer: { find: '\d', lookahead: true }
            neither: { fold: latin }
            both: { find: '\d', runs: [3] }
            "#,
        );
        assert!(patterns.has("good"));
        let names: Vec<&str> = dropped.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(names, ["bad_regex", "both", "neither", "newer"]);
        assert!(!patterns.has("newer"), "a key this engine does not know");
    }
}
