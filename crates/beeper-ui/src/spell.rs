//! Desktop's composer spellcheck ("Enable spellcheck"), with the system Hunspell dictionary of the locale.

use std::ops::Range;
use std::sync::OnceLock;

static DICT: OnceLock<Option<spellbook::Dictionary>> = OnceLock::new();

/// Load the dictionary of `$LANG` (en_US for the C locale). Like Desktop, a changed setting applies after a relaunch.
pub fn load() {
    DICT.get_or_init(|| {
        let lang = std::env::var("LANG").unwrap_or_default();
        let lang = lang.split('.').next().filter(|l| l.contains('_')).unwrap_or("en_US");
        let home = std::env::var("HOME").map(|h| h + "/.local/share/hunspell").unwrap_or_default();
        let found = ["/usr/share/hunspell", "/usr/share/myspell", &home].into_iter().find_map(|dir| {
            let read = |ext| std::fs::read_to_string(format!("{dir}/{lang}.{ext}")).ok();
            Some((read("aff")?, read("dic")?))
        })?;
        spellbook::Dictionary::new(&found.0, &found.1).map_err(|e| eprintln!("spellcheck dictionary {lang}: {e}")).ok()
    });
}

/// The byte ranges of the misspelled words of `text`. Links, mentions, codes and the word at `caret` are not checked.
pub fn misspelled(text: &str, caret: usize) -> Vec<Range<usize>> {
    let Some(dict) = DICT.get().and_then(Option::as_ref) else { return Vec::new() };
    let at = |s: &str| s.as_ptr() as usize - text.as_ptr() as usize;
    text.split(char::is_whitespace)
        .filter(|t| !t.contains(['@', '/', ':', '#']) && !t.contains(|c: char| c.is_ascii_digit()))
        .flat_map(|t| t.split(|c: char| !c.is_alphabetic() && c != '\''))
        .map(|w| w.trim_matches('\''))
        .filter(|w| !w.is_empty() && at(w) + w.len() != caret && !dict.check(w))
        .map(|w| at(w)..at(w) + w.len())
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn finds_misspelled_words() {
        let dict = spellbook::Dictionary::new("SET UTF-8\n", "3\nhello\nworld\nit's\n").unwrap();
        assert!(super::DICT.set(Some(dict)).is_ok());
        let text = "hello wrold, it's https://x.io @bob wrold teh";
        assert_eq!(super::misspelled(text, text.len()), [6..11, 36..41]);
    }
}
