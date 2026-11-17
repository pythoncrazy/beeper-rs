//! Desktop's links in text: linkifyjs (`Epe`) with its default options, which links URLs with a scheme and bare
//! domains on a known top-level domain (with `http://`), and leaves out the punctuation around a link.

use std::collections::HashSet;
use std::ops::Range;
use std::sync::LazyLock;

/// linkifyjs's top-level domains, ASCII then international, in its own packed form: letters push onto a word, and a
/// number ends the word and pops that many letters.
static TLDS: LazyLock<HashSet<String>> = LazyLock::new(|| {
    let mut words = HashSet::new();
    for packed in include_str!("../assets/tlds.txt").lines() {
        let (mut stack, mut digits) = (Vec::<char>::new(), String::new());
        for c in packed.chars().chain(['\0']) {
            if c.is_ascii_digit() {
                digits.push(c);
                continue;
            }
            if !digits.is_empty() {
                words.insert(stack.iter().collect::<String>());
                let pops: usize = digits.parse().expect("a count");
                stack.truncate(stack.len().saturating_sub(pops));
                digits.clear();
            }
            if c != '\0' {
                stack.push(c);
            }
        }
    }
    words
});

/// The links of `text`: where each is, and where it goes.
pub fn links(text: &str) -> Vec<(Range<usize>, String)> {
    let mut found = Vec::new();
    let mut at = 0;
    for token in text.split(char::is_whitespace) {
        let start = at;
        at += token.len() + text[at + token.len()..].chars().next().map_or(0, char::len_utf8);
        if let Some((range, href)) = link(token) {
            found.push((start + range.start..start + range.end, href));
        }
    }
    found
}

/// The link in a word, without the punctuation around it.
pub fn link(word: &str) -> Option<(Range<usize>, String)> {
    let lead = word.len() - word.trim_start_matches(['(', '[', '{', '<', '"', '\'']).len();
    let mut end = word.len();
    loop {
        let core = &word[lead..end];
        let last = core.chars().next_back()?;
        let open = |o: char| core.matches(o).count() >= core.matches(last).count();
        let drop = match last {
            '.' | ',' | ';' | ':' | '!' | '?' | '"' | '\'' => true,
            ')' => !open('('),
            ']' => !open('['),
            '}' => !open('{'),
            '>' => true,
            _ => false,
        };
        if !drop {
            break;
        }
        end -= last.len_utf8();
    }
    let core = &word[lead..end];
    let lower = core.to_lowercase();
    let href = if ["http://", "https://"].iter().any(|s| lower.starts_with(s) && lower.len() > s.len()) {
        core.to_string()
    } else if host(core) {
        format!("http://{core}")
    } else {
        return None;
    };
    Some((lead..end, href))
}

/// Whether `s` starts with a domain on a known top-level domain, followed by a port, a path, a query or nothing.
fn host(s: &str) -> bool {
    let name = s.split(['/', '?', '#', ':']).next().unwrap_or_default();
    if s.contains('@') || name.len() < s.len() && !s[name.len()..].starts_with(['/', '?', '#', ':']) {
        return false;
    }
    let labels: Vec<&str> = name.split('.').collect();
    let ok = |l: &&str| {
        !l.is_empty() && !l.starts_with('-') && !l.ends_with('-') && l.chars().all(|c| c.is_alphanumeric() || c == '-')
    };
    labels.len() >= 2 && labels.iter().all(ok) && TLDS.contains(&labels[labels.len() - 1].to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_like_linkifyjs() {
        let hrefs = |t: &str| links(t).into_iter().map(|(r, h)| (t[r].to_string(), h)).collect::<Vec<_>>();
        let pair = |a: &str, b: &str| (a.to_string(), b.to_string());
        assert_eq!(hrefs("see example.com."), [pair("example.com", "http://example.com")]);
        assert_eq!(hrefs("(www.x.org/a?b=1)"), [pair("www.x.org/a?b=1", "http://www.x.org/a?b=1")]);
        assert_eq!(
            hrefs("https://en.wikipedia.org/wiki/Rust_(language)!"),
            [pair("https://en.wikipedia.org/wiki/Rust_(language)", "https://en.wikipedia.org/wiki/Rust_(language)")]
        );
        assert_eq!(hrefs("read file.txt, e.g. this, or a.b"), []);
        assert_eq!(hrefs("mail me@x.com"), [], "an address is not a link here");
        assert_eq!(
            hrefs("ünï.de\nbeeper.com:8080/x"),
            [pair("ünï.de", "http://ünï.de"), pair("beeper.com:8080/x", "http://beeper.com:8080/x")]
        );
        assert!(TLDS.contains("com") && TLDS.contains("рф") && !TLDS.contains("txt"));
    }
}
