//! Desktop's drafts sync (`DraftSyncStore`, flag `drafts-sync`): the draft of a chat goes to the other devices of the
//! user as the room account data `com.beeper.draft`, which Desktop's SDK encrypts with the MSC4483 key of secret
//! storage, so this device writes only with the recovery key stored.
//!
//! A draft record is Desktop's draft thread prop: a `com.beeper.revision_id`, the message `contents` that the draft
//! would send, its `send_room_id` and `send_later`, and the `device_id` and `ts` of the write. A record without
//! `contents` is a tombstone: the draft went. Each change of what syncs gets a new revision, which the next write
//! expects as the current one (`com.beeper.expect_revision_id`). A draft here that changed since it last matched the
//! synced one wins: the record waits (`isApplied` false) until the draft here goes.
use crate::matrix::{Mention, Outgoing, ROOM_MENTION};
use crate::models::{ChatDraft, SyncedDraft};
use crate::store::BeeperStore;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Desktop's `RoomAccountDataType.DRAFT`.
pub const DRAFT: &str = "com.beeper.draft";
/// Desktop's revision field of a draft record, which MSC4483 keeps in plaintext.
pub const REVISION: &str = "com.beeper.revision_id";
/// Desktop's `FeatureFlag.DRAFTS_SYNC`.
pub const FLAG: &str = "drafts-sync";
/// Desktop's `scheduleDraftPush` delay: a changed draft goes 10 s after its last save; a removed one goes at once.
pub const PUSH_DELAY: Duration = Duration::from_secs(10);
const HTML: &str = "org.matrix.custom.html";

/// The composer of a chat open in a window, as the drafts sync sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Composer {
    /// Desktop's `acceptsRemoteDraft`: no edit, voice note, file or unsaved change, so a synced draft can replace it.
    pub accepts: bool,
    /// Desktop's `composerUnsyncable`: an edit or a voice note, whose draft goes as a removal.
    pub unsyncable: bool,
}

/// What the windows share with the drafts sync without a write lock of the store.
#[derive(Debug, Default)]
pub struct Link {
    /// The composer of each open chat by chat ID: Desktop's selected threads.
    pub open: HashMap<String, Composer>,
    /// The open chats whose draft a synced one replaced, for their windows to show (`applyRemoteDraft`).
    pub applied: Vec<String>,
}

/// The state of the drafts sync in the store.
#[derive(Debug, Default)]
pub struct DraftSync {
    pub link: Arc<Mutex<Link>>,
    /// Desktop's `pushTimers`: when the push of each chat's changed draft is due.
    pub(crate) timers: HashMap<String, Instant>,
    /// The rooms whose synced draft could not be decrypted yet, which the next sync with the key reads again.
    pub(crate) undecrypted: HashSet<String>,
    /// The flag at the last sync, so that its change to on reconciles the waiting records (`flagReaction`).
    flag: bool,
}

/// Desktop's `pushDraftStateNow` decision (`$Kt`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Push {
    /// A synced record waits for the draft here to go, so nothing goes.
    HoldConflict,
    /// The draft cannot sync (it has files, or the composer edits or records), so the synced one goes.
    Clear,
    Nothing,
    InSync,
    Draft,
}

/// Desktop's `reconcileDraftRecord` decision (`UKt`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reconcile {
    Ignore,
    Hold,
    AdoptAndPush,
    Clear,
    Apply,
}

/// A random UUID v4, like Desktop's revision IDs.
pub fn new_revision() -> String {
    use ring::rand::SecureRandom;
    let mut b = [0u8; 16];
    ring::rand::SystemRandom::new().fill(&mut b).expect("random bytes");
    (b[6], b[8]) = ((b[6] & 0x0f) | 0x40, (b[8] & 0x3f) | 0x80);
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &hex[..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..])
}

/// The revision of a record.
pub fn revision(record: &Value) -> Option<&str> {
    record[REVISION].as_str()
}

/// Desktop's `n3`: a record without contents, a tombstone.
pub fn is_tombstone(record: &Value) -> bool {
    record["contents"].as_array().is_none_or(Vec::is_empty)
}

/// Desktop's `$ye`: the msgtype of a file by its MIME type.
fn file_type(mime: &str) -> &'static str {
    match mime.split('/').next() {
        Some("image") => "m.image",
        Some("video") => "m.video",
        Some("audio") => "m.audio",
        _ => "m.file",
    }
}

/// Desktop's `Jle`: the draft record of `d`, with its revision. The text goes like a sent message: the composer
/// Markdown as the body, its HTML, the mentions and the quoted message.
pub fn record(st: &BeeperStore, d: &ChatDraft) -> Value {
    let mut contents = Vec::new();
    if let Some(text) = d.text.as_deref().filter(|t| !t.trim().is_empty()) {
        let mut c = json!({ "msgtype": "m.text", "body": text });
        let users: Vec<&Mention> = d.mentions.iter().filter(|m| m.1 != ROOM_MENTION).collect();
        let room = d.mentions.iter().any(|m| m.1 == ROOM_MENTION);
        if !users.is_empty() || room {
            let mut ids: Vec<&str> = users.iter().map(|m| m.1.as_str()).collect();
            ids.dedup();
            c["m.mentions"] = json!({ "user_ids": ids });
            if room {
                c["m.mentions"]["room"] = true.into();
            }
        }
        if let Some(html) = crate::codec::markdown_html(&crate::matrix::mention_links(text, &users)) {
            (c["format"], c["formatted_body"]) = (HTML.into(), html.into());
        }
        if let Some(reply) = &d.reply_to_message_id {
            let id = st.sent.get(reply).unwrap_or(reply);
            c["m.relates_to"] = json!({ "m.in_reply_to": { "event_id": id } });
        }
        contents.push(c);
    }
    for a in &d.attachments {
        let mime = a.mime_type.as_deref().unwrap_or_default();
        let body = a.file_name.as_deref().unwrap_or_default();
        contents.push(json!({ "msgtype": file_type(mime), "body": body, "info": { "mimetype": mime } }));
    }
    let mut r = json!({ "contents": contents });
    if let Some(rev) = &d.revision_id {
        r[REVISION] = rev.clone().into();
    }
    if let Some(room) = &d.send_room_id {
        r["send_room_id"] = room.clone().into();
    }
    if let Some((ts, cancel)) = d.send_later {
        r["send_later"] = json!({ "ts": ts });
        if cancel {
            r["send_later"]["cancel_on_reply"] = true.into();
        }
    }
    r
}

/// Desktop's `HKt` with `withoutUnsyncableMetadata`: the record that goes for `d`, by `device` at `d.ts` (else
/// `now`), without its Send Later time unless the plan schedules without a limit.
pub fn wire(st: &BeeperStore, d: &ChatDraft, device: &str, now: i64, send_later: bool) -> Value {
    let mut r = record(st, d);
    let o = r.as_object_mut().expect("a record is an object");
    if !send_later && o.remove("send_later").is_some() {
        eprintln!("[drafts-sync] stripping send_later: not unlimited-schedule eligible");
    }
    o.insert("device_id".into(), device.into());
    o.insert("ts".into(), d.ts.unwrap_or(now).into());
    r
}

/// Desktop's `qKt`: the tombstone record of a removed draft, with a new revision.
pub fn tombstone(device: &str) -> Value {
    json!({ REVISION: new_revision(), "device_id": device })
}

/// Desktop's `YD` of a value: sorted keys and no null fields.
fn canonical(v: &Value) -> Value {
    match v {
        Value::Object(m) => {
            let sorted: BTreeMap<&String, Value> =
                m.iter().filter(|(_, v)| !v.is_null()).map(|(k, v)| (k, canonical(v))).collect();
            Value::Object(sorted.into_iter().map(|(k, v)| (k.clone(), v)).collect::<Map<_, _>>())
        }
        Value::Array(a) => Value::Array(a.iter().map(canonical).collect()),
        v => v.clone(),
    }
}

/// Desktop's `lG`: what a record syncs, without its revision, device and time, for comparisons.
pub fn content_key(record: &Value) -> String {
    let mut m = record.as_object().cloned().unwrap_or_default();
    for k in [REVISION, "device_id", "ts"] {
        m.remove(k);
    }
    canonical(&Value::Object(m)).to_string()
}

/// Desktop's `_ae`: the draft here is the synced one that it took, or both are gone.
pub fn in_sync(local: Option<&ChatDraft>, synced: Option<&SyncedDraft>) -> bool {
    match local {
        None => synced.is_none_or(|s| is_tombstone(&s.record)),
        Some(d) => d
            .revision_id
            .as_deref()
            .is_some_and(|r| synced.is_some_and(|s| s.applied && revision(&s.record) == Some(r))),
    }
}

/// Desktop's `QFe`: a draft of text alone, which syncs.
fn syncable(d: &ChatDraft) -> bool {
    d.text.as_deref().is_some_and(|t| !t.trim().is_empty()) && d.attachments.is_empty()
}

/// Desktop's `$Kt`: what goes for the draft here.
pub fn push_decision(local: Option<&ChatDraft>, synced: Option<&SyncedDraft>, unsyncable: bool) -> Push {
    if synced.is_some_and(|s| !s.applied) {
        return Push::HoldConflict;
    }
    if !local.is_some_and(syncable) || unsyncable {
        return match synced.is_some_and(|s| !is_tombstone(&s.record)) {
            true => Push::Clear,
            false => Push::Nothing,
        };
    }
    match in_sync(local, synced) {
        true => Push::InSync,
        false => Push::Draft,
    }
}

/// Desktop's `UKt`: what a synced `record` does to the draft here, which last took `synced`.
pub fn reconcile_decision(
    record: &Value,
    local: Option<&ChatDraft>,
    synced: Option<&SyncedDraft>,
    own_device: &str,
    can_apply: bool,
) -> Reconcile {
    let rev = revision(record);
    let known = rev.is_some() && synced.is_some_and(|s| s.applied && revision(&s.record) == rev);
    if known || record["device_id"].as_str().is_some_and(|d| d == own_device) {
        return Reconcile::Ignore;
    }
    let matched = in_sync(local, synced);
    match (is_tombstone(record), local.is_some()) {
        (true, false) => Reconcile::Clear,
        (true, true) if matched && can_apply => Reconcile::Clear,
        (true, true) if synced.is_some_and(|s| !s.applied) => Reconcile::AdoptAndPush,
        (false, _) if can_apply && (matched || local.is_none()) => Reconcile::Apply,
        _ => Reconcile::Hold,
    }
}

/// Desktop's `mM`: the draft of a synced record, at `now`. The HTML of the text becomes composer Markdown with its
/// mention pills, and a Send Later time that passed goes.
pub fn draft_of(record: &Value, now: i64) -> Option<ChatDraft> {
    let contents = record["contents"].as_array()?;
    let texts: Vec<&Value> = contents.iter().filter(|c| c["msgtype"] == "m.text").collect();
    let html = texts.iter().find(|c| c["format"] == HTML).and_then(|c| c["formatted_body"].as_str());
    let (text, mut mentions) = match html {
        Some(h) => html_markdown(h),
        None => (texts.iter().find_map(|c| c["body"].as_str())?.to_string(), Vec::new()),
    };
    if contents.iter().any(|c| c["m.mentions"]["room"] == true) && text.contains(ROOM_MENTION) {
        mentions.push(("room".into(), ROOM_MENTION.into()));
    }
    mentions.dedup();
    let relation = &contents.first()?["m.relates_to"];
    let reply = (relation["rel_type"] != "m.replace").then(|| relation["m.in_reply_to"]["event_id"].as_str()).flatten();
    let later = &record["send_later"];
    let send_later = later["ts"].as_i64().filter(|&ts| ts > now).map(|ts| (ts, later["cancel_on_reply"] == true));
    Some(ChatDraft {
        text: Some(text).filter(|t| !t.trim().is_empty()),
        reply_to_message_id: reply.map(String::from),
        send_later,
        mentions,
        ts: record["ts"].as_i64(),
        send_room_id: record["send_room_id"].as_str().map(String::from),
        revision_id: revision(record).map(String::from),
        ..ChatDraft::default()
    })
    .filter(|d| d.text.is_some())
}

/// Matrix HTML as composer Markdown, with the mention pills of its `matrix.to` links.
fn html_markdown(html: &str) -> (String, Vec<Mention>) {
    let (mut lines, mut mentions) = (Vec::new(), Vec::new());
    for b in crate::codec::html_blocks(html) {
        let mut text = String::new();
        if b.pre {
            text = format!("```\n{}\n```", b.runs.iter().map(|r| r.text.as_str()).collect::<String>());
        } else {
            for r in b.runs {
                let mention = r.link.as_deref().and_then(|l| l.strip_prefix("https://matrix.to/#/"));
                let mention = mention.map(|m| m.split('?').next().unwrap_or(m)).filter(|m| m.starts_with('@'));
                let piece = if let Some(id) = mention {
                    let id = crate::codec::url_decode(id).unwrap_or_else(|| id.to_string());
                    let label = r.text.trim().trim_start_matches('@').to_string();
                    mentions.push((label.clone(), id));
                    format!("@{label}")
                } else if let Some(url) = &r.emoticon {
                    let name = r.title.as_deref().unwrap_or(&r.text).trim_matches(':').to_string();
                    crate::codec::Emoticon { name, url: url.clone() }.markdown()
                } else {
                    let mut t = r.text.clone();
                    if r.code {
                        t = format!("`{t}`");
                    }
                    if let Some(url) = r.link.as_ref().filter(|u| **u != r.text) {
                        t = format!("[{t}]({url})");
                    }
                    for (on, mark) in [(r.strike, "~~"), (r.italic, "_"), (r.bold, "**")] {
                        if on && !t.trim().is_empty() {
                            t = format!("{mark}{t}{mark}");
                        }
                    }
                    t
                };
                text.push_str(&piece);
            }
            let marker = b.marker.map(|m| if m == "•" { "- ".to_string() } else { format!("{m} ") });
            let heading = (b.heading > 0).then(|| format!("{} ", "#".repeat(b.heading.into())));
            text = format!("{}{}{text}", heading.unwrap_or_default(), marker.unwrap_or_default());
        }
        if b.quote > 0 {
            let prefix = "> ".repeat(b.quote.into());
            text = text.lines().map(|l| format!("{prefix}{l}")).collect::<Vec<_>>().join("\n");
        }
        lines.push(text);
    }
    (lines.join("\n"), mentions)
}

impl BeeperStore {
    /// Desktop's `canPushDrafts`: the chat is on a bridged account (every Matrix room; only Desktop's local
    /// platform SDK accounts are not), the flag is on, and the recovery key is stored for the encryption.
    pub fn can_push_drafts(&self, idx: usize) -> bool {
        let c = &self.chats[idx];
        let bridged = !self.config.demo && c.id.starts_with('!') && !c.account_id.is_empty();
        bridged && self.feature(FLAG).on && !self.beeper_state.secrets.recovery_code.is_empty()
    }

    /// Desktop's `persistDraftRecordNow`: the draft of the chat `idx`, which keeps its revision while what syncs of
    /// it stays the same. A new revision goes to the other devices (`onLocalDraftPersisted`).
    pub fn put_draft(&mut self, idx: usize, draft: Option<ChatDraft>) {
        let old = self.chats[idx].draft.clone();
        let draft = draft.map(|mut d| {
            let kept = old.as_ref().filter(|o| content_key(&record(self, o)) == content_key(&record(self, &d)));
            d.revision_id = kept.and_then(|o| o.revision_id.clone()).or_else(|| Some(new_revision()));
            d
        });
        let changed = old.as_ref().map(|d| &d.revision_id) != draft.as_ref().map(|d| &d.revision_id);
        let has = draft.is_some();
        self.update_chat(idx, |c| c.draft = draft);
        if changed {
            self.draft_persisted(idx, has);
        }
    }

    /// Desktop's `onLocalDraftPersisted`: a changed draft goes after [`PUSH_DELAY`], a removed one at once.
    fn draft_persisted(&mut self, idx: usize, debounce: bool) {
        let id = self.chats[idx].id.clone();
        if !self.can_push_drafts(idx) {
            let flag = self.feature(FLAG).on;
            let recovery = !self.beeper_state.secrets.recovery_code.is_empty();
            eprintln!("[drafts-sync] not pushing for {id}: flag={flag} recovery={recovery}");
            return;
        }
        match debounce {
            true => self.schedule_draft_push(&id),
            false => self.push_draft_now(&id),
        }
    }

    /// Desktop's `scheduleDraftPush`.
    fn schedule_draft_push(&mut self, chat: &str) {
        self.drafts.timers.insert(chat.into(), Instant::now() + PUSH_DELAY);
        self.outbox_ready.notify_one();
    }

    /// Desktop's `pushDraftState`: the outbox pushes the chat's draft, once for any number of changes before.
    fn push_draft_now(&mut self, chat: &str) {
        self.drafts.timers.remove(chat);
        if !self.outbox.iter().any(|(_, c, o)| c == chat && matches!(o, Outgoing::Draft)) {
            self.queue(chat, Outgoing::Draft);
        }
    }

    /// Desktop's `flushDraftPush`: a push that waits goes now.
    pub fn flush_draft_push(&mut self, chat: &str) {
        if self.drafts.timers.contains_key(chat) {
            self.push_draft_now(chat);
        }
    }

    /// Queues the pushes whose time came, and gives the time of the next one.
    pub(crate) fn queue_due_drafts(&mut self) -> Option<Instant> {
        let now = Instant::now();
        let due: Vec<String> = self.drafts.timers.iter().filter(|(_, &t)| t <= now).map(|(c, _)| c.clone()).collect();
        for chat in due {
            self.push_draft_now(&chat);
        }
        self.drafts.timers.values().min().copied()
    }

    /// Desktop's `pushDraftStateNow` up to the write: the record for the chat, and the revision that its write
    /// expects, or `None` when nothing goes.
    pub(crate) fn draft_push(&self, chat: &str) -> Option<(Value, String)> {
        let idx = self.resolve_chat_idx(chat)?;
        if !self.can_push_drafts(idx) {
            return None;
        }
        let c = &self.chats[idx];
        let unsyncable = self.drafts.link.lock().unwrap().open.get(&c.id).is_some_and(|l| l.unsyncable);
        let synced = c.synced_draft.as_deref();
        let decision = push_decision(c.draft.as_ref(), synced, unsyncable);
        eprintln!("[drafts-sync] push decision {decision:?} for {}", c.id);
        let device = &self.beeper_state.device_id;
        let unlimited = self.feature("scheduled-message").on && !self.features.has_freebie("scheduled-message");
        let now = crate::config::now_unix_millis();
        let record = match (decision, &c.draft) {
            (Push::Draft, Some(d)) => wire(self, d, device, now, unlimited),
            (Push::Clear, _) => tombstone(device),
            _ => return None,
        };
        let expected = synced.and_then(|s| revision(&s.record)).unwrap_or_default().to_string();
        Some((record, expected))
    }

    /// The homeserver took `record` for the chat.
    pub(crate) fn draft_pushed(&mut self, chat: &str, record: Value) {
        let Some(idx) = self.resolve_chat_idx(chat) else { return };
        let what = if is_tombstone(&record) { "tombstone" } else { "draft" };
        eprintln!("[drafts-sync] pushed {what} for {chat}");
        self.write_synced(idx, record, true);
    }

    fn write_synced(&mut self, idx: usize, record: Value, applied: bool) {
        self.update_chat(idx, |c| c.synced_draft = Some(Box::new(SyncedDraft { record, applied })));
    }

    /// Desktop's `com.beeper.draft` handler of the room, then `handleDraftSyncReceived`: a record of another
    /// revision waits with the flag off, and with the flag on it reconciles with the draft here.
    pub fn draft_received(&mut self, chat: &str, record: Value) {
        let Some(idx) = self.resolve_chat_idx(chat) else {
            eprintln!("[drafts-sync] no account for room, dropping draft record");
            return;
        };
        let synced = self.chats[idx].synced_draft.as_deref().cloned();
        let rev = revision(&record);
        if rev.is_some() && rev == synced.as_ref().and_then(|s| revision(&s.record)) {
            eprintln!("[drafts-sync] already-known revision, skipping");
            return;
        }
        if !self.feature(FLAG).on {
            eprintln!("[drafts-sync] flag off, recording unapplied for {chat}");
            return self.write_synced(idx, record, false);
        }
        self.reconcile_draft(idx, record, synced);
    }

    /// Desktop's `reconcileDraftRecord`.
    fn reconcile_draft(&mut self, idx: usize, rec: Value, synced: Option<SyncedDraft>) {
        let id = self.chats[idx].id.clone();
        let live = self.drafts.link.lock().unwrap().open.get(&id).copied();
        let local = self.chats[idx].draft.clone();
        let can_apply = live.map_or_else(|| local.as_ref().is_none_or(|d| d.attachments.is_empty()), |l| l.accepts);
        let own = self.beeper_state.device_id.clone();
        let decision = reconcile_decision(&rec, local.as_ref(), synced.as_ref(), &own, can_apply);
        eprintln!("[drafts-sync] {decision:?} for {id} (live={})", live.is_some());
        let rev = revision(&rec).map(String::from);
        match decision {
            Reconcile::Ignore => {
                if rec["device_id"] == own.as_str()
                    && rev.as_deref() != synced.as_ref().and_then(|s| revision(&s.record))
                {
                    self.write_synced(idx, rec, true);
                } else if let Some(s) = synced.filter(|s| !s.applied && revision(&s.record) == rev.as_deref()) {
                    self.write_synced(idx, s.record, true);
                }
            }
            Reconcile::Hold => self.write_synced(idx, rec, false),
            Reconcile::AdoptAndPush => {
                self.write_synced(idx, rec, true);
                self.schedule_draft_push(&id);
            }
            Reconcile::Clear | Reconcile::Apply => {
                let apply = decision == Reconcile::Apply;
                let same = local.as_ref().is_some_and(|d| content_key(&record(self, d)) == content_key(&rec));
                if apply && same {
                    eprintln!("[drafts-sync] adopted same-content revision for {id}");
                    self.update_chat(idx, |c| c.draft.as_mut().expect("the same draft").revision_id = rev);
                    return self.write_synced(idx, rec, true);
                }
                let draft = apply.then(|| draft_of(&rec, crate::config::now_unix_millis())).flatten();
                self.update_chat(idx, |c| c.draft = draft);
                if live.is_some() {
                    self.drafts.link.lock().unwrap().applied.push(id);
                }
                self.write_synced(idx, rec, true);
            }
        }
    }

    /// Desktop's `reconcileUnappliedDraft`: a record that waits reconciles again, unless its chat is open.
    pub fn reconcile_unapplied(&mut self, chat: &str) {
        let Some(idx) = self.resolve_chat_idx(chat) else { return };
        let Some(s) = self.chats[idx].synced_draft.as_deref().filter(|s| !s.applied).cloned() else { return };
        if !self.feature(FLAG).on || self.drafts.link.lock().unwrap().open.contains_key(&self.chats[idx].id) {
            return;
        }
        eprintln!("[drafts-sync] retrying unapplied record for {chat}");
        self.reconcile_draft(idx, s.record.clone(), Some(s));
    }

    /// Desktop's `onThreadDeselected`: the chat closed, so its waiting push goes, and a waiting record reconciles.
    pub fn leave_draft(&mut self, chat: &str) {
        let Some(idx) = self.resolve_chat_idx(chat) else { return };
        let id = self.chats[idx].id.clone();
        self.drafts.link.lock().unwrap().open.remove(&id);
        self.flush_draft_push(&id);
        self.reconcile_unapplied(&id);
    }

    /// Desktop's `replayUnappliedDrafts` when the flag turns on, and at the first sync with it on.
    pub(crate) fn replay_drafts(&mut self) {
        let on = self.feature(FLAG).on;
        if on && !std::mem::replace(&mut self.drafts.flag, on) {
            let waiting: Vec<String> =
                self.chats.iter().filter(|c| c.synced_draft.is_some()).map(|c| c.id.clone()).collect();
            if !waiting.is_empty() {
                eprintln!("[drafts-sync] flag on, reconciling synced drafts across {} threads", waiting.len());
            }
            for chat in waiting {
                self.reconcile_unapplied(&chat);
            }
        }
        self.drafts.flag = on;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ChatDraftAttachment;

    fn synced(record: Value, applied: bool) -> SyncedDraft {
        SyncedDraft { record, applied }
    }

    fn text(t: &str, rev: &str) -> ChatDraft {
        ChatDraft { text: Some(t.into()), revision_id: Some(rev.into()), ..ChatDraft::default() }
    }

    fn rec(rev: &str, device: &str, body: &str) -> Value {
        json!({ REVISION: rev, "device_id": device, "ts": 1, "contents": [{ "msgtype": "m.text", "body": body }] })
    }

    fn draft_text(st: &BeeperStore, idx: usize) -> Option<&str> {
        st.chats[idx].draft.as_ref().and_then(|d| d.text.as_deref())
    }

    /// Desktop's `$Kt`: a record that waits holds every push; a draft that cannot sync removes the synced one; a
    /// draft that is the synced one it took stays; another one goes.
    #[test]
    fn the_push_decision_is_desktops() {
        let a = rec("r1", "OTHER", "a");
        let local = text("a", "r1");
        assert_eq!(push_decision(Some(&local), Some(&synced(a.clone(), false)), false), Push::HoldConflict);
        assert_eq!(push_decision(Some(&local), Some(&synced(a.clone(), true)), false), Push::InSync);
        assert_eq!(push_decision(Some(&text("b", "r2")), Some(&synced(a.clone(), true)), false), Push::Draft);
        assert_eq!(push_decision(Some(&local), Some(&synced(a.clone(), true)), true), Push::Clear, "an edit");
        let file = ChatDraftAttachment { upload_id: "u".into(), file_name: None, mime_type: None };
        let with_file = ChatDraft { attachments: vec![file], ..local.clone() };
        assert_eq!(push_decision(Some(&with_file), Some(&synced(a.clone(), true)), false), Push::Clear, "a file");
        assert_eq!(push_decision(None, Some(&synced(a, true)), false), Push::Clear);
        assert_eq!(push_decision(None, Some(&synced(tombstone("D"), true)), false), Push::Nothing);
        assert_eq!(push_decision(None, None, false), Push::Nothing);
        assert_eq!(push_decision(Some(&local), None, false), Push::Draft);
    }

    /// Desktop's `UKt`: the own device and a known revision change nothing; a draft that matches the last synced
    /// one takes the new one, and a changed draft holds it; a removal clears a matching draft, and a changed draft
    /// goes over a removal that waited.
    #[test]
    fn the_reconcile_decision_is_desktops() {
        let r1 = synced(rec("r1", "OTHER", "a"), true);
        let r2 = rec("r2", "OTHER", "b");
        let gone = json!({ REVISION: "r3", "device_id": "OTHER" });
        let (same, changed) = (text("a", "r1"), text("mine", "r9"));
        let d = |record: &Value, local: Option<&ChatDraft>, s: Option<&SyncedDraft>, can: bool| {
            reconcile_decision(record, local, s, "DEV", can)
        };
        assert_eq!(d(&rec("r1", "OTHER", "a"), Some(&same), Some(&r1), true), Reconcile::Ignore);
        assert_eq!(d(&rec("r5", "DEV", "x"), None, None, true), Reconcile::Ignore, "the own device");
        assert_eq!(d(&r2, Some(&same), Some(&r1), true), Reconcile::Apply);
        assert_eq!(d(&r2, Some(&same), Some(&r1), false), Reconcile::Hold, "the composer has a file");
        assert_eq!(d(&r2, Some(&changed), Some(&r1), true), Reconcile::Hold);
        assert_eq!(d(&r2, None, Some(&r1), true), Reconcile::Apply);
        assert_eq!(d(&r2, None, None, false), Reconcile::Hold);
        assert_eq!(d(&gone, Some(&same), Some(&r1), true), Reconcile::Clear);
        assert_eq!(d(&gone, None, Some(&r1), false), Reconcile::Clear);
        assert_eq!(d(&gone, Some(&changed), Some(&r1), true), Reconcile::Hold);
        let waiting = synced(gone.clone(), false);
        assert_eq!(d(&gone, Some(&changed), Some(&waiting), true), Reconcile::AdoptAndPush);
    }

    /// The record of a draft is the message it would send, and a record of Desktop's composer comes back as
    /// composer Markdown with its mention pills, the quoted message, Send Later and the member chat to send to.
    #[test]
    fn records_are_desktops_draft_messages() {
        let st = BeeperStore::from_config(crate::testing::demo_config("draft-records"));
        let d = ChatDraft {
            text: Some("**hi** @Vic".into()),
            mentions: vec![("Vic".into(), "@vic:x".into())],
            reply_to_message_id: Some("$q".into()),
            send_later: Some((4_000_000_000_000, true)),
            send_room_id: Some("!member:x".into()),
            ts: Some(7),
            revision_id: Some("r1".into()),
            ..ChatDraft::default()
        };
        let r = wire(&st, &d, "DEV", 9, true);
        let c = &r["contents"][0];
        assert_eq!((c["msgtype"].as_str(), c["body"].as_str()), (Some("m.text"), Some("**hi** @Vic")));
        assert_eq!(c["formatted_body"], r#"<strong>hi</strong> <a href="https://matrix.to/#/@vic:x">@Vic</a>"#);
        assert_eq!(c["m.mentions"], json!({ "user_ids": ["@vic:x"] }));
        assert_eq!(c["m.relates_to"], json!({ "m.in_reply_to": { "event_id": "$q" } }));
        assert_eq!((&r[REVISION], &r["device_id"], &r["ts"]), (&json!("r1"), &json!("DEV"), &json!(7)));
        assert_eq!(r["send_later"], json!({ "ts": 4_000_000_000_000_i64, "cancel_on_reply": true }));
        assert!(wire(&st, &d, "DEV", 9, false).get("send_later").is_none(), "a limited plan keeps Send Later here");
        assert_eq!(draft_of(&r, 10), Some(d.clone()));
        assert_eq!(draft_of(&r, 5_000_000_000_000).and_then(|d| d.send_later), None, "a passed time goes");
        let html = "see <em>this</em> from <a href=\"https://matrix.to/#/@una:x\" class=\"mention\" \
            onclick=\"return false\">@Una</a><br>line <code>2</code>";
        let desktop = json!({ REVISION: "r2", "device_id": "D2", "ts": 3, "send_room_id": null, "send_later": null,
            "contents": [{ "msgtype": "m.text", "body": "see this from @Una\nline 2",
                "format": "org.matrix.custom.html", "formatted_body": html,
                "m.mentions": { "user_ids": ["@una:x"] } }] });
        let got = draft_of(&desktop, 0).expect("a draft");
        assert_eq!(got.text.as_deref(), Some("see _this_ from @Una\nline `2`"));
        assert_eq!(got.mentions, [("Una".to_string(), "@una:x".to_string())]);
        assert_eq!((got.send_room_id, got.send_later, got.revision_id.as_deref()), (None, None, Some("r2")));
        assert_eq!(draft_of(&tombstone("D"), 0), None);
        let moved = json!({ "contents": desktop["contents"], "ts": 99, "device_id": "D3" });
        assert_eq!(content_key(&desktop), content_key(&moved), "the revision, device and time do not count");
    }

    /// Desktop's draft revisions: what syncs of a draft keeps its revision, and a change gets a new one. With the
    /// flag off a synced record waits; the flag on takes it into a chat without a draft. An open composer with a
    /// change of its own holds a record until its chat closes without a draft.
    #[test]
    fn synced_records_wait_for_the_flag_and_the_draft_here() {
        let mut st = BeeperStore::from_config(crate::testing::demo_config("draft-sync-store"));
        let (idx, id) = (0, st.chats[0].id.clone());
        st.put_draft(idx, Some(ChatDraft { text: Some("a".into()), ts: Some(1), ..ChatDraft::default() }));
        let rev = st.chats[idx].draft.as_ref().and_then(|d| d.revision_id.clone()).expect("a revision");
        st.put_draft(idx, Some(ChatDraft { text: Some("a".into()), ts: Some(2), ..ChatDraft::default() }));
        assert_eq!(st.chats[idx].draft.as_ref().and_then(|d| d.revision_id.clone()), Some(rev), "the same text");
        st.put_draft(idx, None);
        st.draft_received(&id, rec("r1", "OTHER", "from phone"));
        assert_eq!(st.chats[idx].synced_draft.as_deref(), Some(&synced(rec("r1", "OTHER", "from phone"), false)));
        assert!(st.chats[idx].draft.is_none(), "the flag is off");
        st.features.flags = json!({ "features": { "*": { FLAG: true } } });
        st.replay_drafts();
        assert_eq!(draft_text(&st, idx), Some("from phone"));
        assert!(st.chats[idx].synced_draft.as_ref().is_some_and(|s| s.applied));
        let busy = Composer { accepts: false, unsyncable: false };
        st.drafts.link.lock().unwrap().open.insert(id.clone(), busy);
        st.draft_received(&id, rec("r2", "OTHER", "newer"));
        assert_eq!(draft_text(&st, idx), Some("from phone"), "an unsaved change holds the record");
        st.reconcile_unapplied(&id);
        assert!(st.chats[idx].synced_draft.as_ref().is_some_and(|s| !s.applied), "an open chat keeps it waiting");
        st.leave_draft(&id);
        assert_eq!(draft_text(&st, idx), Some("from phone"), "the draft here wins");
        st.put_draft(idx, None);
        st.leave_draft(&id);
        assert_eq!(draft_text(&st, idx), Some("newer"), "the waiting record comes once the draft here went");
        st.drafts.link.lock().unwrap().open.insert(id.clone(), Composer { accepts: true, unsyncable: false });
        st.draft_received(&id, rec("r3", "OTHER", "newest"));
        assert_eq!(draft_text(&st, idx), Some("newest"), "a draft that matches the synced one takes the next");
        assert_eq!(
            std::mem::take(&mut st.drafts.link.lock().unwrap().applied),
            std::slice::from_ref(&id),
            "the composer shows it"
        );
        st.draft_received(&id, json!({ REVISION: "r4", "device_id": "OTHER" }));
        assert_eq!(st.chats[idx].draft, None, "a removal clears it");
    }
}
