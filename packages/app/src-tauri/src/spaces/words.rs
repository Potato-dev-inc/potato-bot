//! Invite words: 11 secret bytes <-> 8 words from the BIP-39 English list (2048 words, 11 bits each).
use std::sync::OnceLock;

static RAW: &str = include_str!("words/en.txt");
pub const WORD_COUNT: usize = 2048;
pub const CODE_WORDS: usize = 8;
pub const SECRET_BYTES: usize = 11;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WordsError {
    /// How many words were typed (not 8).
    Count(usize),
    /// 1-based position and the typed word.
    Unknown(usize, String),
}

pub fn list() -> &'static [&'static str] {
    static LIST: OnceLock<Vec<&'static str>> = OnceLock::new();
    LIST.get_or_init(|| RAW.lines().map(str::trim).filter(|w| !w.is_empty()).collect())
}

pub fn encode(secret: &[u8; SECRET_BYTES]) -> [&'static str; CODE_WORDS] {
    let l = list();
    let mut out = [""; CODE_WORDS];
    let (mut acc, mut bits, mut i) = (0u32, 0u32, 0usize);
    for &b in secret {
        acc = (acc << 8) | b as u32;
        bits += 8;
        while bits >= 11 {
            bits -= 11;
            out[i] = l[((acc >> bits) & 0x7ff) as usize];
            i += 1;
        }
        acc &= (1 << bits) - 1;
    }
    out
}

fn index_of(word: &str) -> Option<usize> {
    let l = list();
    if let Ok(i) = l.binary_search(&word) { return Some(i); }
    // A unique 4-letter (or longer) prefix is accepted: "abandon" can be typed "aban".
    if word.len() >= 4 {
        let hits: Vec<usize> = (0..l.len()).filter(|&i| l[i].starts_with(word)).collect();
        if hits.len() == 1 { return Some(hits[0]); }
    }
    None
}

pub fn decode(words: &[&str]) -> Result<[u8; SECRET_BYTES], WordsError> {
    if words.len() != CODE_WORDS { return Err(WordsError::Count(words.len())); }
    let mut out = [0u8; SECRET_BYTES];
    let (mut acc, mut bits, mut o) = (0u32, 0u32, 0usize);
    for (pos, w) in words.iter().enumerate() {
        let idx = index_of(w).ok_or_else(|| WordsError::Unknown(pos + 1, w.to_string()))?;
        acc = (acc << 11) | idx as u32;
        bits += 11;
        while bits >= 8 {
            bits -= 8;
            out[o] = ((acc >> bits) & 0xff) as u8;
            o += 1;
        }
        acc &= (1 << bits) - 1;
    }
    Ok(out)
}

/// What a person typed: any mix of spaces, dashes, commas, dots and new lines between words; any case.
pub fn parse_typed(text: &str) -> Result<[u8; SECRET_BYTES], WordsError> {
    let lower = text.to_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| c.is_whitespace() || c == '-' || c == ',' || c == '.')
        .filter(|w| !w.is_empty())
        .collect();
    decode(&words)
}

/// Up to 5 list words starting with `prefix` (for the word field's suggestions).
pub fn suggest(prefix: &str) -> Vec<&'static str> {
    let p = prefix.trim().to_lowercase();
    if p.is_empty() { return Vec::new(); }
    list().iter().copied().filter(|w| w.starts_with(&p)).take(5).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_has_2048_sorted_unique_words() {
        let l = list();
        assert_eq!(l.len(), WORD_COUNT);
        assert!(l.windows(2).all(|w| w[0] < w[1]));
        assert!(l.iter().all(|w| (3..=8).contains(&w.len()) && w.bytes().all(|c| c.is_ascii_lowercase())));
    }
    #[test]
    fn first_four_letters_are_unique() {
        let mut p: Vec<&str> = list().iter().map(|w| &w[..w.len().min(4)]).collect();
        p.sort();
        p.dedup();
        assert_eq!(p.len(), WORD_COUNT);
    }
    #[test]
    fn zero_secret_is_eight_times_the_first_word() {
        assert_eq!(encode(&[0u8; 11]), [list()[0]; 8]);
    }
    #[test]
    fn all_ones_secret_is_eight_times_the_last_word() {
        assert_eq!(encode(&[0xffu8; 11]), [list()[2047]; 8]);
    }
    #[test]
    fn round_trip() {
        let s: [u8; 11] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];
        assert_eq!(decode(&encode(&s)).unwrap(), s);
    }
    #[test]
    fn typed_with_dashes_caps_and_prefixes_works() {
        let s: [u8; 11] = [200, 1, 77, 3, 99, 5, 250, 7, 8, 9, 10];
        let w = encode(&s);
        let typed = format!(
            "{}-{} {},{}\n{} {} {} {}",
            w[0].to_uppercase(), &w[1][..4.min(w[1].len())], w[2], w[3], w[4], w[5], w[6], w[7]
        );
        assert_eq!(parse_typed(&typed).unwrap(), s);
    }
    #[test]
    fn wrong_count_and_unknown_word_are_reported() {
        assert_eq!(parse_typed("one two"), Err(WordsError::Count(2)));
        let w = encode(&[0u8; 11]);
        let typed = format!("{} {} zzzzzz {} {} {} {} {}", w[0], w[1], w[3], w[4], w[5], w[6], w[7]);
        assert_eq!(parse_typed(&typed), Err(WordsError::Unknown(3, "zzzzzz".into())));
    }
    #[test]
    fn suggest_gives_at_most_five() {
        assert!(suggest("a").len() <= 5);
        assert!(suggest("").is_empty());
        assert_eq!(suggest(list()[10]), vec![list()[10]]);
    }
}
