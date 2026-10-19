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

/// A format of Desktop's `Intl.DateTimeFormat` with the app language.
#[derive(Clone, Copy)]
pub enum Date {
    MonthDay = 2,
    Medium,
    Month,
    MonthYear,
    Numeric,
    Am,
    Pm,
    Clock24,
}

/// The month names, the short month names and the [`Date`] patterns of English and of each language of [`LANGS`].
const DATES: [[&str; 10]; 6] = [
    [
        "January February March April May June July August September October November December",
        "Jan Feb Mar Apr May Jun Jul Aug Sep Oct Nov Dec",
        "{MMMM} {d}",
        "{MMM} {d}, {y}",
        "{MMMM}",
        "{MMMM} {y}",
        "{MM}/{dd}/{y}",
        "{h}:{mm} AM",
        "{h}:{mm} PM",
        "{HH}:{mm}",
    ],
    [
        "Januar Februar März April Mai Juni Juli August September Oktober November Dezember",
        "Jan. Feb. März Apr. Mai Juni Juli Aug. Sept. Okt. Nov. Dez.",
        "{d}. {MMMM}",
        "{d}. {MMM} {y}",
        "{MMMM}",
        "{MMMM} {y}",
        "{dd}.{MM}.{y}",
        "{h}:{mm} AM",
        "{h}:{mm} PM",
        "{HH}:{mm}",
    ],
    [
        "enero febrero marzo abril mayo junio julio agosto septiembre octubre noviembre diciembre",
        "ene feb mar abr may jun jul ago sept oct nov dic",
        "{d} de {MMMM}",
        "{d} {MMM} {y}",
        "{MMMM}",
        "{MMMM} de {y}",
        "{dd}/{MM}/{y}",
        "{h}:{mm} a.\u{a0}m.",
        "{h}:{mm} p.\u{a0}m.",
        "{H}:{mm}",
    ],
    [
        "janvier février mars avril mai juin juillet août septembre octobre novembre décembre",
        "janv. févr. mars avr. mai juin juil. août sept. oct. nov. déc.",
        "{d} {MMMM}",
        "{d} {MMM} {y}",
        "{MMMM}",
        "{MMMM} {y}",
        "{dd}/{MM}/{y}",
        "{h}:{mm} AM",
        "{h}:{mm} PM",
        "{HH}:{mm}",
    ],
    [
        "",
        "",
        "{M}月{d}日",
        "{y}年{M}月{d}日",
        "{M}月",
        "{y}年{M}月",
        "{y}/{MM}/{dd}",
        "午前{h}:{mm}",
        "午後{h}:{mm}",
        "{H}:{mm}",
    ],
    [
        "janeiro fevereiro março abril maio junho julho agosto setembro outubro novembro dezembro",
        "jan. fev. mar. abr. mai. jun. jul. ago. set. out. nov. dez.",
        "{d} de {MMMM}",
        "{d} de {MMM} de {y}",
        "{MMMM}",
        "{MMMM} de {y}",
        "{dd}/{MM}/{y}",
        "{h}:{mm} AM",
        "{h}:{mm} PM",
        "{HH}:{mm}",
    ],
];

/// A local date and time in the format `f` of the app language, like Desktop's `Intl.DateTimeFormat` with `k3()`.
pub fn date(f: Date, ymd: (i64, u32, u32), hm: (i64, i64)) -> String {
    fill(&DATES[CURRENT.load(Ordering::Relaxed).wrapping_add(1)], f, ymd, hm)
}

/// [`date`] in English.
pub fn en_date(f: Date, ymd: (i64, u32, u32), hm: (i64, i64)) -> String {
    fill(&DATES[0], f, ymd, hm)
}

/// The English short name of month `m`, from 1.
pub fn en_short_month(m: u32) -> &'static str {
    DATES[0][1].split(' ').nth(m as usize - 1).expect("a month name")
}

fn fill(lang: &[&str; 10], f: Date, (y, m, d): (i64, u32, u32), (h, min): (i64, i64)) -> String {
    let name = |i: usize| lang[i].split(' ').nth(m as usize - 1).expect("a month name").to_string();
    let mut out = String::new();
    let mut rest = lang[f as usize];
    while let Some((head, tail)) = rest.split_once('{') {
        let (token, tail) = tail.split_once('}').expect("a closed token");
        out.push_str(head);
        out.push_str(&match token {
            "MMMM" => name(0),
            "MMM" => name(1),
            "M" => m.to_string(),
            "MM" => format!("{m:02}"),
            "d" => d.to_string(),
            "dd" => format!("{d:02}"),
            "y" => y.to_string(),
            "h" => ((h + 11) % 12 + 1).to_string(),
            "H" => h.to_string(),
            "HH" => format!("{h:02}"),
            "mm" => format!("{min:02}"),
            _ => unreachable!("date token {token}"),
        });
        rest = tail;
    }
    out + rest
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

    /// The `Intl.DateTimeFormat` output of Node for 2025-10-10 09:05 and 2025-05-03 21:45, without regions.
    #[test]
    fn dates_match_intl() {
        use super::Date::*;
        let expected = [
            [
                "October 10",
                "Oct 10, 2025",
                "October",
                "October 2025",
                "10/10/2025",
                "9:05 AM",
                "09:05",
                "9:45 PM",
                "May 3, 2025",
            ],
            [
                "10. Oktober",
                "10. Okt. 2025",
                "Oktober",
                "Oktober 2025",
                "10.10.2025",
                "9:05 AM",
                "09:05",
                "9:45 PM",
                "3. Mai 2025",
            ],
            [
                "10 de octubre",
                "10 oct 2025",
                "octubre",
                "octubre de 2025",
                "10/10/2025",
                "9:05 a.\u{a0}m.",
                "9:05",
                "9:45 p.\u{a0}m.",
                "3 may 2025",
            ],
            [
                "10 octobre",
                "10 oct. 2025",
                "octobre",
                "octobre 2025",
                "10/10/2025",
                "9:05 AM",
                "09:05",
                "9:45 PM",
                "3 mai 2025",
            ],
            [
                "10月10日",
                "2025年10月10日",
                "10月",
                "2025年10月",
                "2025/10/10",
                "午前9:05",
                "9:05",
                "午後9:45",
                "2025年5月3日",
            ],
            [
                "10 de outubro",
                "10 de out. de 2025",
                "outubro",
                "outubro de 2025",
                "10/10/2025",
                "9:05 AM",
                "09:05",
                "9:45 PM",
                "3 de mai. de 2025",
            ],
        ];
        for (lang, want) in super::DATES.iter().zip(expected) {
            let (oct, may) = (((2025, 10, 10), (9, 5)), ((2025, 5, 3), (21, 45)));
            let got =
                [MonthDay, Medium, Month, MonthYear, Numeric, Am, Clock24].map(|f| super::fill(lang, f, oct.0, oct.1));
            assert_eq!(got[..], want[..7]);
            assert_eq!([super::fill(lang, Pm, may.0, may.1), super::fill(lang, Medium, may.0, may.1)], want[7..]);
        }
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
