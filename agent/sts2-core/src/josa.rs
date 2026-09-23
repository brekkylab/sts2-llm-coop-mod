//! Picks Korean particles by the preceding syllable's final consonant, since
//! humans read the briefing too.

/// Hangul syllables are laid out as `0xAC00 + (initial*21 + medial)*28 + final`, so
/// the remainder mod 28 is the final consonant (0 = none).
fn has_final_consonant(word: &str) -> Option<bool> {
    let last = word.chars().rev().find(|c| !c.is_whitespace())?;
    let code = last as u32;
    if (0xAC00..=0xD7A3).contains(&code) {
        return Some((code - 0xAC00) % 28 != 0);
    }
    // Digits follow their Korean reading; 0 1 3 6 7 8 end in a consonant.
    if let Some(d) = last.to_digit(10) {
        return Some(matches!(d, 0 | 1 | 3 | 6 | 7 | 8));
    }
    None
}

/// Neither Hangul nor a digit: the first form.
fn pick<'a>(word: &str, with_final: &'a str, without: &'a str) -> &'a str {
    match has_final_consonant(word) {
        Some(true) => with_final,
        Some(false) => without,
        None => with_final,
    }
}

pub fn eul(word: &str) -> &'static str {
    pick(word, "을", "를")
}

pub fn eun(word: &str) -> &'static str {
    pick(word, "은", "는")
}

pub fn i_ga(word: &str) -> &'static str {
    pick(word, "이", "가")
}

/// Copula form, as in "6이라" / "12라".
pub fn ira(word: &str) -> &'static str {
    pick(word, "이라", "라")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_by_final_consonant() {
        assert_eq!(eul("압축벌레"), "를");
        assert_eq!(eul("슬라임"), "을");
        assert_eq!(eun("타격"), "은");
        assert_eq!(eun("수비"), "는");
        assert_eq!(i_ga("동료"), "가");
        assert_eq!(i_ga("사람"), "이");
    }

    #[test]
    fn reads_digits_aloud() {
        assert_eq!(ira("6"), "이라");   // yuk
        assert_eq!(ira("12"), "라");    // sip-i
        assert_eq!(ira("80"), "이라");  // pal-sip
        assert_eq!(ira("5"), "라");     // o
    }

    #[test]
    fn falls_back_for_other_scripts() {
        assert_eq!(eul("Strike"), "을");
    }
}
