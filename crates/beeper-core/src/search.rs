//! Desktop's search operators (`vs` and `Zs`): `has:`, `from:`, `in:`, `before:`, `after:` and `network:` in the
//! text of a message search, which `Hct` reads, and Desktop's `searchByParticipants` chat search.

use crate::config::local_millis;
use crate::models::Chat;
use crate::store::{BeeperStore, Query};

/// The `has:` kinds of Desktop's media filter (`epe`) with the attachment type of each, in the filter's order.
pub const HAS: [(&str, &str); 5] =
    [("image", "img"), ("video", "video"), ("audio", "audio"), ("link", "link"), ("file", "unknown")];

/// The `before:` and `after:` dates of Desktop's `Q5`, in the order of its time periods (`Xhe`).
pub const DATES: [&str; 6] = ["hour", "today", "yesterday", "week", "month", "year"];

/// The media of a `has:` search.
#[derive(Debug, Clone, PartialEq)]
pub enum Media {
    /// `has:attachment`: any attachment.
    Any,
    /// The attachment types of `HAS`.
    Kinds(Vec<&'static str>),
}

/// The options of a message search, Desktop's `searchRoomMessages` options.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Search {
    pub media: Option<Media>,
    /// `from:me` (`true`) or `from:others`.
    pub sender: Option<bool>,
    /// `in:group` (`true`) or `in:dm`.
    pub group: Option<bool>,
    /// The chat IDs of `in:`.
    pub chats: Vec<String>,
    /// The account IDs of `network:`.
    pub accounts: Vec<String>,
    /// `after:` and `before:` in Unix milliseconds.
    pub after: Option<i64>,
    pub before: Option<i64>,
}

impl Search {
    /// Desktop's `Hct`: the options of the operators in `q` and the text without them. A word with an unknown
    /// operator or value stays in the text. The dates count from `now`, in Unix milliseconds.
    pub fn parse(q: &str, now: i64) -> (Self, String) {
        let mut s = Self::default();
        let text: Vec<&str> = q.split(' ').filter(|w| !s.take(w, now)).collect();
        (s, text.join(" "))
    }

    /// Takes the operator `word`, or returns false when it is not one.
    fn take(&mut self, word: &str, now: i64) -> bool {
        let Some((op, value)) = word.split_once(':') else { return false };
        match (op, value) {
            ("has", "attachment") => self.media = Some(Media::Any),
            ("has", kind) => {
                let Some(&(_, t)) = HAS.iter().find(|h| h.0 == kind) else { return false };
                match &mut self.media {
                    Some(Media::Any) => {}
                    Some(Media::Kinds(kinds)) if kinds.contains(&t) => {}
                    Some(Media::Kinds(kinds)) => kinds.push(t),
                    None => self.media = Some(Media::Kinds(vec![t])),
                }
            }
            ("from", "me" | "others") => self.sender = Some(value == "me"),
            ("in", "group" | "dm") => self.group = Some(value == "group"),
            ("in" | "network", "") => return false,
            ("in", chat) => self.chats.push(chat.into()),
            ("network", account) => self.accounts.push(account.into()),
            ("after" | "before", key) => {
                let Some(at) = date(key, now) else { return false };
                *if op == "after" { &mut self.after } else { &mut self.before } = Some(at);
            }
            _ => return false,
        }
        true
    }

    /// The conditions of the options for `BeeperStore::scan`, without the text.
    pub fn query(&self, st: &BeeperStore) -> Query {
        let mut q = Query::default();
        if let Some(me) = self.sender {
            q = q.sent(me);
        }
        q = match &self.media {
            None => q,
            Some(Media::Any) => q.has(None),
            Some(Media::Kinds(kinds)) => q.has(Some(kinds)),
        };
        if let Some(at) = self.after {
            q = q.time(at, true);
        }
        if let Some(at) = self.before {
            q = q.time(at, false);
        }
        if self.group.is_none() && self.chats.is_empty() && self.accounts.is_empty() {
            return q;
        }
        let ids: Vec<&str> = st
            .chats
            .iter()
            .filter(|c| {
                self.group.is_none_or(|g| (c.chat_type == "group") == g)
                    && (self.chats.is_empty() || self.chats.contains(&c.id))
                    && (self.accounts.is_empty() || self.accounts.contains(&c.account_id))
            })
            .map(|c| c.id.as_str())
            .collect();
        q.chats(&ids)
    }
}

/// Desktop's `Q5` date of a `before:` or `after:` key in Unix milliseconds: an hour, a week, 30 days or a year
/// before `now`, or the local midnight of today or yesterday.
pub fn date(key: &str, now: i64) -> Option<i64> {
    const DAY: i64 = 86_400_000;
    let today = now - local_millis(now).rem_euclid(DAY);
    Some(match key {
        "hour" => now - 3_600_000,
        "today" => today,
        "yesterday" => today - DAY,
        "week" => now - 7 * DAY,
        "month" => now - 30 * DAY,
        "year" => now - 365 * DAY,
        _ => return None,
    })
}

/// Desktop's `searchByParticipants` match: each word of `q` but "and" is in the name, nickname, username, email or
/// phone digits of one of the first 50 members that did not leave. A query of only "and" needs one of its words.
pub fn participants_match(c: &Chat, q: &str) -> bool {
    let words: Vec<String> = q
        .to_lowercase()
        .split(|ch: char| ch.is_whitespace() || "|!'^=,;?(){}[]\"`~#$%&*+<>\\".contains(ch))
        .filter(|w| !w.is_empty())
        .map(String::from)
        .collect();
    if words.is_empty() {
        return false;
    }
    let people: Vec<String> = c
        .participants
        .items
        .iter()
        .filter(|u| u.has_exited != Some(true))
        .take(50)
        .map(|u| {
            let phone = u.phone_number.as_ref().map(|p| p.chars().filter(char::is_ascii_digit).collect::<String>());
            [&u.full_name, &u.custom_nickname, &u.username, &u.email, &phone]
                .into_iter()
                .flatten()
                .filter(|s| !s.is_empty())
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase()
        })
        .collect();
    let hit = |w: &String| people.iter().any(|p| p.contains(w.as_str()));
    let required: Vec<&String> = words.iter().filter(|w| *w != "and").collect();
    match required.is_empty() {
        true => words.iter().any(hit),
        false => required.into_iter().all(hit),
    }
}

#[cfg(test)]
mod tests {
    use super::{Media, Search};

    const NOW: i64 = 1_700_000_000_000;

    /// Desktop's `Hct`: each known operator becomes an option, and the other words stay in the text.
    #[test]
    fn operators_parse_like_desktop() {
        let (s, text) = Search::parse("has:image cats from:me in:group has:link network:telegram after:week", NOW);
        assert_eq!(text, "cats");
        assert_eq!(s.media, Some(Media::Kinds(vec!["img", "link"])));
        assert_eq!((s.sender, s.group, s.accounts.as_slice()), (Some(true), Some(true), &["telegram".to_string()][..]));
        assert_eq!((s.after, s.before), (Some(NOW - 7 * 86_400_000), None));
        let (s, text) = Search::parse("has:attachment has:video from:others in:dm in:!a:b.c", NOW);
        assert_eq!(text, "");
        assert_eq!((s.media, s.sender, s.group), (Some(Media::Any), Some(false), Some(false)));
        assert_eq!(s.chats, ["!a:b.c"], "a chat ID keeps its colons");
        let (s, text) = Search::parse("has:gif from:you in: before:2024 network: HAS:image note:", NOW);
        assert_eq!(s, Search::default(), "unknown values are text");
        assert_eq!(text, "has:gif from:you in: before:2024 network: HAS:image note:");
        let (s, _) = Search::parse("before:today after:yesterday", NOW);
        assert_eq!(s.before.unwrap() - s.after.unwrap(), 86_400_000, "yesterday is a day");
        assert_eq!(Search::parse("a  b", NOW).1, "a  b", "the text keeps its spaces");
    }

    /// Each option keeps the messages that Desktop's `searchRoomMessages` keeps for it.
    #[test]
    fn options_filter_messages_like_desktop() {
        use serde_json::json;
        let mut st = crate::store::BeeperStore::from_config(crate::testing::demo_config("search-options"));
        let (group, dm) = ("!tg_rust:beeper.com", "!wa_eric:beeper.com");
        let rows = [
            ("z1", group, true, json!({})),
            ("z2", group, false, json!({ "attachments": [{ "type": "img" }] })),
            ("z3", dm, false, json!({ "attachments": [{ "type": "unknown", "fileName": "a.pdf" }] })),
            ("z4", dm, true, json!({ "links": [{ "url": "https://a.b", "title": "A" }] })),
            ("z5", group, false, json!({ "attachments": [{ "type": "video" }] })),
        ];
        for (i, (id, chat, mine, mut m)) in rows.into_iter().enumerate() {
            let account = if chat == group { "acc_telegram" } else { "acc_whatsapp" };
            m.as_object_mut().expect("an object").extend([
                ("id".into(), json!(id)),
                ("chatID".into(), json!(chat)),
                ("accountID".into(), json!(account)),
                ("senderID".into(), json!(if mine { "@alex:beeper.com" } else { "@them:beeper.com" })),
                ("isSender".into(), json!(mine)),
                ("sortKey".into(), json!(format!("00000000000099{i:02}"))),
                ("timestamp".into(), json!(format!("2026-10-0{}T10:00:00.000Z", i + 1))),
                ("text".into(), json!(format!("zebra {id}"))),
                ("type".into(), json!("TEXT")),
            ]);
            st.upsert_message(serde_json::from_value(m).expect("a message"));
        }
        let now = crate::config::millis_from_rfc3339("2026-10-05T10:30:00.000Z").expect("a time");
        let found = |q: &str, partial: bool| -> Vec<String> {
            let (s, text) = Search::parse(q, now);
            let mut ids = Vec::new();
            st.scan(&s.query(&st).phrase(&text, partial), |m| {
                ids.push(m.id);
                true
            });
            ids.sort();
            ids
        };
        let all = |q: &str| found(q, true);
        assert_eq!(all("zebra"), ["z1", "z2", "z3", "z4", "z5"]);
        assert_eq!(found("zeb", true).len(), 5, "a partial match takes the start of a word");
        assert!(found("zeb", false).is_empty(), "without partial matches a word matches whole");
        assert_eq!(all("zebra from:me"), ["z1", "z4"]);
        assert_eq!(all("zebra from:others"), ["z2", "z3", "z5"]);
        assert_eq!(all("zebra in:group"), ["z1", "z2", "z5"]);
        assert_eq!(all("zebra in:dm"), ["z3", "z4"]);
        assert_eq!(all(&format!("zebra in:{dm}")), ["z3", "z4"]);
        assert_eq!(all(&format!("zebra in:{dm} in:{group} from:me")), ["z1", "z4"]);
        assert_eq!(all("zebra network:acc_whatsapp"), ["z3", "z4"]);
        assert!(all("zebra network:acc_whatsapp in:group").is_empty());
        assert_eq!(all("zebra has:attachment"), ["z2", "z3", "z5"], "a link preview is not an attachment");
        assert_eq!(all("zebra has:image"), ["z2"]);
        assert_eq!(all("zebra has:link"), ["z4"]);
        assert_eq!(all("zebra has:file has:video"), ["z3", "z5"]);
        assert!(all("zebra has:audio").is_empty());
        assert_eq!(all("zebra after:hour"), ["z5"]);
        assert_eq!(all("zebra before:hour after:week"), ["z1", "z2", "z3", "z4"]);
        assert!(st.toggle_bookmark("z3"));
        let mut marked = Vec::new();
        st.scan(&Search::default().query(&st).text("zebra").bookmarked(), |m| {
            marked.push(m.id);
            true
        });
        assert_eq!(marked, ["z3"]);
    }

    /// Desktop's `searchByParticipants`: every word is in some member, a member that left does not count.
    #[test]
    fn participants_match_like_desktop() {
        use crate::models::User;
        let mut c: crate::models::Chat =
            serde_json::from_value(serde_json::json!({ "id": "c", "accountID": "a", "title": "Team", "type": "group",
                "unreadCount": 0, "participants": { "items": [], "hasMore": false, "total": 0 } }))
            .expect("a chat");
        let user = |name: &str, phone: Option<&str>, left: bool| User {
            full_name: Some(name.into()),
            phone_number: phone.map(Into::into),
            has_exited: left.then_some(true),
            ..Default::default()
        };
        c.participants.items =
            vec![user("Alice Smith", Some("+1 (415) 555-0100"), false), user("Bob Jones", None, false)];
        c.participants.items.push(user("Carol Left", None, true));
        assert!(super::participants_match(&c, "alice"));
        assert!(super::participants_match(&c, "ali jon"), "the words can match different members");
        assert!(super::participants_match(&c, "smith and bob"), "\"and\" is not a word to match");
        assert!(super::participants_match(&c, "4155550100"), "a phone number matches by its digits");
        assert!(!super::participants_match(&c, "carol"), "a member that left does not match");
        assert!(!super::participants_match(&c, "alice zed"), "every word matches");
        assert!(!super::participants_match(&c, "and"), "\"and\" alone needs a member with it");
        assert!(!super::participants_match(&c, " , "));
    }
}
