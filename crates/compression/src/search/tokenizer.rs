// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// `kernel/src/search/tokenizer.ts` and `kernel/src/search/stemmer.ts` of
// acp-kernel 0.0.189 at commit `4be21b08e`. Copyright (c) 2026 ranxianglei.
// MIT License with an additional attribution term (NOTICE).
// Modified for hanafubuki-agent: there is no dictionary segmenter
// (`Intl.Segmenter`), so a CJK run of two or more characters becomes its
// overlapping bigrams (most dictionary words are two characters) and a
// one-character run stays itself; bigrams are of characters, not UTF-16
// units.

use std::collections::HashMap;

/// Ideographs, kana and hangul: the scripts with no spaces between words.
pub(crate) fn is_cjk(character: char) -> bool {
    matches!(
        character,
        '\u{3400}'..='\u{9fff}' | '\u{f900}'..='\u{faff}' | '\u{3040}'..='\u{30ff}' | '\u{ac00}'..='\u{d7af}'
    )
}

/// JavaScript's `\s`.
pub(crate) fn is_space(character: char) -> bool {
    character.is_whitespace() || character == '\u{feff}'
}

/// The search terms of `text`, lowercased: its Latin words of two or more
/// characters (stemmed when `stem`), then its CJK runs in text order.
pub fn tokenize(text: &str, stem: bool) -> Vec<String> {
    let lower = text.to_lowercase();
    let mut tokens: Vec<String> = latin_words(&lower)
        .into_iter()
        .filter(|word| word.len() >= 2)
        .map(|word| {
            if stem {
                stem_word(word)
            } else {
                word.to_owned()
            }
        })
        .collect();
    for run in cjk_runs(&lower) {
        if run.len() == 1 {
            tokens.push(run[0].to_string());
        } else {
            tokens.extend(run.windows(2).map(|pair| pair.iter().collect::<String>()));
        }
    }
    tokens
}

/// The adjacent character pairs of `text` with no whitespace in them.
pub fn char_bigrams(text: &str) -> Vec<String> {
    let characters: Vec<char> = text.chars().collect();
    characters
        .windows(2)
        .filter(|pair| !pair.iter().any(|character| is_space(*character)))
        .map(|pair| pair.iter().collect())
        .collect()
}

/// How often each term of `text` occurs.
pub fn term_frequencies(text: &str, stem: bool) -> HashMap<String, u32> {
    let mut frequencies = HashMap::new();
    for token in tokenize(text, stem) {
        *frequencies.entry(token).or_insert(0) += 1;
    }
    frequencies
}

/// A light suffix-stripping stemmer for English: `compressed` becomes
/// `compress`, `tokens` `token`. Not Porter: `authenticate` and
/// `authenticated` stay apart.
pub fn stem_word(word: &str) -> String {
    let mut w = word.to_owned();
    if w.len() <= 3 {
        return w;
    }
    if w.ends_with("ies") {
        w.truncate(w.len() - 3);
        w.push('y');
    } else if ["ses", "xes", "zes", "ches", "shes"]
        .iter()
        .any(|suffix| w.ends_with(suffix))
    {
        w.truncate(w.len() - 2);
    } else if w.ends_with('s') && !w.ends_with("ss") {
        w.truncate(w.len() - 1);
    }
    strip(&mut w, "ing", 5, 3);
    strip(&mut w, "ed", 4, 2);
    if w.ends_with("ation") && w.len() > 6 {
        w.truncate(w.len() - 3);
    } else if w.ends_with("tion") && w.len() > 5 {
        w.truncate(w.len() - 4);
        w.push('t');
    } else {
        strip(&mut w, "ion", 4, 3);
    }
    strip(&mut w, "ment", 6, 4);
    strip(&mut w, "ness", 6, 4);
    strip(&mut w, "ly", 4, 2);
    w
}

fn strip(word: &mut String, suffix: &str, longer_than: usize, cut: usize) {
    if word.ends_with(suffix) && word.len() > longer_than {
        word.truncate(word.len() - cut);
    }
}

/// The matches of `[a-z][a-z0-9_]*[a-z0-9]|[a-z0-9]` in `lower`, left to
/// right.
fn latin_words(lower: &str) -> Vec<&str> {
    let bytes = lower.as_bytes();
    let mut words = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte.is_ascii_lowercase() {
            let run = bytes[index..]
                .iter()
                .take_while(|byte| {
                    byte.is_ascii_lowercase() || byte.is_ascii_digit() || **byte == b'_'
                })
                .count();
            let word = lower[index..index + run].trim_end_matches('_');
            words.push(word);
            index += word.len();
        } else if byte.is_ascii_digit() {
            words.push(&lower[index..index + 1]);
            index += 1;
        } else {
            index += 1;
        }
    }
    words
}

fn cjk_runs(lower: &str) -> Vec<Vec<char>> {
    let mut runs = Vec::new();
    let mut current: Vec<char> = Vec::new();
    for character in lower.chars() {
        if is_cjk(character) {
            current.push(character);
        } else if !current.is_empty() {
            runs.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        runs.push(current);
    }
    runs
}
