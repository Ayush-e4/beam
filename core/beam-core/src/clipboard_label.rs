//! Telling links and one-time codes apart from other copied text, so the receiver can show a
//! link as a link and a code big, with a copy button, and keep codes out of history.
//!
//! The rules are deliberately narrow: a false "code" only changes how text is shown, but a
//! missed one is still plain text the user can copy.

/// Shortest and longest code, after dropping spaces and dashes.
const CODE_LEN: std::ops::RangeInclusive<usize> = 4..=10;
/// A code has at least this many digits, so words and names never qualify.
const MIN_CODE_DIGITS: usize = 4;
/// Longest text that is itself a code ("482 913", "G-482913").
const MAX_BARE_CODE_LEN: usize = 16;
/// Longest message searched for a code (a typical SMS or notification).
const MAX_MESSAGE_LEN: usize = 500;
/// Longest link, in bytes.
const MAX_LINK_LEN: usize = 2048;
/// Words that mark a message as carrying a code, matched in lowercase.
const CODE_WORDS: [&str; 7] = [
    "code",
    "otp",
    "passcode",
    "password",
    "pin",
    "verification",
    "one-time",
];

/// What a clipboard item looks like.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClipLabel {
    /// Any other text.
    Text,
    /// A single web link.
    Link,
    /// A one-time code, such as an OTP.
    Code,
    /// An image.
    Image,
}

/// Labels copied text: a lone `http(s)` link, a bare code, or plain text.
#[must_use]
pub fn label_text(text: &str) -> ClipLabel {
    let text = text.trim();
    if is_link(text) {
        ClipLabel::Link
    } else if text.len() <= MAX_BARE_CODE_LEN && as_code(text).is_some() {
        ClipLabel::Code
    } else {
        ClipLabel::Text
    }
}

/// The one-time code in `text`, ready to type: the text itself if it is a bare code, else the
/// first code-like word in a short message that mentions a code ("Your OTP is 482913").
/// Spaces, dashes and a letter prefix such as `G-` are dropped.
#[must_use]
pub fn find_code(text: &str) -> Option<String> {
    let text = text.trim();
    if text.len() <= MAX_BARE_CODE_LEN
        && let Some(code) = as_code(text)
    {
        return Some(code);
    }
    if text.len() > MAX_MESSAGE_LEN || !mentions_code(text) {
        return None;
    }
    text.split(|c: char| c.is_whitespace() || (c.is_ascii_punctuation() && c != '-'))
        .find_map(as_code)
}

fn is_link(text: &str) -> bool {
    let lower = text.get(..8).unwrap_or(text).to_ascii_lowercase();
    let rest = if lower.starts_with("https://") {
        &text[8..]
    } else if lower.starts_with("http://") {
        &text[7..]
    } else {
        return false;
    };
    text.len() <= MAX_LINK_LEN
        && rest.chars().next().is_some_and(char::is_alphanumeric)
        && !text.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// `word` as a code, or `None`.
fn as_code(word: &str) -> Option<String> {
    let word = strip_letter_prefix(word);
    let code: String = word.chars().filter(|c| *c != ' ' && *c != '-').collect();
    let digits = code.chars().filter(char::is_ascii_digit).count();
    let ok = CODE_LEN.contains(&code.len())
        && code.chars().all(|c| c.is_ascii_alphanumeric())
        && digits >= MIN_CODE_DIGITS
        && !word.starts_with('-')
        && !word.ends_with('-');
    ok.then_some(code)
}

/// "G-482913" → "482913": a short letter prefix before a dash names the service, it isn't
/// part of what is typed.
fn strip_letter_prefix(word: &str) -> &str {
    match word.split_once('-') {
        Some((prefix, rest))
            if (1..=3).contains(&prefix.len())
                && prefix.chars().all(|c| c.is_ascii_alphabetic())
                && rest.chars().all(|c| c.is_ascii_digit()) =>
        {
            rest
        }
        _ => word,
    }
}

fn mentions_code(text: &str) -> bool {
    let lower = text.to_lowercase();
    CODE_WORDS.iter().any(|word| lower.contains(word))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_are_single_http_urls() {
        for link in [
            "https://beam.app/x?y=1",
            "HTTP://example.com",
            " https://a.b \n",
        ] {
            assert_eq!(label_text(link), ClipLabel::Link, "{link:?}");
        }
        for text in [
            "https://",
            "https:// x",
            "see https://a.b",
            "ftp://a.b",
            "beam.app",
        ] {
            assert_ne!(label_text(text), ClipLabel::Link, "{text:?}");
        }
    }

    #[test]
    fn bare_codes_are_labelled_codes() {
        for code in [
            "482913", "482 913", "123-456", "G-482913", "AB12CD34", "4829",
        ] {
            assert_eq!(label_text(code), ClipLabel::Code, "{code:?}");
        }
        for text in [
            "hello",
            "abc1",
            "12",
            "12345678901",
            "-1234",
            "call me at 5pm",
        ] {
            assert_eq!(label_text(text), ClipLabel::Text, "{text:?}");
        }
    }

    #[test]
    fn codes_are_found_in_short_messages_that_mention_one() {
        let cases = [
            ("482913", Some("482913")),
            ("G-482913", Some("482913")),
            ("Your OTP is 482913. Do not share it.", Some("482913")),
            ("G-123456 is your Google verification code.", Some("123456")),
            ("Use code 1234-5678 to sign in", Some("12345678")),
            ("Meet at gate 4512 tomorrow", None),
            ("Your code is ready", None),
        ];
        for (text, code) in cases {
            assert_eq!(find_code(text).as_deref(), code, "{text:?}");
        }
        let long = format!("code 482913 {}", "x".repeat(MAX_MESSAGE_LEN));
        assert_eq!(find_code(&long), None);
    }
}
