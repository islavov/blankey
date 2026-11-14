//! Cyrillic and Latin letters that print alike. OCR mixes them freely, so text is folded to one script before it is
//! compared, and a value is folded to the script its field is printed in.

const PAIRS: &[(char, char)] = &[
    ('А', 'A'),
    ('В', 'B'),
    ('Е', 'E'),
    ('К', 'K'),
    ('М', 'M'),
    ('Н', 'H'),
    ('О', 'O'),
    ('Р', 'P'),
    ('С', 'C'),
    ('Т', 'T'),
    ('У', 'Y'),
    ('Х', 'X'),
    ('а', 'a'),
    ('е', 'e'),
    ('о', 'o'),
    ('р', 'p'),
    ('с', 'c'),
    ('у', 'y'),
    ('х', 'x'),
];
// only for comparing: lowercased capitals that have no look-alike in lowercase
const LOWER_ONLY: &[(char, char)] = &[('в', 'b'), ('к', 'k'), ('м', 'm'), ('н', 'h'), ('т', 't')];

/// Cyrillic look-alikes as Latin letters.
pub fn latin(text: &str) -> String {
    text.chars().map(|c| PAIRS.iter().find(|(cyr, _)| *cyr == c).map_or(c, |(_, lat)| *lat)).collect()
}

/// Latin look-alikes as Cyrillic letters.
pub fn cyrillic(text: &str) -> String {
    text.chars().map(|c| PAIRS.iter().find(|(_, lat)| *lat == c).map_or(c, |(cyr, _)| *cyr)).collect()
}

/// The text for comparing: lowercase, look-alikes as Latin. One char per char of the text, so positions carry over.
pub fn skeleton(text: &str) -> Vec<char> {
    text.chars()
        .map(|c| {
            let lower = c.to_lowercase().next().unwrap_or(c);
            let pairs = PAIRS.iter().chain(LOWER_ONLY);
            pairs.clone().find(|(cyr, _)| *cyr == lower).map_or(lower, |(_, lat)| *lat)
        })
        .collect()
}

/// Where `needle` is in `hay` with at most `errors` wrong, missing or extra chars: (errors, start, end), the best
/// match that starts a word. A needle that allows no errors must also end a word.
pub fn find(hay: &[char], needle: &[char], errors: usize) -> Option<(usize, usize, usize)> {
    find_with(hay, needle, errors, &[])
}

/// Where a capital follows a small letter in the text: a label run into its value, "SexЖ", ends there.
pub fn case_breaks(text: &str) -> Vec<bool> {
    let chars: Vec<char> = text.chars().collect();
    (0..chars.len()).map(|i| i > 0 && chars[i].is_uppercase() && chars[i - 1].is_lowercase()).collect()
}

/// `find`, where a word also ends at each position marked in `breaks`.
pub fn find_with(hay: &[char], needle: &[char], errors: usize, breaks: &[bool]) -> Option<(usize, usize, usize)> {
    if needle.is_empty() || hay.is_empty() {
        return None;
    }
    let boundary = |at: usize| at == 0 || !hay[at - 1].is_alphanumeric();
    let ends = |at: usize| at >= hay.len() || !hay[at].is_alphanumeric() || breaks.get(at).copied().unwrap_or(false);
    // approximate substring search: row j holds (errors, start) of the best match of needle[..j] ending here
    let mut prev: Vec<(usize, usize)> = (0..=needle.len()).map(|j| (j, 0)).collect();
    let mut best: Option<(usize, usize, usize)> = None;
    for i in 1..=hay.len() {
        let mut cur = Vec::with_capacity(needle.len() + 1);
        cur.push((0, i));
        for j in 1..=needle.len() {
            let substitute = (prev[j - 1].0 + usize::from(hay[i - 1] != needle[j - 1]), prev[j - 1].1);
            let skip_hay = (prev[j].0 + 1, prev[j].1);
            let skip_needle = (cur[j - 1].0 + 1, cur[j - 1].1);
            cur.push([substitute, skip_hay, skip_needle].into_iter().min_by_key(|c| c.0).unwrap());
        }
        let (cost, start) = cur[needle.len()];
        if cost <= errors && start < i && boundary(start) && (errors > 0 || ends(i)) && best.is_none_or(|b| cost < b.0)
        {
            best = Some((cost, start, i));
        }
        prev = cur;
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(hay: &str, needle: &str, errors: usize) -> Option<(usize, usize)> {
        find(&skeleton(hay), &skeleton(needle), errors).map(|(_, start, end)| (start, end))
    }

    #[test]
    fn folds_between_scripts() {
        assert_eq!(latin("СА1234АВ"), "CA1234AB");
        assert_eq!(cyrillic("COБCTBEHИK"), "СОБСТВЕНИК");
        assert_eq!(skeleton("СВИДЕТЕЛСТВО"), skeleton("CBИДETEЛCTBO"));
    }

    #[test]
    fn finds_labels_at_word_starts_with_a_few_errors() {
        assert_eq!(at("Father's name GEORGIEVA", "father's name", 2), Some((0, 13)));
        assert_eq!(at("Място на раждане/Placе of bith СОФИЯ", "place of birth", 2), Some((17, 30)));
        assert_eq!(at("Презиме ГЕОРГИЕВА", "име", 0), None, "not inside a word");
        assert_eq!(at("Име СЛАВИНА", "име", 0), Some((0, 3)));
        assert_eq!(at("Именуван", "име", 0), None, "an exact label ends its word");
        assert_eq!(at("ERH/ID", "id", 0), Some((4, 6)));
        let glued = "SexЖ|F";
        assert_eq!(
            find_with(&skeleton(glued), &skeleton("sex"), 0, &case_breaks(glued)).map(|m| (m.1, m.2)),
            Some((0, 3))
        );
    }
}
