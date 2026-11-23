//! Desktop's chat search index `Rd`, behind `searchTitlesOrNetworkDetailed` and `searchParticipantsDetailed`: the
//! words of a query match the search fields of a chat as substrings, and words of four or more characters also as the
//! fuzzy matches of Fuse.js 7.1.0.

use crate::config::account_network;
use crate::models::{member_order, Chat, User};
use crate::store::BeeperStore;
use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasher, Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};
use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};
use unicode_properties::{GeneralCategoryGroup, UnicodeGeneralCategory};

/// Desktop's `ft`: the Latin letters of the accented and the Cyrillic letters of its table `yd`.
fn latin(s: &str) -> String {
    static TABLE: OnceLock<HashMap<char, &str>> = OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let entry = |l: &'static str| l.split_once('\t').map(|(k, v)| (k.chars().next().expect("a character"), v));
        include_str!("latin.tsv").lines().map(|l| entry(l).expect("a tab-separated entry")).collect()
    });
    // The table has no ASCII characters.
    s.chars().fold(String::with_capacity(s.len()), |mut out, c| {
        match (!c.is_ascii()).then(|| table.get(&c)).flatten() {
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
pub fn js_trim(s: &str) -> &str {
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

/// A value of the search fields, in Latin letters and lowercase, with its forms for `contains` and Fuse.js.
struct Text {
    /// The value for `contains`, which takes the Latin letters of the lowercase value again.
    plain: String,
    /// The value for Fuse.js, which skips a blank value.
    fuse: Option<Vec<u16>>,
}

impl Text {
    fn of(s: &str) -> Self {
        let value = latin(s).to_lowercase();
        Text { plain: latin(&value).to_lowercase(), fuse: (!js_trim(&value).is_empty()).then(|| fuse_text(&value)) }
    }
}

/// The search fields of a chat, Desktop's `Sd`: its title, its network, and the names, usernames, emails and phone
/// digits of its first 50 present participants in Desktop's order, each in Latin letters and lowercase.
struct Fields {
    title: Text,
    network: Text,
    participants: Vec<Text>,
}

impl Fields {
    fn of(c: &Chat) -> Self {
        let mut members: Vec<_> = c.participants.items.iter().filter(|u| u.has_exited != Some(true)).collect();
        // The order only picks the first 50: a match of any participant counts the same.
        if members.len() > 50 {
            members.sort_by(|a, b| member_order(a, b));
        }
        let person = |u: &User| {
            let digits = u.phone_number.as_ref().map(|p| p.chars().filter(char::is_ascii_digit).collect::<String>());
            let names = [&u.full_name, &u.custom_nickname, &u.username, &u.email, &digits];
            let names: Vec<&str> = names.into_iter().flatten().filter(|n| !n.is_empty()).map(String::as_str).collect();
            (!names.is_empty()).then(|| Text::of(&names.join(" ")))
        };
        let network = c.network_name.as_deref().unwrap_or(account_network(&c.account_id).display_name);
        Fields {
            title: Text::of(&c.title),
            network: Text::of(network),
            participants: members.into_iter().take(50).filter_map(person).collect(),
        }
    }

    /// A hash of what the fields of `c` come from.
    fn source(c: &Chat) -> u64 {
        let mut h = foldhash::fast::FixedState::default().build_hasher();
        (&c.title, &c.network_name, &c.account_id).hash(&mut h);
        for u in &c.participants.items {
            (u.has_exited, u.is_self, u.is_network_bot, u.is_admin, u.is_pending, &u.id).hash(&mut h);
            (&u.full_name, &u.custom_nickname, &u.username, &u.email, &u.phone_number).hash(&mut h);
        }
        h.finish()
    }

    /// The fields of each chat, from the fields of the last search for the chats that did not change since.
    fn all<'a>(chats: impl Iterator<Item = &'a Chat>) -> Vec<(&'a Chat, Arc<Fields>)> {
        type Cache = HashMap<String, (u64, Arc<Fields>), foldhash::fast::RandomState>;
        static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
        let mut cache = CACHE.get_or_init(Default::default).lock().expect("the chat search cache");
        let all: Vec<_> = chats
            .map(|c| {
                let source = Fields::source(c);
                let fields = match cache.get(&c.id) {
                    Some((s, f)) if *s == source => f.clone(),
                    _ => {
                        let f = Arc::new(Fields::of(c));
                        cache.insert(c.id.clone(), (source, f.clone()));
                        f
                    }
                };
                (c, fields)
            })
            .collect();
        // The chats that left, or are past a filter of this search.
        if cache.len() > 2 * all.len() {
            let ids: HashSet<&str> = all.iter().map(|(c, _)| c.id.as_str()).collect();
            cache.retain(|id, _| ids.contains(id.as_str()));
        }
        all
    }

    fn values(&self, field: Field) -> &[Text] {
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
fn contains(values: &[Text], word: &str) -> bool {
    let word = js_trim(word);
    !word.is_empty() && values.iter().any(|v| v.plain.contains(word))
}

/// The UTF-16 code units of a text for Fuse.js, lowercase and without diacritics (`ignoreDiacritics`).
fn fuse_text(s: &str) -> Vec<u16> {
    s.to_lowercase().nfd().filter(|&c| !is_combining_mark(c)).collect::<String>().encode_utf16().collect()
}

/// Fuse.js's `MAX_BITS`, the longest pattern of a bitap search.
const MAX_BITS: usize = 32;

/// Fuse.js's `BitapSearch.searchIn` with Desktop's options (`ignoreLocation`, a threshold of 0.32): whether `pattern`
/// matches `text`, in chunks of 32 code units, both in the form of `fuse_text`.
fn fuse_match(pattern: &[u16], text: &[u16]) -> bool {
    if pattern == text {
        return true;
    }
    let len = pattern.len();
    let starts: Vec<usize> = match len {
        0 => Vec::new(),
        1..=MAX_BITS => vec![0],
        _ => (0..len - len % MAX_BITS)
            .step_by(MAX_BITS)
            .chain((!len.is_multiple_of(MAX_BITS)).then_some(len - MAX_BITS))
            .collect(),
    };
    starts.into_iter().any(|at| bitap(text, &pattern[at..len.min(at + MAX_BITS)], at))
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
    // The bits of each code unit of the pattern, which has at most 32.
    let mut alphabet: Vec<(u16, i32)> = Vec::with_capacity(pattern.len());
    for (i, &c) in pattern.iter().enumerate() {
        let bit = 1i32.wrapping_shl((d - i as isize - 1) as u32);
        match alphabet.iter_mut().find(|(k, _)| *k == c) {
            Some((_, bits)) => *bits |= bit,
            None => alphabet.push((c, bit)),
        }
    }
    let bits_of = |c: u16| alphabet.iter().find(|(k, _)| *k == c).map_or(0, |&(_, bits)| bits);
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
            let found = text.get(j as usize - 1).map_or(0, |&c| bits_of(c));
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
/// (`score.title.hits`). Without `fuzzy`, like the index of the command bar (`fuzzy: false`), a word matches only as
/// a substring.
pub fn search<'a>(
    chats: impl Iterator<Item = &'a Chat>,
    query: &str,
    people: bool,
    fuzzy: bool,
) -> Vec<(&'a Chat, usize)> {
    let chats = Fields::all(chats);
    matches(chats.iter().map(|(c, f)| (*c, &**f)).collect(), query, people, fuzzy)
}

/// [`search`] of the chats of the store `st` that `keep` takes. The fields of its chats hold while its chats stay the
/// same (`chat_changes`), so a key of the command bar only matches them.
pub fn search_in<'a>(
    st: &'a BeeperStore,
    keep: impl Fn(&Chat) -> bool,
    query: &str,
    people: bool,
    fuzzy: bool,
) -> Vec<(&'a Chat, usize)> {
    type ByChat = (Option<(u64, u64)>, Vec<Arc<Fields>>);
    static FIELDS: OnceLock<Mutex<ByChat>> = OnceLock::new();
    let mut fields = FIELDS.get_or_init(Default::default).lock().expect("the chat search fields");
    let revision = Some((st.instance, st.chat_changes));
    if fields.0 != revision || fields.1.len() != st.chats.len() {
        *fields = (revision, Fields::all(st.chats.iter()).into_iter().map(|(_, f)| f).collect());
    }
    let chats = st.chats.iter().zip(&fields.1).filter(|(c, _)| keep(c)).map(|(c, f)| (c, &**f)).collect();
    matches(chats, query, people, fuzzy)
}

fn matches<'a>(chats: Vec<(&'a Chat, &Fields)>, query: &str, people: bool, fuzzy: bool) -> Vec<(&'a Chat, usize)> {
    let fields: &[Field] = if people { &[Field::Participants] } else { &[Field::Title, Field::Network] };
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
    let patterns: Vec<(&String, Vec<u16>)> = terms.iter().map(|&t| (t, fuse_text(t))).collect();
    for (c, f) in &chats {
        let (mut matched, mut hits) = (HashSet::new(), 0);
        for &(term, ref pattern) in &patterns {
            for &k in fields {
                let values = f.values(k);
                let long = || term.encode_utf16().count() >= 4;
                let fuse = || values.iter().any(|v| v.fuse.as_ref().is_some_and(|text| fuse_match(pattern, text)));
                let hit = match fuzzy {
                    true => fuse() && (contains(values, term) || long()),
                    false => contains(values, term),
                };
                if hit {
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
            assert_eq!(fuse_match(&fuse_text(pattern), &fuse_text(text)), want, "{pattern} in {text}");
        }
    }

    #[test]
    fn the_index_matches_and_counts_title_hits_like_desktop() {
        let st = crate::store::BeeperStore::from_config(crate::testing::demo_config("chat-index"));
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
                search(chats.iter(), query, people, true).into_iter().map(|(c, hits)| (c.id.as_str(), hits)).collect();
            found.sort();
            assert_eq!(found, want, "{query} {people}");
        }
    }

    /// A search takes the fields of a chat again when what they come from changes, and only the first 50 members
    /// in Desktop's order.
    #[test]
    fn the_index_follows_changed_chats() {
        let st = crate::store::BeeperStore::from_config(crate::testing::demo_config("chat-index-cache"));
        let person = |name: String| User { full_name: Some(name), ..User::default() };
        let mut c = st.chats[0].clone();
        (c.id, c.title) = ("!cache:x".into(), "Old Title".into());
        c.participants.items = (0..60).map(|i| person(format!("Member {i:02}"))).collect();
        let found = |c: &Chat, q: &str, people: bool| search(std::iter::once(c), q, people, true).len();
        assert_eq!((found(&c, "old", false), found(&c, "member 49", true), found(&c, "member 50", true)), (1, 1, 0));
        c.title = "New Title".into();
        c.participants.items[0].full_name = Some("Ann".into());
        let found =
            [found(&c, "old", false), found(&c, "new", false), found(&c, "member 00", true), found(&c, "ann", true)];
        assert_eq!(found, [0, 1, 0, 1]);
    }

    /// The command bar's index (`fuzzy: false`): a word matches only as a substring, in any order, every word but
    /// "and" matches, and a member that left does not count.
    #[test]
    fn the_command_bar_index_takes_substrings_only() {
        let st = crate::store::BeeperStore::from_config(crate::testing::demo_config("chat-index-exact"));
        let user = |name: &str, phone: Option<&str>, left: bool| User {
            full_name: Some(name.into()),
            phone_number: phone.map(Into::into),
            has_exited: left.then_some(true),
            ..Default::default()
        };
        let mut c = st.chats[0].clone();
        (c.id, c.title, c.network_name) = ("!team:x".into(), "Beeper Rust Café".into(), Some("Telegram".into()));
        c.participants.items = vec![
            user("Alice Smith", Some("+1 (415) 555-0100"), false),
            user("Bob Jones", None, false),
            user("Carol Left", None, true),
        ];
        let found = |q: &str, people: bool| search(std::iter::once(&c), q, people, false).first().map(|f| f.1);
        assert_eq!(found("rust beeper", false), Some(2), "the words in any order");
        assert_eq!(found("cafe", false), Some(1), "in Latin letters");
        assert_eq!(found("telegram", false), Some(0), "by the network only");
        assert_eq!(found("rsut", false), None, "no fuzzy match");
        assert_eq!(found("alice", true), Some(0));
        assert_eq!(found("ali jon", true), Some(0), "the words can match different members");
        assert_eq!(found("smith and bob", true), Some(0), "\"and\" is not a word to match");
        assert_eq!(found("4155550100", true), Some(0), "a phone number matches by its digits");
        assert_eq!(found("carol", true), None, "a member that left does not match");
        assert_eq!(found("alice zed", true), None, "every word matches");
        assert_eq!(found("and", true), None, "\"and\" alone needs a member with it");
        assert_eq!(found(" , ", true), None);
    }
}
