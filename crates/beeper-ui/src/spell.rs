//! Desktop's composer spellcheck ("Enable spellcheck"), with the system Hunspell dictionary of the locale.

use std::ops::Range;
use std::path::PathBuf;
use std::sync::{OnceLock, RwLock};
use unicode_script::{Script, UnicodeScript};

/// The dictionary, its script, and Chromium's "Custom Dictionary.txt" of the words added with "Learn Spelling".
type Dict = (RwLock<spellbook::Dictionary>, Script, PathBuf);
static DICT: OnceLock<Option<Dict>> = OnceLock::new();

/// The script of the first letter of the first word of a `.dic` file.
fn script(dic: &str) -> Script {
    dic.lines().nth(1).and_then(|w| w.chars().find(|c| c.is_alphabetic())).expect("a dictionary has a word").script()
}

/// Load the dictionary of `$LANG` (en_US for the C locale) and the learned words in `custom`. Like Desktop, a changed
/// setting applies after a relaunch.
pub fn load(custom: PathBuf) {
    DICT.get_or_init(|| {
        let lang = std::env::var("LANG").unwrap_or_default();
        let lang = lang.split('.').next().filter(|l| l.contains('_')).unwrap_or("en_US");
        let home = std::env::var("HOME").map(|h| h + "/.local/share/hunspell").unwrap_or_default();
        let found = ["/usr/share/hunspell", "/usr/share/myspell", &home].into_iter().find_map(|dir| {
            let read = |ext| std::fs::read_to_string(format!("{dir}/{lang}.{ext}")).ok();
            Some((read("aff")?, read("dic")?))
        })?;
        let mut dict = spellbook::Dictionary::new(&found.0, &found.1)
            .map_err(|e| eprintln!("spellcheck dictionary {lang}: {e}"))
            .ok()?;
        for word in std::fs::read_to_string(&custom).unwrap_or_default().lines() {
            dict.add(word).expect("a learned word has no flags");
        }
        Some((RwLock::new(dict), script(&found.1), custom))
    });
}

/// Chromium's suggestions for a misspelled word, at most 5.
pub fn suggest(word: &str) -> Vec<String> {
    let mut out = Vec::new();
    let (dict, _, _) = DICT.get().and_then(Option::as_ref).expect("a misspelled word has a dictionary");
    dict.read().unwrap().suggest(word, &mut out);
    out.truncate(5);
    out
}

/// "Learn Spelling": adds `word` to the dictionary and to the custom dictionary file.
pub fn learn(word: &str) -> std::io::Result<()> {
    let (dict, _, custom) = DICT.get().and_then(Option::as_ref).expect("a misspelled word has a dictionary");
    dict.write().unwrap().add(word).expect("a word has no flags");
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(custom)?;
    std::io::Write::write_all(&mut file, format!("{word}\n").as_bytes())
}

/// The byte ranges of the misspelled words of `text`. Links, mentions, codes, the word at `caret` and, like Chromium,
/// words with letters of an other script than the dictionary are not checked.
pub fn misspelled(text: &str, caret: usize) -> Vec<Range<usize>> {
    let Some((dict, script, _)) = DICT.get().and_then(Option::as_ref) else { return Vec::new() };
    let dict = dict.read().unwrap();
    let at = |s: &str| s.as_ptr() as usize - text.as_ptr() as usize;
    text.split(char::is_whitespace)
        .filter(|t| !t.contains(['@', '/', ':', '#']) && !t.contains(|c: char| c.is_ascii_digit()))
        .flat_map(|t| t.split(|c: char| !c.is_alphabetic() && c != '\''))
        .map(|w| w.trim_matches('\''))
        .filter(|w| !w.is_empty() && w.chars().all(|c| !c.is_alphabetic() || c.script() == *script))
        .filter(|w| at(w) + w.len() != caret && !dict.check(w))
        .map(|w| at(w)..at(w) + w.len())
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn finds_misspelled_words() {
        let dict =
            spellbook::Dictionary::new("SET UTF-8\nTRY esianrtolcdugmphbyfvkwz\n", "3\nhello\nworld\nit's\n").unwrap();
        let custom = std::env::temp_dir().join(format!("beeper-rs-spell-{}.txt", std::process::id()));
        assert!(super::DICT.set(Some((dict.into(), super::script("3\nhello\n"), custom.clone()))).is_ok());
        let text = "hello wrold, it's https://x.io @bob wrold 日本語の 한국어 привет teh";
        assert_eq!(super::misspelled(text, text.len()), [6..11, 36..41]);
        assert_eq!(super::suggest("wrold"), ["world"]);
        super::learn("wrold").unwrap();
        assert!(super::misspelled(text, text.len()).is_empty());
        assert_eq!(std::fs::read_to_string(&custom).unwrap(), "wrold\n");
        std::fs::remove_file(custom).unwrap();
    }
}
