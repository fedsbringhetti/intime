//! Upstream names are in capitals ("PORTA NUOVA"); messages show them as titles ("Porta Nuova"),
//! like the web app. A port of `displayName` in `web/src/lib/format.ts`: keep the two in step.

// Words written in lowercase inside a name, as in "Strada del Drosso".
const SMALL_WORDS: &[&str] = &[
    "a", "ad", "al", "alla", "alle", "agli", "ai", "con", "da", "dal", "dalla", "dalle", "dai",
    "degli", "dei", "del", "della", "delle", "dello", "di", "e", "ed", "fra", "gli", "i", "il",
    "in", "la", "le", "lo", "nel", "nella", "per", "sul", "sulla", "su", "tra",
];
// The same before an apostrophe: "dell'Arsenale", "all'Angelo".
const SMALL_ELISIONS: &[&str] = &["all", "dall", "dell", "nell", "sull", "l", "d", "un"];
const ACRONYMS: &[&str] = &["GTT", "SFM", "FS", "RFI", "ASL", "ATC", "ITIS"];

/// Ends reachable after matching one group of the Roman numeral pattern at `pos`:
/// `pair` (two letters) or an optional `lead` followed by up to `max` of `rep`.
fn group_ends(s: &[u8], pos: usize, pair: [&[u8; 2]; 2], lead: u8, rep: u8) -> Vec<usize> {
    let mut ends = Vec::new();
    for p in pair {
        if s[pos..].starts_with(p) {
            ends.push(pos + 2);
        }
    }
    let mut start = pos;
    ends.push(start);
    if s.get(pos) == Some(&lead) {
        start = pos + 1;
    }
    let mut at = start;
    ends.push(at);
    while at - start < 3 && s.get(at) == Some(&rep) {
        at += 1;
        ends.push(at);
    }
    ends
}

/// `^(?=[IVXLC]{2,}$)(C[DM]|D?C{0,3})(X[CL]|L?X{0,3})(I[XV]|V?I{0,3})$`
fn is_roman(word: &str) -> bool {
    let s = word.as_bytes();
    if s.len() < 2 || !s.iter().all(|b| b"IVXLC".contains(b)) {
        return false;
    }
    group_ends(s, 0, [b"CD", b"CM"], b'D', b'C')
        .into_iter()
        .any(|a| {
            group_ends(s, a, [b"XC", b"XL"], b'L', b'X')
                .into_iter()
                .any(|b| {
                    group_ends(s, b, [b"IX", b"IV"], b'V', b'I')
                        .into_iter()
                        .any(|c| c == s.len())
                })
        })
}

fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first
            .to_uppercase()
            .chain(chars.flat_map(char::to_lowercase))
            .collect(),
        None => String::new(),
    }
}

fn title_word(word: &str, first: bool) -> String {
    if word.chars().any(|c| c.is_ascii_digit()) || ACRONYMS.contains(&word) || is_roman(word) {
        return word.to_owned();
    }
    let lower = word.to_lowercase();
    if !first && SMALL_WORDS.contains(&lower.as_str()) {
        return lower;
    }
    capitalize(word)
}

/// A word is "first" at the start and after a dash, an opening bracket or a slash.
fn starts_part(before: &str) -> bool {
    let trimmed = before.trim_end();
    trimmed.is_empty() && before.is_empty()
        || matches!(trimmed.chars().next_back(), Some('-' | '(' | '/'))
}

/// A name that already has lowercase letters (other companies) is left alone. Codes, acronyms
/// and Roman numerals stay as they are.
pub fn display_name(name: &str) -> String {
    if name.chars().any(char::is_lowercase) {
        return name.to_owned();
    }
    let chars: Vec<(usize, char)> = name.char_indices().collect();
    let word = |c: char| c.is_alphanumeric();
    let mut out = String::with_capacity(name.len());
    let mut i = 0;
    let mut last = 0;
    while i < chars.len() {
        if !word(chars[i].1) {
            i += 1;
            continue;
        }
        // Token: alphanumeric runs joined by apostrophes that are followed by alphanumerics.
        let mut j = i;
        loop {
            while j < chars.len() && word(chars[j].1) {
                j += 1;
            }
            if j + 1 < chars.len() && chars[j].1 == '\'' && word(chars[j + 1].1) {
                j += 1;
            } else {
                break;
            }
        }
        let start = chars[i].0;
        let end = chars.get(j).map_or(name.len(), |c| c.0);
        let token = &name[start..end];
        let before = &name[..start];
        out.push_str(&name[last..start]);
        last = end;
        i = j;

        let first = starts_part(before);
        let mut before_chars = before.chars().rev();
        let abbreviation = before_chars.next() == Some('.')
            && before_chars.next().is_some_and(char::is_alphabetic)
            && (1..=3).contains(&token.chars().count())
            && token.chars().all(char::is_alphabetic);
        if abbreviation {
            out.push_str(&token.to_lowercase());
            continue;
        }
        let parts: Vec<&str> = token.split('\'').collect();
        let titled: Vec<String> = parts
            .iter()
            .enumerate()
            .map(|(index, part)| {
                let is_first = first && index == 0;
                if index < parts.len() - 1
                    && !is_first
                    && SMALL_ELISIONS.contains(&part.to_lowercase().as_str())
                {
                    part.to_lowercase()
                } else {
                    title_word(part, is_first)
                }
            })
            .collect();
        out.push_str(&titled.join("'"));
    }
    out.push_str(&name[last..]);
    out
}

#[cfg(test)]
mod tests {
    use super::display_name;

    fn check(input: &str, expected: &str) {
        assert_eq!(display_name(input), expected, "{input}");
    }

    #[test]
    fn turns_capitals_into_a_title() {
        check("PORTA NUOVA", "Porta Nuova");
        check("STRADA DEL DROSSO", "Strada del Drosso");
        check(
            "MONCALIERI, TESTONA - PIAZZA MARCONI",
            "Moncalieri, Testona - Piazza Marconi",
        );
    }

    #[test]
    fn keeps_codes_acronyms_and_roman_numerals() {
        check("PORTA NUOVA - M1 - STAZIONE", "Porta Nuova - M1 - Stazione");
        check("VITTORIO EMANUELE II CAP", "Vittorio Emanuele II Cap");
        check("C. ROMA N.45 MONCALIERI", "C. Roma N.45 Moncalieri");
        check("STAZIONE SFM DORA", "Stazione SFM Dora");
    }

    #[test]
    fn keeps_abbreviations_with_a_dot_readable() {
        check("ASTI (C.SO CASALE /V. TOSI)", "Asti (C.so Casale /V. Tosi)");
        check("CALUSO (P.ZA MAZZINI)", "Caluso (P.za Mazzini)");
        check("FAIALLO (FRAZ.CAMPETTI)", "Faiallo (Fraz.Campetti)");
        check("PLAVA C.30", "Plava C.30");
    }

    #[test]
    fn handles_apostrophes() {
        check("SANT'ANNA", "Sant'Anna");
        check("PIAZZA DELL'ARSENALE", "Piazza dell'Arsenale");
        check("L'AQUILA", "L'Aquila");
    }

    #[test]
    fn capitalizes_small_words_that_start_a_name_or_a_part() {
        check("LA LOGGIA", "La Loggia");
        check("ARSENALE (DI FRONTE)", "Arsenale (Di Fronte)");
    }

    #[test]
    fn leaves_names_that_are_already_mixed_case() {
        check("Torino Porta Nuova", "Torino Porta Nuova");
    }

    #[test]
    fn handles_the_summary_examples() {
        check("VAL SALICE, PIAZZALE ADUA", "Val Salice, Piazzale Adua");
        check("", "");
    }
}
