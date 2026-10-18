//! Beeper Desktop's translations of the UI. `scripts/i18n.py` copies them from Desktop.

use std::collections::HashMap;
use std::fmt::Display;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;

/// Desktop's languages other than English, in the order of the "App language" options after "System" and "English".
const LANGS: [(&str, &str); 5] = [
    ("de", include_str!("../assets/i18n/de.json")),
    ("es", include_str!("../assets/i18n/es.json")),
    ("fr", include_str!("../assets/i18n/fr.json")),
    ("ja", include_str!("../assets/i18n/ja.json")),
    ("pt-BR", include_str!("../assets/i18n/pt-BR.json")),
];

/// The plural forms of each translation, for each language of [`LANGS`].
static TABLES: [OnceLock<HashMap<String, Vec<String>>>; 5] = [const { OnceLock::new() }; 5];

/// The index in [`LANGS`] of the current language, or `usize::MAX` for English.
static CURRENT: AtomicUsize = AtomicUsize::new(usize::MAX);

/// Uses the language of the "App language" option `choice`, where 0 is the system language and 1 is English.
pub fn set(choice: u8) {
    let locale = ["LANGUAGE", "LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()));
    let index = match choice {
        0 => locale
            .as_deref()
            .and_then(language)
            .map(|c| LANGS.iter().position(|l| l.0 == c).expect("a listed language")),
        1 => None,
        i => Some(usize::from(i) - 2),
    };
    CURRENT.store(index.unwrap_or(usize::MAX), Ordering::Relaxed);
}

/// The current language and its translations, or `None` for English.
fn table() -> Option<(&'static str, &'static HashMap<String, Vec<String>>)> {
    let i = CURRENT.load(Ordering::Relaxed);
    let (code, json) = LANGS.get(i)?;
    Some((code, TABLES[i].get_or_init(|| serde_json::from_str(json).expect("valid translations"))))
}

/// Desktop's language for a system locale such as `de_DE.UTF-8`, or `None` for English.
fn language(locale: &str) -> Option<&'static str> {
    let tag = locale.split([':', '.', '@']).next().expect("split yields an item").replace('_', "-").to_lowercase();
    match tag.split('-').next().expect("split yields an item") {
        "pt" => Some("pt-BR"),
        base => LANGS.iter().map(|l| l.0).find(|c| *c == base),
    }
}

fn forms(key: &str) -> Option<&'static [String]> {
    table().and_then(|t| t.1.get(key)).map(Vec::as_slice)
}

/// The translation of a UI string.
pub fn tr(s: &'static str) -> &'static str {
    forms(s).map_or(s, |f| &f[0])
}

/// The translation of a UI string that Desktop translates by context. `scripts/i18n.py` merges the contexts with
/// the same translation into the plain string.
pub fn trx(s: &'static str, context: &str) -> &'static str {
    forms(&format!("{context}\u{4}{s}")).or_else(|| forms(s)).map_or(s, |f| &f[0])
}

/// The translation of a UI string with `printf` placeholders such as `%s`, `%d` and `%2$s`, filled with `args`.
pub fn trf(s: &'static str, args: &[&dyn Display]) -> String {
    sprintf(tr(s), args)
}

/// The translation of a settings text, with its placeholders filled with `args`.
pub fn trs(s: &'static str, args: &[&str]) -> String {
    sprintf(tr(s), &args.iter().map(|a| a as &dyn Display).collect::<Vec<_>>())
}

/// The plural form for `n` of a UI string with placeholders, filled with `args`.
pub fn trn(one: &'static str, many: &'static str, n: usize, args: &[&dyn Display]) -> String {
    plural(one, [one, many], n, args)
}

/// The plural form for `n` of a UI string that Desktop translates by context.
pub fn trnx(one: &'static str, many: &'static str, n: usize, context: &str, args: &[&dyn Display]) -> String {
    plural(&format!("{context}\u{4}{one}"), [one, many], n, args)
}

fn plural(key: &str, english: [&str; 2], n: usize, args: &[&dyn Display]) -> String {
    let form = match table().map_or("en", |t| t.0) {
        "ja" => 0,
        "fr" => usize::from(n > 1),
        _ => usize::from(n != 1),
    };
    sprintf(forms(key).map_or(english[form], |f| &f[form]), args)
}

fn sprintf(s: &str, args: &[&dyn Display]) -> String {
    let (mut out, mut next) = (String::new(), 0);
    let mut parts = s.split('%');
    out.push_str(parts.next().expect("split yields an item"));
    for part in parts {
        let digits = part.find(|c: char| !c.is_ascii_digit()).expect("a placeholder ends with its type");
        let (index, rest) = match part[digits..].strip_prefix('$') {
            Some(rest) => (part[..digits].parse::<usize>().expect("a placeholder position") - 1, rest),
            None => (next, part),
        };
        assert!(rest.starts_with(['s', 'd']), "the placeholder %{part} has the type s or d");
        out.push_str(&args[index].to_string());
        out.push_str(&rest[1..]);
        next += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn system_locales() {
        let langs = ["de_DE.UTF-8", "pt_PT", "pt_BR.UTF-8", "ja", "fr_CA@euro", "es:en", "en_US.UTF-8", "C", "zh_CN"];
        let want = [Some("de"), Some("pt-BR"), Some("pt-BR"), Some("ja"), Some("fr"), Some("es"), None, None, None];
        assert_eq!(langs.map(super::language), want);
    }

    #[test]
    fn placeholders() {
        assert_eq!(super::sprintf("%1$s um %2$s Uhr", &[&"Gesendet", &12]), "Gesendet um 12 Uhr");
        assert_eq!(super::sprintf("%d chats, %s", &[&3, &"x"]), "3 chats, x");
        assert_eq!(super::trn("%d chat selected", "%d chats selected", 2, &[&2]), "2 chats selected");
    }

    #[test]
    fn contexts_are_translated() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src");
        let mut calls = 0;
        for file in std::fs::read_dir(dir).unwrap() {
            let src = std::fs::read_to_string(file.unwrap().path()).unwrap();
            for (call, n) in [("trx(\"", 2), ("trnx(\"", 3)] {
                for (at, _) in src.match_indices(call).filter(|(at, _)| !src[..*at].ends_with("fn ")) {
                    let args: Vec<&str> = src[at + call.len() - 1..].split('"').skip(1).step_by(2).take(n).collect();
                    let key = format!("{}\u{4}{}", args[n - 1], args[0]);
                    for (lang, json) in super::LANGS {
                        let table: std::collections::HashMap<String, Vec<String>> = serde_json::from_str(json).unwrap();
                        assert!(table.contains_key(&key) || table.contains_key(args[0]), "{lang} has no {key:?}");
                    }
                    calls += 1;
                }
            }
        }
        assert!(calls > 5, "found {calls} calls");
    }
}
