//! Desktop's chat search index `Rd`, behind `searchTitlesOrNetworkDetailed` and `searchParticipantsDetailed`: the
//! words of a query match the search fields of a chat as substrings, and words of four or more characters also as the
//! fuzzy matches of Fuse.js 7.1.0.

use beeper_core::config::account_network;
use beeper_core::models::{member_order, Chat};
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;
use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};
use unicode_properties::{GeneralCategoryGroup, UnicodeGeneralCategory};

/// Desktop's `ft`: the Latin letters of the accented and the Cyrillic letters of its table `yd`.
fn latin(s: &str) -> String {
    static TABLE: OnceLock<HashMap<char, &str>> = OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let entry = |l: &'static str| l.split_once('\t').map(|(k, v)| (k.chars().next().expect("a character"), v));
        include_str!("latin.tsv").lines().map(|l| entry(l).expect("a tab-separated entry")).collect()
    });
    s.chars().fold(String::with_capacity(s.len()), |mut out, c| {
        match table.get(&c) {
            Some(v) => out.push_str(v),
            None => out.push(c),
        }
        out
    })
}

/// JavaScript's `\s`.
fn js_space(c: char) -> bool {
    (c.is_whitespace() && c != '\u{85}') || c == '\u{feff}'
}

/// JavaScript's `String.prototype.trim`.
pub(super) fn js_trim(s: &str) -> &str {
    s.trim_matches(js_space)
}

/// Desktop's `Id`: the words of a query, without its punctuation.
fn words(query: &str) -> Vec<String> {
    let lower = latin(query).to_lowercase();
    let spaced: String =
        lower.chars().map(|c| if "|!'^=,;?(){}[]\"`~#$%&*+<>\\".contains(c) { ' ' } else { c }).collect();
    spaced.split(js_space).filter(|w| !w.is_empty()).map(str::to_string).collect()
}

/// Desktop's `xd`: the whole query, lowercase with single spaces.
fn phrase(query: &str) -> String {
    let lower = latin(query).to_lowercase();
    lower.split(js_space).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ")
}

/// Desktop's `Ed`: whether the query has a character other than letters, digits, `_`, `-` and spaces.
fn has_symbol(phrase: &str) -> bool {
    phrase.chars().any(|c| {
        let group = c.general_category_group();
        !(group == GeneralCategoryGroup::Letter || group == GeneralCategoryGroup::Number)
            && !matches!(c, '_' | '-')
            && !js_space(c)
    })
}

/// The search fields of a chat, Desktop's `Sd`: its title, its network, and the names, usernames, emails and phone
/// digits of its first 50 present participants in Desktop's order, each in Latin letters and lowercase.
struct Fields {
    title: String,
    network: String,
    participants: Vec<String>,
}

impl Fields {
    fn of(c: &Chat) -> Self {
        let lower = |s: &str| latin(s).to_lowercase();
        let mut members: Vec<_> = c.participants.items.iter().filter(|u| u.has_exited != Some(true)).collect();
        members.sort_by(|a, b| member_order(a, b));
        let person = |u: &beeper_core::models::User| {
            let digits = u.phone_number.as_ref().map(|p| p.chars().filter(char::is_ascii_digit).collect::<String>());
            let names = [&u.full_name, &u.custom_nickname, &u.username, &u.email, &digits];
            let names: Vec<String> = names.into_iter().flatten().filter(|n| !n.is_empty()).map(|n| lower(n)).collect();
            (!names.is_empty()).then(|| names.join(" "))
        };
        let network = c.network_name.as_deref().unwrap_or(account_network(&c.account_id).display_name);
        Fields {
            title: lower(&c.title),
            network: lower(network),
            participants: members.into_iter().take(50).filter_map(person).collect(),
        }
    }

    fn values(&self, field: Field) -> &[String] {
        match field {
            Field::Title => std::slice::from_ref(&self.title),
            Field::Network => std::slice::from_ref(&self.network),
            Field::Participants => &self.participants,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Field {
    Title,
    Network,
    Participants,
}

/// Desktop's `jr`: whether a value of the field contains the trimmed word, as an exact, a prefix, a word, a token prefix
/// or a substring match.
fn contains(values: &[String], word: &str) -> bool {
    let word = js_trim(word);
    values.iter().any(|v| !v.is_empty() && !word.is_empty() && latin(v).to_lowercase().contains(word))
}

/// The UTF-16 code units of a text for Fuse.js, lowercase and without diacritics (`ignoreDiacritics`).
fn fuse_text(s: &str) -> Vec<u16> {
    s.to_lowercase().nfd().filter(|&c| !is_combining_mark(c)).collect::<String>().encode_utf16().collect()
}

/// Fuse.js's `MAX_BITS`, the longest pattern of a bitap search.
const MAX_BITS: usize = 32;

/// Fuse.js's `BitapSearch.searchIn` with Desktop's options (`ignoreLocation`, a threshold of 0.32): whether `pattern`
/// matches `text`, in chunks of 32 code units.
fn fuse_match(pattern: &str, text: &str) -> bool {
    let (pattern, text) = (fuse_text(pattern), fuse_text(text));
    if pattern == text {
        return true;
    }
    let len = pattern.len();
    let starts: Vec<usize> = match len {
        0 => Vec::new(),
        1..=MAX_BITS => vec![0],
        _ => {
            (0..len - len % MAX_BITS).step_by(MAX_BITS).chain((len % MAX_BITS != 0).then_some(len - MAX_BITS)).collect()
        }
    };
    starts.into_iter().any(|at| bitap(&text, &pattern[at..len.min(at + MAX_BITS)], at))
}

/// Fuse.js's `search` of a bitap chunk with `ignoreLocation`, so that each error costs `1 / pattern.len()`: whether the
/// pattern matches the text with at most 32% errors.
fn bitap(text: &[u16], pattern: &[u16], location: usize) -> bool {
    let (d, p) = (pattern.len() as isize, text.len() as isize);
    let score = |errors: isize| errors as f64 / d as f64;
    let expected = (location as isize).min(p);
    let mut threshold = 0.32_f64;
    let mut from = expected as usize;
    while let Some(at) = text.get(from..).and_then(|rest| rest.windows(pattern.len()).position(|w| w == pattern)) {
        threshold = threshold.min(score(0));
        from += at + pattern.len();
    }
    let mut alphabet: HashMap<u16, i32> = HashMap::new();
    for (i, &c) in pattern.iter().enumerate() {
        *alphabet.entry(c).or_default() |= 1i32.wrapping_shl((d - i as isize - 1) as u32);
    }
    let (mut best, mut last, mut bin_max) = (-1, Vec::<i32>::new(), d + p);
    let mask = 1i32.wrapping_shl(d as u32 - 1);
    let at = |bits: &[i32], i: isize| bits.get(i as usize).copied().unwrap_or(0);
    for errors in 0..d {
        let (mut bin_min, mut bin_mid) = (0, bin_max);
        while bin_min < bin_mid {
            if score(errors) <= threshold {
                bin_min = bin_mid;
            } else {
                bin_max = bin_mid;
            }
            bin_mid = (bin_max - bin_min) / 2 + bin_min;
        }
        bin_max = bin_mid;
        let mut start = 1.max(expected - bin_mid + 1);
        let finish = (expected + bin_mid).min(p) + d;
        let mut bits = vec![0i32; finish as usize + 2];
        bits[finish as usize + 1] = 1i32.wrapping_shl(errors as u32).wrapping_sub(1);
        let mut j = finish;
        while j >= start {
            let found = text.get(j as usize - 1).and_then(|c| alphabet.get(c)).copied().unwrap_or(0);
            bits[j as usize] = (at(&bits, j + 1).wrapping_shl(1) | 1) & found;
            if errors > 0 {
                bits[j as usize] |= (at(&last, j + 1) | at(&last, j)).wrapping_shl(1) | 1 | at(&last, j + 1);
            }
            if bits[j as usize] & mask != 0 && score(errors) <= threshold {
                (threshold, best) = (score(errors), j - 1);
                if best <= expected {
                    break;
                }
                start = 1.max(2 * expected - best);
            }
            j -= 1;
        }
        if score(errors + 1) > threshold {
            break;
        }
        last = bits;
    }
    best >= 0
}

/// Desktop's `Rd.search`, through `searchTitlesOrNetworkDetailed` (the title and the network) or, with `people`,
/// `searchParticipantsDetailed`: the chats that match the query, each with the number of its title matches
/// (`score.title.hits`).
pub(super) fn search<'a>(chats: impl Iterator<Item = &'a Chat>, query: &str, people: bool) -> Vec<(&'a Chat, usize)> {
    let fields: &[Field] = if people { &[Field::Participants] } else { &[Field::Title, Field::Network] };
    let chats: Vec<(&Chat, Fields)> = chats.map(|c| (c, Fields::of(c))).collect();
    let whole = phrase(query);
    if !people && has_symbol(&whole) {
        let found: Vec<_> = chats
            .iter()
            .filter(|(_, f)| fields.iter().any(|&k| contains(f.values(k), &whole)))
            .map(|(c, f)| (*c, contains(f.values(Field::Title), &whole) as usize))
            .collect();
        if !found.is_empty() {
            return found;
        }
    }
    let all = words(query);
    let required: Vec<&String> = all.iter().filter(|w| *w != "and").collect();
    let terms: Vec<&String> = if required.is_empty() { all.iter().collect() } else { required.clone() };
    let mut found = Vec::new();
    for (c, f) in &chats {
        let (mut matched, mut hits) = (HashSet::new(), 0);
        for &term in &terms {
            for &k in fields {
                let values = f.values(k);
                let fuzzy = || term.encode_utf16().count() >= 4;
                let fuse = values.iter().any(|v| !js_trim(v).is_empty() && fuse_match(term, v));
                if fuse && (contains(values, term) || fuzzy()) {
                    matched.insert(term);
                    hits += (k == Field::Title) as usize;
                }
            }
        }
        if !matched.is_empty() && required.iter().all(|w| matched.contains(w)) {
            found.push((*c, hits));
        }
    }
    found
}

/// The expected values come from Desktop's own `Id`, `xd`, Fuse.js and `Rd` on the same inputs.
#[cfg(test)]
mod tests {
    use super::*;
    use beeper_core::models::User;

    #[test]
    fn words_and_phrases_are_latin_and_lowercase() {
        let cases: [(&str, &[&str], &str); 5] = [
            ("  Ÿou, & ME!  and (Привет)", &["you", "me", "and", "privet"], "you, & me! and (privet)"),
            (" Café \tdu  Monde ", &["cafe", "du", "monde"], "cafe du monde"),
            ("a.b@c", &["a.b@c"], "a.b@c"),
            ("and", &["and"], "and"),
            ("x|y=z^w", &["x", "y", "z", "w"], "x|y=z^w"),
        ];
        for (query, want, whole) in cases {
            assert_eq!(
                (words(query), phrase(query)),
                (want.iter().map(|w| w.to_string()).collect(), whole.into()),
                "{query}"
            );
        }
        assert!(has_symbol("a.b") && has_symbol("x@y") && !has_symbol("über_alles-2 ok"));
    }

    #[test]
    fn fuse_matches_like_fuse_js() {
        let long = "the quick brown fox jumps over the lazy dog again";
        let alphabet = "abcdefghijklmnopqrstuvwxyz0123456789";
        let cases = [
            ("eric", "eric migicovsky", true),
            ("erik", "eric migicovsky", true),
            ("migicovski", "eric migicovsky", true),
            ("cafe", "café", true),
            ("rust", "signal security ops", false),
            ("zzzz", "eric", false),
            ("ab", "ab", true),
            ("ab", "cd", false),
            ("abcd", "a", false),
            (long, &format!("{long}!"), true),
            (long, "the quick brown fox", false),
            ("rust", "this week in rust", true),
            ("rsut", "this week in rust", false),
            ("ruts", "trust", true),
            ("desing", "#design-review", true),
            ("famliy", "family", false),
            ("xyz", "family", false),
            ("slak", "slack", true),
            ("eric", "erc", true),
            ("ericm", "eric", true),
            (alphabet, &format!("zz {alphabet} zz"), true),
            (alphabet, &alphabet[..35], true),
            ("telegram", "telegrma", true),
            ("sec", "signal security ops", true),
            ("opz", "signal security ops", false),
        ];
        for (pattern, text, want) in cases {
            assert_eq!(fuse_match(pattern, text), want, "{pattern} in {text}");
        }
    }

    #[test]
    fn the_index_matches_and_counts_title_hits_like_desktop() {
        let st = beeper_core::store::BeeperStore::from_config(beeper_core::testing::demo_config("chat-index"));
        let person = |name: &str| User { full_name: Some(name.into()), ..User::default() };
        let chat = |id: &str, title: &str, network: &str, people: Vec<User>| {
            let mut c = st.chats[0].clone();
            (c.id, c.title, c.network_name) = (id.into(), title.into(), Some(network.into()));
            c.participants.items = people;
            c
        };
        let chats = [
            chat(
                "eric",
                "Eric Migicovsky",
                "WhatsApp",
                vec![User { phone_number: Some("+1 555 0100".into()), ..person("Eric Migicovsky") }, person("Alex")],
            ),
            chat(
                "rust",
                "Beeper Rust Core Team",
                "Telegram",
                vec![User { username: Some("tulir".into()), ..person("Tulir Asokan") }, person("Batuhan")],
            ),
            chat("weekly", "This Week in Rust", "Telegram", vec![person("Nell")]),
            chat(
                "release",
                "#desktop-release",
                "Slack",
                vec![
                    person("Devon Brooks"),
                    User { email: Some("eric@example.com".into()), ..person("Eric Migicovsky") },
                ],
            ),
            chat("design", "#design-review", "Slack", vec![person("Priya")]),
            chat("security", "Signal Security Ops", "Signal", vec![person("SecOps Lead")]),
            chat("family", "Family", "WhatsApp", vec![person("Mom")]),
            chat("rustrust", "Rust and Rust", "Signal", Vec::new()),
            chat("cafe", "Café Ünïcode", "Instagram", vec![person("Zoë")]),
        ];
        // A query, whether it searches the participants, and the chats with their title hits.
        type Case = (&'static str, bool, &'static [(&'static str, usize)]);
        let cases: [Case; 22] = [
            ("eric", false, &[("eric", 1)]),
            ("eric", true, &[("eric", 0), ("release", 0)]),
            ("rust", false, &[("rust", 1), ("rustrust", 1), ("weekly", 1)]),
            ("rust rust", false, &[("rust", 2), ("rustrust", 2), ("weekly", 2)]),
            ("rust and", false, &[("rust", 1), ("rustrust", 1), ("weekly", 1)]),
            ("and", false, &[("rustrust", 1)]),
            ("desing", false, &[("design", 1)]),
            ("#desktop", false, &[("release", 1)]),
            ("#nope", false, &[]),
            ("slack", false, &[("design", 0), ("release", 0)]),
            ("telegram rust", false, &[("rust", 1), ("weekly", 1)]),
            ("cafe", false, &[("cafe", 1)]),
            ("zoe", true, &[("cafe", 0)]),
            ("5550100", true, &[("eric", 0)]),
            ("eric@example", true, &[("release", 0)]),
            ("eric@example", false, &[]),
            ("rsut", false, &[]),
            ("ops", false, &[("security", 1)]),
            ("signal sec", false, &[("security", 2)]),
            ("tulir", true, &[("rust", 0)]),
            ("unicode", false, &[("cafe", 1)]),
            ("d", false, &[("cafe", 1), ("design", 1), ("release", 1), ("rustrust", 1)]),
        ];
        for (query, people, want) in cases {
            let mut found: Vec<(&str, usize)> =
                search(chats.iter(), query, people).into_iter().map(|(c, hits)| (c.id.as_str(), hits)).collect();
            found.sort();
            assert_eq!(found, want, "{query} {people}");
        }
    }
}
