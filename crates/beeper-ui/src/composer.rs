//! The composer: its drafts, the autocomplete, the attachments and link previews, the composer buttons, and the send
//! of a message.

use crate::chat_list::{message_tag, preview_body};
use crate::gifs;
use crate::gifs::gif_width;
use crate::i18n::{tr, trf};
use crate::scheduled;
use crate::scheduled::send_day;
use crate::style::{
    bordered_box, brand_svg, btn_style, clamp, fill_box, hex, ic, icon, input_style, mix, scroll_style, semi,
    shadow_box, text_width, txt, Bare, CLEAR,
};
use crate::threads;
use crate::units::human_size;
use crate::vcard::parse_vcard;
use crate::{
    account_display_name, account_user, anim, brand_key, can_broadcast, checkbox, clipboard_paste, contact_card,
    dashed, emoji, format, has_flag, hotkey_key, icon_btn, images, locate, menu_item, menu_rule, network_name, portal,
    record, removable, several_accounts, spell, switch_targets, text_urls, tinted, toasts, truncate, vlist, voice,
    BeeperGuiApp, Clip, El, Frames, MenuEdit, Pane, PendingMedia, Picker, Queued, Toast, UiMessage, CHEVRON_DOWN,
    COMPOSER_ICONS, FILE_ICONS, MENU_ITEM, MESSAGE_MENU_W, SEND_ARROW,
};
use beeper_core::config::{account_network, now_unix_millis};
use beeper_core::matrix::{Mention, ROOM_MENTION};
use beeper_core::models::*;
use beeper_core::prefs;
use beeper_core::store::{BeeperStore, Draft, OutFile};
use iced::font::Weight;
use iced::keyboard::{key::Named, Key};
use iced::widget::operation;
use iced::widget::scrollable::{Direction, Scrollbar};
use iced::widget::{
    button, column, container, image, mouse_area, row, scrollable, stack, svg, text_editor, text_input, tooltip,
    Column, Row, Space,
};
use iced::{window, Bottom, Center, Color, Fill, Length, Padding, Pixels, Shrink, Task};
use std::collections::HashSet;

/// Composer content with `text` and the cursor at the end.
pub(crate) fn composer_with(text: &str) -> text_editor::Content {
    format::content(text, text.len()..text.len())
}

/// A misspelled word of the composer and its suggestions.
pub(crate) type Typo = (std::ops::Range<usize>, Vec<String>);

/// A row of the composer autocomplete: an emoji and its name, or a mention label, its user ID and the member.
pub enum Suggestion {
    Emoji(String, &'static str),
    Mention(String, String, Option<Box<User>>),
}

/// Desktop's `xk`: the label of a mention of `u`.
pub(crate) fn mention_label(u: &User) -> String {
    let username = u.username.as_deref().map(|n| n.strip_prefix('@').unwrap_or(n));
    [u.full_name.as_deref(), username, u.email.as_deref(), u.phone_number.as_deref()]
        .into_iter()
        .flatten()
        .find(|s| !s.is_empty())
        .unwrap_or(&u.id)
        .to_string()
}

/// Desktop's composer `linkPreviews` entry: a `com.beeper.linkpreviews` entry and its image when loaded.
pub enum LinkState {
    Loading,
    Loaded(serde_json::Value, Option<(Vec<u8>, image::Handle)>),
    Discarded,
}

/// The mention pills of Matrix HTML: the `matrix.to` links of users, with the label after "@".
pub(crate) fn html_pills(html: &str) -> Vec<Mention> {
    let runs = beeper_core::codec::html_blocks(html).into_iter().flat_map(|b| b.runs);
    let user = |link: &str| {
        beeper_core::codec::url_decode(link.strip_prefix("https://matrix.to/#/")?).filter(|id| id.starts_with('@'))
    };
    runs.filter_map(|r| Some((r.text.strip_prefix('@')?.to_string(), user(r.link.as_deref()?)?))).collect()
}

/// The mention pills that the text still has, once for each label.
fn pills(text: &str, mentions: &[Mention]) -> Vec<Mention> {
    let mut labels = HashSet::new();
    let kept = mentions.iter().filter(|(label, _)| text.contains(&format!("@{label}")) && labels.insert(label));
    kept.cloned().collect()
}

/// Desktop's `persistDraftRecordNow`: saves the composer state of the chat open in `pane` as its draft, or the state
/// that an edit put aside. The attachments go to the uploads once and stay there while the draft keeps them.
pub(crate) fn save_draft(st: &mut BeeperStore, pane: &Pane) {
    let Some(chat) = pane.selected_chat_id.as_deref() else { return };
    let (text, reply, files, links, later, mentions) = match &pane.stash {
        Some(s) => (s.text.clone(), &s.reply_to, &s.attachments, &s.links, s.send_later, &s.mentions),
        None => {
            let (reply, links) = (&pane.reply_to_message_id, &pane.composer_links);
            (pane.composer.text(), reply, &pane.attachments, links, pane.send_later, &pane.mentions)
        }
    };
    let idx = st.resolve_chat_idx(chat).expect("the open chat exists");
    let old: Vec<ChatDraftAttachment> = st.chats[idx].draft.iter().flat_map(|d| d.attachments.clone()).collect();
    let old_ids: Vec<String> = old.iter().map(|a| a.upload_id.clone()).collect();
    let kept = |(a, f): (&ChatDraftAttachment, &PendingMedia)| {
        st.assets.get(&a.upload_id).is_some_and(|s| s.file_name == f.name && s.file_size == f.bytes.len() as u64)
    };
    let attachments = match old.len() == files.len() && old.iter().zip(files).all(kept) {
        true => old,
        false => files
            .iter()
            .map(|f| {
                let a = st.store_uploaded_asset(f.name.clone(), f.mime.clone(), &f.bytes);
                ChatDraftAttachment {
                    upload_id: a.upload_id,
                    file_name: Some(a.file_name),
                    mime_type: Some(a.mime_type),
                }
            })
            .collect(),
    };
    let discarded = links.iter().filter(|(_, s)| matches!(s, LinkState::Discarded)).map(|(u, _)| u.clone());
    let mentions = pills(&text, mentions);
    let draft = ChatDraft {
        text: Some(text).filter(|t| !t.trim().is_empty()),
        attachments,
        reply_to_message_id: reply.clone(),
        send_later: later,
        discarded_links: discarded.collect(),
        mentions,
        ts: None,
    };
    // Desktop's `applyDraftTime`: the draft time changes with the text or the files.
    let old = st.chats[idx].draft.as_ref();
    let same = old.is_some_and(|o| o.text == draft.text && o.attachments == draft.attachments);
    let ts = if same { old.and_then(|o| o.ts) } else { Some(now_unix_millis()) };
    let draft = (draft.text.is_some() || !draft.attachments.is_empty()).then_some(ChatDraft { ts, ..draft });
    if draft != st.chats[idx].draft {
        st.patch_chat(chat, true, None, None).expect("clear draft");
        st.patch_chat(chat, false, draft, None).expect("save draft");
        // A sent draft or a changed file leaves its upload behind.
        st.drop_unused_uploads(&old_ids);
    }
}

/// Desktop's `restoreDraftSnapshot`: the attachments of a saved draft. A draft from the API's focus endpoint names its
/// attachments by file path.
pub(crate) fn load_draft(st: &BeeperStore, draft: &ChatDraft) -> Result<Vec<PendingMedia>, String> {
    let file = |a: &ChatDraftAttachment| match st.assets.contains_key(&a.upload_id) {
        true => load_upload(st, &a.upload_id),
        false => portal::load_file(std::path::Path::new(&a.upload_id)),
    };
    draft.attachments.iter().map(file).collect()
}

/// A stored upload as a file for the composer, with its first name.
fn load_upload(st: &BeeperStore, id: &str) -> Result<PendingMedia, String> {
    let (path, _) = st.resolve_asset_path(id).expect("a stored upload has a path");
    Ok(PendingMedia { name: st.assets[id].file_name.clone(), ..portal::load_file(&path)? })
}

/// Desktop's caption rule: the text goes with a single file as its caption where the chat takes captions on it.
fn caption(text: &str, files: &[PendingMedia], rules: &FileRules) -> Option<String> {
    let takes = |f: &PendingMedia| beeper_core::media::takes_caption(rules, &f.mime, f.is_gif, false);
    (!text.is_empty() && matches!(files, [f] if takes(f))).then(|| text.to_string())
}

/// Desktop's `KFe`: a composer file to send. A video and an audio file go with their measured length and size. An
/// image keeps its frames in `media`, under the URL of its local echo.
fn out_file(media: &mut images::Images<Result<Frames, String>>, st: &mut BeeperStore, f: PendingMedia) -> OutFile {
    let PendingMedia { bytes, name, mime, is_gif, size, frames, duration, sticker } = f;
    let image = !frames.is_empty() && mime.starts_with("image/");
    let key = format!("pasted:{}", st.next_id("img"));
    if image {
        media.insert(key.clone(), Some(Ok(frames)));
    }
    OutFile {
        bytes,
        name,
        mime,
        size: (size.0 > 0).then_some(size),
        duration,
        gif: is_gif,
        sticker: sticker.is_some(),
        pack: sticker,
        voice: None,
        echo_url: image.then_some(key),
    }
}

/// The composer state that Desktop's `saveDraft` keeps while you edit a message.
pub struct Stash {
    pub(crate) text: String,
    pub(crate) reply_to: Option<String>,
    pub(crate) attachments: Vec<PendingMedia>,
    pub(crate) links: Vec<(String, LinkState)>,
    pub(crate) send_later: Option<(i64, bool)>,
    pub(crate) mentions: Vec<Mention>,
}

const ATTACH_BUTTONS: [u8; 8] = [0, 7, 3, 5, 2, 1, 4, 6];

/// Desktop's composer action state (`rIt`, `_R` and `oIt`): the tooltip, the hotkey that it shows, and whether the
/// button works. Talk to Type is the only action while you edit a message.
pub(crate) fn composer_action(
    st: &BeeperStore,
    chat: &Chat,
    id: u8,
    editing: bool,
) -> (String, Option<&'static str>, bool) {
    let (label, hotkey, plus) = COMPOSER_ACTION_INFO[usize::from(id)];
    let supported = match id {
        2 => chat.takes(FILE_GIF),
        5 => chat.takes(FILE_VOICE),
        6 => chat.polls.is_some(),
        7 => chat.takes(FILE_STICKER),
        _ => true,
    };
    let usable = supported && (id == 1 || !editing);
    let locked = plus.is_some_and(|f| !st.feature(f).on);
    let label = match () {
        _ if locked => trf("%s (Upgrade to Beeper Plus)", &[&tr(label)]),
        _ if editing && id != 1 => trf("%s (Unavailable while editing)", &[&tr(label)]),
        _ if supported => tr(label).to_string(),
        _ => trf("%1$s (Unavailable for %2$s)", &[&tr(label), &network_name(&chat.account_id)]),
    };
    (label, hotkey.filter(|_| usable), usable && !locked)
}

/// Desktop's composer action configs (`Ome`) by composer button: the label, the hotkey, and the Beeper Plus feature.
const COMPOSER_ACTION_INFO: [(&str, Option<&str>, Option<&str>); 8] = [
    ("Add Emoji", None, None),
    ("Talk to Type", None, Some("talk-to-type")),
    ("Add GIF", Some("Message: Send GIF"), None),
    ("Add Attachment", Some("Chat: Send File"), None),
    ("Send Later", Some("Message: Send Later"), Some("scheduled-message")),
    ("Record Audio", Some("Message: Record Audio"), None),
    ("Create Poll", None, None),
    ("Add a sticker", Some("Message: Send Sticker"), None),
];

/// Desktop's `ComposerHiddenActions`: the actions behind the plus button. Create Poll needs `enable-polls`, and
/// "Add a sticker" needs "Use sticker packs".
pub(crate) fn hidden_actions(st: &BeeperStore) -> Vec<u8> {
    let (pri, sec) = composer_buttons(st);
    let shown = |x: u8| match x {
        6 => st.feature("enable-polls").on,
        7 => st.settings.on("stickerPacks"),
        _ => true,
    };
    ATTACH_BUTTONS.into_iter().filter(|&x| Some(x) != pri && Some(x) != sec && shown(x)).collect()
}

/// The composer buttons in the pill by "Primary composer action" and "Secondary composer action".
fn composer_buttons(st: &BeeperStore) -> (Option<u8>, Option<u8>) {
    let action = |key| [Some(0), Some(1), Some(5), Some(2), Some(3), Some(4), None][usize::from(st.settings.pref(key))];
    let pri = action("primaryComposerButton");
    (pri, action("secondaryComposerButton").filter(|&x| Some(x) != pri))
}

impl BeeperGuiApp {
    /// Desktop's `processAttachment`: attaches each file that the open chat accepts, and shows a toast for the others.
    pub(crate) fn attach_files(&mut self, files: Vec<Result<PendingMedia, String>>) {
        let st = self.store.read();
        let chat = &st.chats
            [st.resolve_chat_idx(self.pane.selected_chat_id.as_deref().expect("an open chat")).expect("chat exists")];
        let checked: Vec<_> = files
            .into_iter()
            .map(|f| f.and_then(|f| portal::check(&f, &chat.file, chat.room_features).map(|()| f)))
            .collect();
        drop(st);
        for f in checked {
            match f {
                Ok(f) => self.pane.attachments.push(f),
                Err(e) => self.fail(Toast::new(e)),
            }
        }
    }

    /// Desktop's `addFileBuffers` of a sticker, and `addRecentSticker`: the newest of 24 recent stickers.
    pub(crate) fn sticker_loaded(&mut self, r: Result<PendingMedia, String>) {
        let f = match r {
            Ok(_) if self.pane.editing_message_id.is_some() => {
                return self.status_banner = tr("Attachments can't be added while editing a message").into();
            }
            Ok(f) => f,
            Err(e) => {
                eprintln!("ComposeMessage: loading sticker failed: {e}");
                return self.status_banner = tr("Couldn't load this sticker. Try again.").into();
            }
        };
        let s = f.sticker.clone().expect("a sticker");
        self.pane.attachments.push(f);
        self.patch_settings(|st| {
            st.recent_stickers.retain(|r| r.url != s.url);
            st.recent_stickers.insert(0, *s);
            st.recent_stickers.truncate(24);
        });
    }

    /// Desktop's composer autocomplete (`TextAreaAutocomplete`) for the "@" or ":" word before the caret: the byte
    /// offset of the trigger, and up to 8 members (`createMentionTriggerSettings`) or the emoji.
    pub(crate) fn suggestions(&self, st: &BeeperStore) -> Option<(usize, Vec<Suggestion>)> {
        let pane = self.pane();
        let text = pane.composer.text();
        let before = &text[..format::offset(&text, pane.composer.cursor().position)];
        let at = before.rfind(['@', ':', '\n']).filter(|_| pane.suggest_hidden.as_ref() != Some(&text))?;
        let query = &before[at + 1..];
        // Desktop's `findSuggestionMatch` (`[\p{L}\d+\-_]*` after the trigger): ":)" or "@a.b" suggest nothing.
        let word = |c: char| c.is_alphabetic() || c.is_ascii_digit() || "+-_".contains(c);
        let picks: Vec<Suggestion> = match before.as_bytes()[at] {
            _ if !query.chars().all(word) => return None,
            b'@' => {
                let chat = &st.chats[st.resolve_chat_idx(pane.selected_chat_id.as_deref()?).expect("the open chat")];
                let key =
                    |u: &&User| [&u.full_name, &u.username, &u.email].map(|k| k.clone().unwrap_or_default()).to_vec();
                let room = chat.can_mention_room && !query.is_empty() && "room".starts_with(&query.to_lowercase());
                let room = room.then(|| Suggestion::Mention("room".into(), ROOM_MENTION.into(), None));
                let users = prefs::match_sort_keys(chat.participants.items.iter(), query, key).into_iter();
                let users =
                    users.map(|u| Suggestion::Mention(mention_label(u), u.id.clone(), Some(Box::new(u.clone()))));
                room.into_iter().chain(users).take(8).collect()
            }
            b':' if st.settings.on("emojiAutocomplete") => {
                let picks = emoji::suggestions(':', query, st.skin_tone).into_iter();
                picks.map(|(c, name)| Suggestion::Emoji(c, name)).collect()
            }
            _ => return None,
        };
        (!picks.is_empty()).then_some((at, picks))
    }

    /// The autocomplete list, with its left edge at the trigger in the composer and its bottom at the text top.
    pub(crate) fn view_suggestions(&self, st: &BeeperStore) -> Option<El<'_>> {
        let p = &self.pal;
        let (trigger, picks) = self.suggestions(st)?;
        let at = self.pane().suggest.min(picks.len() - 1);
        let label = |s: &Suggestion| match s {
            Suggestion::Emoji(c, name) => format!("{c} {name}"),
            Suggestion::Mention(label, ..) => format!("@{label}"),
        };
        let face = |s: &Suggestion| match s {
            Suggestion::Emoji(..) => None,
            Suggestion::Mention(_, _, Some(u)) => {
                Some(self.face(u.img_url.as_deref(), u.full_name.as_deref().unwrap_or_default(), 24.0, 0.0))
            }
            Suggestion::Mention(_, _, None) => {
                let chat = self.pane().selected_chat_id.as_deref().and_then(|c| st.resolve_chat_idx(c));
                Some(self.chat_avatar(&st.chats[chat.expect("the open chat")], 24.0, 0.0))
            }
        };
        let faces = if matches!(picks[0], Suggestion::Mention(..)) { 30.0 } else { 0.0 };
        let width =
            picks.iter().map(|s| text_width(&label(s), 13.0, Weight::Normal)).fold(0.0, f32::max) + 12.0 + faces;
        let items = picks.iter().enumerate().map(|(i, s)| {
            let (bg, fg) = if i == at { (p.accent, Color::WHITE) } else { (CLEAR, p.text) };
            let name = txt(label(s), 13.0).wrapping(iced::widget::text::Wrapping::None);
            let b = button(row![].push(face(s)).push(name).spacing(6).align_y(Center)).padding([4, 6]).width(width);
            b.style(btn_style([bg; 3], [fg; 3], 4.0)).on_press(UiMessage::Composer(Msg::PickSuggestion(i))).into()
        });
        let list =
            container(Column::with_children(items).padding([4, 4])).style(shadow_box(p.menu, p.menu_border, 6.0, 16.0));
        let unveiled = hidden_actions(st).len() as f32;
        let plus = if self.pane().show_attach_menu { 36.0 + 6.0 + 2.0 + unveiled * 30.0 - 2.0 } else { 36.0 };
        let text = self.pane().composer.text();
        let line = &text[text[..trigger].rfind('\n').map_or(0, |i| i + 1)..trigger];
        let x = 8.0 + plus + 8.0 + 14.0 + 2.0 + text_width(line, 13.0, Weight::Normal);
        let x = x.min(self.pane().size.width / self.zoom - self.left_width() - width - 36.0);
        Some(container(list).align_bottom(Fill).padding(Padding::ZERO.left(x).bottom(34.0)).into())
    }

    /// Saves the drafts of the chats open in the main window and the chat windows.
    pub(crate) fn save_drafts(&mut self) {
        let mut st = self.store.write();
        for pane in std::iter::once(&mut self.pane).chain(self.chat_windows.iter_mut().map(|w| &mut w.1)) {
            save_draft(&mut st, pane);
            pane.draft_due = false;
        }
    }

    /// Adds an edit from `old` to the history, and reacts to the new text.
    pub(crate) fn composer_changed(&mut self, old: String) -> Task<UiMessage> {
        let h = &mut self.pane.history;
        let new = self.pane.composer.text();
        if old != new {
            let (head, old_end, new_end) = format::diff(&old, &new);
            let near = |(t, r): &(std::time::Instant, std::ops::Range<usize>)| {
                t.elapsed().as_millis() < 100 && head <= r.end && old_end >= r.start
            };
            if !h.last.as_ref().is_some_and(near) {
                h.undo.push(old);
                if h.undo.len() > 100 {
                    h.undo.remove(0);
                }
            }
            h.redo.clear();
            h.last = Some((std::time::Instant::now(), head..new_end));
        }
        self.composer_effects()
    }

    /// Whether the open chat has sends that wait for "Allow undo send".
    pub(crate) fn queued_here(&self) -> bool {
        self.queued.iter().any(|q| Some(&q.chat) == self.pane().selected_chat_id.as_ref())
    }

    /// Desktop's reaction to new composer text: the typing notice and link previews.
    pub(crate) fn composer_effects(&mut self) -> Task<UiMessage> {
        self.pane.draft_due = true;
        self.pane.selected_msgs.clear();
        self.pane.link_dialog = None;
        self.pane.suggest = 0;
        // Desktop types nothing for an empty composer or a Send Later message.
        let typing = match self.pane.composer.is_empty() || self.pane.send_later.is_some() {
            true => Task::none(),
            false => self.typing(true),
        };
        Task::batch([typing, self.fetch_link_previews()])
    }

    /// Desktop's `onTyping` and `stopTyping`: one typing notice when you start to type in a chat, and its end when
    /// you send. [`UiMessage::TypingTick`] ends it 3 seconds after the last key.
    pub(crate) fn typing(&mut self, on: bool) -> Task<UiMessage> {
        let chat = self.pane.selected_chat_id.clone().filter(|_| on);
        // The notice of another chat, or of this chat when you stop, ends now.
        let ended = self.pane.typing.take_if(|(c, _)| Some(&*c) != chat.as_ref()).map(|(c, _)| c);
        let started = chat.clone().filter(|_| self.pane.typing.is_none());
        self.pane.typing = chat.map(|c| (c, std::time::Instant::now()));
        let st = self.store.read();
        let Some(client) = st.matrix.clone().filter(|_| st.typing_notices()) else { return Task::none() };
        drop(st);
        let notice = |chat, on| Task::perform(beeper_core::matrix::typing(client.clone(), chat, on), UiMessage::Pushed);
        Task::batch([ended.map(|c| notice(c, false)), started.map(|c| notice(c, true))].into_iter().flatten())
    }

    /// Desktop's `sendMessage`: a new message moves an archived chat to the inbox, unless `archive` is set.
    pub fn submit_composer(&mut self, archive: bool) {
        let chat_id = self.pane.selected_chat_id.clone().expect("a chat is selected");
        let text = self.pane.composer.text().trim().to_string();
        if text.is_empty() && self.pane.attachments.is_empty() {
            return;
        }
        let mentions = pills(&text, &std::mem::take(&mut self.pane.mentions));
        let mut st = self.store.write();
        let idx = st.resolve_chat_idx(&chat_id).expect("chat exists");
        let edit = self.pane.editing_message_id.is_some();
        let later = self.pane.send_later.take();
        if self.pane.editing_message_id.is_none()
            && later.is_none()
            && has_flag(&st.chats[idx], ChatFlag::Archived) != archive
        {
            self.set_archived(&mut st, idx, archive);
        }
        if let Some(id) = self.pane.editing_message_id.take_if(|id| st.scheduled.iter().any(|s| s.id == *id)) {
            st.update_scheduled(&id, |s| {
                s.text = text;
                true
            });
        } else if let Some((at, no_reply)) = later {
            let cid = st.chats[idx].id.clone();
            if !st.scheduled.iter().any(|s| s.chat_id == cid && !s.canceled) {
                self.pane.sched_fold = (false, false);
            }
            // Desktop's `scheduleCurrentDraft`: one entry for each content part, with the reply on the first.
            let mut reply_to = self.pane.reply_target();
            let files = std::mem::take(&mut self.pane.attachments);
            let caption = caption(&text, &files, &st.chats[idx].file);
            if !text.is_empty() && caption.is_none() {
                st.schedule(&chat_id, text, reply_to.take(), (at, no_reply), None);
            }
            for f in files {
                let upload = st.store_uploaded_asset(f.name, f.mime, &f.bytes).upload_id;
                st.schedule(
                    &chat_id,
                    caption.clone().unwrap_or_default(),
                    reply_to.take(),
                    (at, no_reply),
                    Some(upload),
                );
            }
        } else if let Some(target) = self.pane.editing_message_id.take() {
            st.edit_message(&chat_id, &target, text, mentions).expect("edit message");
        } else {
            let reply_to = self.pane.reply_target();
            let files = std::mem::take(&mut self.pane.attachments);
            let delay =
                st.feature("undo-send").on.then(|| prefs::UNDO_SEND_MS[usize::from(st.settings.pref("undoSendDelay"))]);
            let delay = delay.filter(|&ms| ms > 0);
            let restore = delay.map(|_| (text.clone(), reply_to.clone(), files.clone()));
            let urls = text_urls(&text);
            let preview = self.pane.composer_links.iter().find_map(|(u, s)| match s {
                LinkState::Loaded(v, img) if urls.contains(&u.as_str()) => {
                    Some((v.clone(), img.as_ref().map(|(b, _)| b.clone())))
                }
                _ => None,
            });
            let files = files.into_iter().map(|f| out_file(&mut self.media, &mut st, f)).collect();
            let text = Some(text).filter(|t| !t.is_empty());
            let links = !self.pane.composer_links.is_empty();
            let draft = Draft { text, reply_to, mentions, preview, thread: self.pane.send_thread(), files, links };
            let (_, sends) = st.echo_draft(&chat_id, draft).expect("send a message");
            // Desktop's `queuePendingMessages` sends the earlier queue of the chat first.
            if let Some(i) = self.queued.iter().position(|q| q.chat == chat_id) {
                for (_, out) in self.queued.remove(i).sends {
                    st.queue(&chat_id, out);
                }
            }
            match (delay, restore) {
                (Some(ms), Some((text, reply_to, files))) => {
                    let due = now_unix_millis() + ms;
                    let sends = sends.into_iter().map(|o| (o.message().expect("an echo").to_string(), o)).collect();
                    self.queued.push(Queued { chat: chat_id.clone(), due, sends, text, reply_to, files });
                }
                _ => sends.into_iter().for_each(|out| st.queue(&chat_id, out)),
            }
            // A reply in the thread pane reads the thread, not the chat.
            if self.pane.thread_root.is_none() {
                st.mark_chat_read(&chat_id, true);
            }
        }
        if edit {
            self.pane.end_edit();
        } else {
            (self.pane.composer, self.pane.broadcast) = (composer_with(""), false);
            self.pane.composer_links.clear();
        }
        drop(st);
        self.refresh_theme_and_tray();
    }

    /// Desktop's `ScheduledStore.sendMessage`: a typed message or a file, which moves an archived chat to the inbox.
    pub(crate) fn send_scheduled(&mut self, due: Vec<Scheduled>) {
        let mut st = self.store.write();
        let mut failed = Vec::new();
        for s in due {
            let Some(idx) = st.resolve_chat_idx(&s.chat_id) else { continue };
            if has_flag(&st.chats[idx], ChatFlag::Archived) {
                self.set_archived(&mut st, idx, false);
            }
            let text = Some(s.text).filter(|t| !t.is_empty());
            let files = match s.upload {
                None => Vec::new(),
                // "Clear storage" can remove the upload before the send.
                Some(id) => {
                    let f = match st.assets.contains_key(&id) {
                        true => load_upload(&st, &id),
                        false => Err(format!("Couldn't attach {id}")),
                    };
                    let f = match f {
                        Ok(f) => f,
                        Err(e) => {
                            failed.push(e);
                            continue;
                        }
                    };
                    // Desktop removes the file of a sent message.
                    st.drop_unused_uploads(&[id]);
                    vec![out_file(&mut self.media, &mut st, f)]
                }
            };
            let draft = Draft { text, reply_to: s.reply_to, files, ..Draft::default() };
            st.send(&s.chat_id, draft).expect("send scheduled");
        }
        drop(st);
        failed.into_iter().for_each(|e| self.fail(Toast::new(e)));
        self.refresh_theme_and_tray();
    }

    /// Desktop's `fetchLinkPreviews`: forget the links that left the composer, and load the new ones unless "Disable
    /// link previews" is on or the network cannot send previews.
    pub(crate) fn fetch_link_previews(&mut self) -> Task<UiMessage> {
        use beeper_core::models::NetworkKind::*;
        let urls: Vec<String> = text_urls(&self.pane.composer.text()).into_iter().map(str::to_string).collect();
        self.pane.composer_links.retain(|(u, _)| urls.contains(u));
        let st = self.store.read();
        let chat = self.pane.selected_chat_id.as_ref().and_then(|c| st.resolve_chat_idx(c)).map(|i| &st.chats[i]);
        let kind = chat.map(|c| account_network(&c.account_id).kind);
        let can =
            matches!(kind, Some(Beeper | WhatsApp | TwitterX | Telegram | Slack | GoogleMessages | Signal | Line));
        if !can || st.settings.on("noOutgoingLinkPreviews") {
            return Task::none();
        }
        drop(st);
        let mut tasks = Vec::new();
        for url in urls {
            if self.pane.composer_links.iter().all(|(u, _)| *u != url) {
                self.pane.composer_links.push((url.clone(), LinkState::Loading));
                let load = beeper_core::cloud::link_preview(url.clone());
                tasks.push(Task::perform(load, move |r| UiMessage::Composer(Msg::LinkPreviewLoaded(url, r))));
            }
        }
        Task::batch(tasks)
    }

    /// Electron's context menu of the composer: the suggestions for a misspelled word, "Learn Spelling", "Search with
    /// Google" for the selection, and Cut, Copy and Paste.
    pub(crate) fn view_edit_menu(&self, word: Option<&Typo>) -> (El<'_>, f32) {
        let p = &self.pal;
        let text = self.pane().composer.text();
        let sel = format::selection(&self.pane().composer).map(|r| text[r].trim().to_owned()).filter(|s| !s.is_empty());
        let item = |label: &str, msg: Option<UiMessage>| Some(menu_item(p, None, label.into(), "", msg));
        let edit = |e: MenuEdit| Some(UiMessage::Composer(Msg::MenuEdit(e)));
        let mut items: Vec<Option<El<'_>>> = Vec::new();
        if let Some((_, fixes)) = word {
            items.extend(fixes.iter().map(|f| item(f, edit(MenuEdit::Replace(f.clone())))));
            items.extend(fixes.is_empty().then(|| item("No Guesses Found", None)));
            items.extend([Some(menu_rule(p)), item("Learn Spelling", edit(MenuEdit::Learn)), Some(menu_rule(p))]);
        }
        if let Some(q) = &sel {
            let q: String = q
                .bytes()
                .map(|b| match b {
                    b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'*' => char::from(b).to_string(),
                    b' ' => "+".into(),
                    b => format!("%{b:02X}"),
                })
                .collect();
            let google = UiMessage::OpenUrl(format!("https://www.google.com/search?q={q}"));
            items.extend([item("Search with Google", Some(google)), Some(menu_rule(p))]);
        }
        let has = sel.is_some();
        items.extend([
            item("Cut", has.then(|| UiMessage::Composer(Msg::MenuEdit(MenuEdit::Cut)))),
            item("Copy", has.then(|| UiMessage::Composer(Msg::MenuEdit(MenuEdit::Copy)))),
            item("Paste", edit(MenuEdit::Paste)),
        ]);
        let rules = word.map_or(0, |_| 2) + usize::from(has);
        let h = 8.0 + (items.len() - rules) as f32 * MENU_ITEM + rules as f32 * 11.0;
        let menu = container(Column::with_children(items.into_iter().flatten()).padding([4, 0]))
            .width(MESSAGE_MENU_W)
            .style(shadow_box(p.menu, p.menu_border, 6.0, 16.0));
        (menu.into(), h)
    }

    /// Desktop's `composeTextareaPlaceholder`.
    pub(crate) fn composer_placeholder(&self, st: &BeeperStore, chat: &Chat) -> String {
        let pane = self.pane();
        if pane.thread_root.is_some() {
            return tr("Reply in thread…").into();
        }
        let to = match (pane.send_later, pane.reply_thread.is_some() || pane.reply_to_message_id.is_some()) {
            (Some(_), _) => "Send Later".into(),
            (None, reply) => {
                format!("{} {}", if reply { "Reply to" } else { "Message" }, truncate(chat.title.clone(), 128))
            }
        };
        let request = match (chat.is_request == Some(true), chat.chat_type == "group") {
            (false, _) => "",
            (true, true) => " to join the group",
            (true, false) => " to accept the request",
        };
        // Desktop's `bridgeProtocolDisplayName`: the user of the account, when you have more accounts of its bridge.
        let several = several_accounts(st, &chat.account_id);
        let user = Some(account_user(st, &chat.account_id)).filter(|u| several && !u.is_empty());
        let net = network_name(&chat.account_id);
        let net = user.map_or(net.to_string(), |u| format!("{net} ({u})"));
        format!("{to} on {net}{request}")
    }

    pub(crate) fn view_composer<'a>(&'a self, st: &BeeperStore, chat: &Chat) -> El<'a> {
        let p = &self.pal;
        // Desktop's quoted message in the composer (`bfe`): the sender in groups, and at most 128 characters on at most
        // half of the width.
        let quote = self.pane().reply_to_message_id.as_ref().and_then(|r| st.loaded_idx(r)).map(|i| {
            let q = &st.messages[i];
            let text = preview_body(q).map(|t| t.split_whitespace().collect::<Vec<_>>().join(" "));
            let text = text.or_else(|| message_tag(q).map(|t| t.1.to_string())).unwrap_or_default();
            let text = truncate(text, 128);
            let name = (chat.chat_type == "group").then(|| semi(q.sender_name.clone().expect("sender name"), 12.0));
            let body = column![].push(name.map(|n| n.color(p.strong))).push(txt(text, 12.0).color(p.text)).spacing(2);
            let (edge, fill) = match q.is_sender == Some(true) {
                true => (p.sent, mix(p.card, p.sent, 0.3)),
                false => (p.received, p.card),
            };
            let pill = container(body).max_width(self.pane().size.width / self.zoom / 2.0).padding(8);
            removable(p, pill.style(bordered_box(fill, edge, 2.0, 17.0)), UiMessage::Composer(Msg::CancelMode))
        });
        let gif_picker = (self.pane().picker == Some(Picker::Gif)).then(|| {
            let search = text_input(tr("Search KLIPY..."), &self.gif_query)
                .id("gif-search-input")
                .on_input(|a0| UiMessage::Gifs(gifs::Msg::Query(a0)))
                .size(13)
                .padding([6, 12])
                .style(input_style(p.input, p.strong, p.sub, p.accent, 8.0));
            // Desktop's `GifPicker`: the error or "No results" text, or 10 skeleton tiles while the first page loads.
            let note =
                |t: String| -> El<'a> { container(txt(t, 13.0).color(p.text)).center_x(Fill).center_y(215).into() };
            let carousel: El<'a> = if let Some(err) = self.gif_error.as_ref().filter(|_| self.gifs.is_empty()) {
                note(trf("Something went wrong (%s). Try searching something else?", &[err]))
            } else if self.gif_loading && self.gifs.is_empty() {
                let tile = || container(Space::new()).width(300).height(200).style(fill_box(p.hover, 4.0)).into();
                let tiles = Row::with_children((0..10).map(|_| tile())).spacing(4);
                container(tiles).padding(Padding::ZERO.left(14.0)).height(215).clip(true).into()
            } else if self.gifs.is_empty() {
                note(tr("No results. Try searching something else?").to_string())
            } else {
                let items = self.gifs.iter().enumerate().map(|(i, g)| {
                    let w = gif_width(g);
                    let cell: El<'a> = match self.media_at(&g.preview_url) {
                        Some(Ok(frames)) => anim::gif(frames, self.started, move |f| {
                            image(f).width(w).height(215).content_fit(iced::ContentFit::Cover)
                        }),
                        _ => container(Space::new()).width(w).height(215).style(fill_box(p.hover, 4.0)).into(),
                    };
                    // Desktop's `.gif.selected`: a 4 px outline in the primary color inside the GIF.
                    let ring = (self.gif_sel == Some(i)).then(|| {
                        container(Space::new()).width(w).height(215).style(bordered_box(CLEAR, p.accent, 4.0, 0.0))
                    });
                    button(stack![container(cell).clip(true).style(fill_box(CLEAR, 4.0))].push(ring))
                        .padding(0)
                        .style(btn_style([CLEAR, p.hover, p.selected], [p.text; 3], 4.0))
                        .on_press(UiMessage::Gifs(gifs::Msg::PickGif(g.clone())))
                        .into()
                });
                // Desktop's skeleton tiles while the next page loads.
                let tile = || container(Space::new()).width(300).height(200).style(fill_box(p.hover, 4.0)).into();
                let more = self
                    .gif_loading
                    .then(|| Row::with_children((0..10).map(|_| tile())).spacing(4).padding(Padding::ZERO.left(14.0)));
                let dir = Direction::Horizontal(Scrollbar::new().width(4).scroller_width(4));
                let row = Row::with_children(items).push(more).spacing(4);
                scrollable(container(row).padding(Padding::ZERO.left(14.0)))
                    .id("gif-carousel")
                    .on_scroll(|v| {
                        let (x, content, width) = (v.absolute_offset().x, v.content_bounds().width, v.bounds().width);
                        UiMessage::Gifs(gifs::Msg::Scrolled(x, content, width))
                    })
                    .direction(dir)
                    .height(215)
                    .width(Fill)
                    .style(scroll_style(p.scroll))
                    .into()
            };
            column![container(search).padding([0, 14]), carousel].spacing(8).padding(Padding::ZERO.top(4.0))
        });
        let editing = self.pane().editing_message_id.is_some();
        let action_btn = |id: u8| -> El<'a> {
            let (label, hotkey, on) = composer_action(st, chat, id, editing);
            let msg = match id {
                0 => Some(UiMessage::Emoji(emoji::EmojiMsg::Toggle)),
                2 => Some(UiMessage::Composer(Msg::TogglePicker(Picker::Gif))),
                3 => Some(UiMessage::Composer(Msg::PickFiles)),
                4 => Some(UiMessage::Scheduled(scheduled::Msg::Open)),
                1 => Some(UiMessage::Record(record::Msg::Talk)),
                5 => Some(UiMessage::Record(record::Msg::Start)),
                6 => Some(UiMessage::OpenPollCreator(chat.id.clone())),
                7 => Some(UiMessage::Composer(Msg::TogglePicker(Picker::Sticker))),
                _ => None,
            }
            .filter(|_| on);
            let active = (id == 2 && self.pane().picker == Some(Picker::Gif))
                || (id == 7 && self.pane().picker == Some(Picker::Sticker))
                || (id == 0 && self.pane().emoji.as_ref().is_some_and(|pk| pk.react.is_none() && pk.pref.is_none()));
            // Desktop's `Emoji-module__active`: the open emoji picker turns its button `--color-primary`.
            let (color, bg) = match (active, id) {
                (true, 0) => (p.accent, CLEAR),
                (true, _) => (p.strong, p.hover),
                (false, _) => (p.on_secondary, CLEAR),
            };
            let glyph = svg(COMPOSER_ICONS[usize::from(id) + 2].clone())
                .width(20)
                .height(20)
                .opacity(if msg.is_some() { 1.0_f32 } else { 0.3 })
                .style(move |_, _| svg::Style { color: Some(color) });
            let b = button(container(glyph).center(28))
                .padding(0)
                .style(btn_style([bg, p.hover, p.selected], [p.text; 3], 14.0))
                .on_press_maybe(msg);
            let caps = hotkey
                .map(|h| prefs::hotkey_label(st.settings.hotkey_keys(h).split(", ").next().expect("a combination")));
            let caps = caps.map_or_else(Vec::new, |c| c.split_whitespace().map(String::from).collect());
            match id {
                // Desktop shows "Add Emoji" when the picker does not open on hover.
                0 => {
                    let b = mouse_area(b)
                        .on_enter(UiMessage::Emoji(emoji::EmojiMsg::Pointer(0, true)))
                        .on_exit(UiMessage::Emoji(emoji::EmojiMsg::Pointer(0, false)));
                    match st.settings.on("emojiPickerOnHover") {
                        true => b.into(),
                        false => self.tip(b.into(), label, vec![], tooltip::Position::Top).into(),
                    }
                }
                _ => self.tip_caps(b.into(), label, caps, tooltip::Position::Top).into(),
            }
        };
        let (pri, sec) = composer_buttons(st);
        let placeholder = self.composer_placeholder(st, chat);
        let suggesting = self.suggestions(st).is_some();
        let formats = chat.formatting;
        let keys = ["Message: Send Message and Archive", "Send Message", "Insert New Line"]
            .map(|h| st.settings.hotkey_keys(h).to_string());
        let undo = self.queued_here().then(|| st.settings.hotkey_keys("Undo Pending Send").to_string());
        let marks = format::MarkStyle {
            mentions: self.pane().mentions.iter().map(|(label, _)| label.clone()).collect(),
            mention: p.accent,
            link: p.link,
            mark: p.tertiary,
        };
        let input = text_editor(&self.pane().composer)
            .id(if self.pane().thread_root.is_some() { "thread-composer-input" } else { "composer-input" })
            .placeholder(placeholder)
            .on_action(|a0| UiMessage::Composer(Msg::Edit(a0)))
            .key_binding(move |k| match k.key.as_ref() {
                _ if !matches!(k.status, text_editor::Status::Focused { .. }) => None,
                _ if undo.as_deref().zip(hotkey_key(&k.key)).is_some_and(|(keys, key)| {
                    let m = k.modifiers;
                    prefs::hotkey_has(keys, [m.control() || m.command(), m.shift(), m.alt()], &key)
                }) =>
                {
                    Some(text_editor::Binding::Custom(UiMessage::Composer(Msg::UndoSend)))
                }
                Key::Named(Named::Enter) => {
                    let m = k.modifiers;
                    let mods = [m.control() || m.command(), m.shift(), m.alt()];
                    match keys.iter().position(|keys| prefs::hotkey_has(keys, mods, "enter")) {
                        Some(0) => Some(text_editor::Binding::Custom(UiMessage::Composer(Msg::SendAndArchive))),
                        Some(1) => Some(text_editor::Binding::Custom(UiMessage::Composer(Msg::Submit))),
                        _ => Some(text_editor::Binding::Enter),
                    }
                }
                Key::Character("`") if !k.modifiers.command() => {
                    Some(text_editor::Binding::Custom(UiMessage::Composer(Msg::Format(format::CODE))))
                }
                Key::Named(key @ (Named::Backspace | Named::Delete))
                    if k.modifiers.jump() || k.modifiers.macos_command() =>
                {
                    use text_editor::Motion;
                    let back = key == Named::Backspace;
                    let to = match (back, k.modifiers.macos_command()) {
                        (true, false) => Motion::WordLeft,
                        (false, false) => Motion::WordRight,
                        (true, true) => Motion::Home,
                        (false, true) => Motion::End,
                    };
                    Some(text_editor::Binding::Custom(UiMessage::Composer(Msg::DeleteTo(to, back))))
                }
                Key::Named(Named::ArrowUp | Named::ArrowDown) if suggesting => None,
                Key::Named(Named::Escape) => None,
                Key::Character(_) if k.modifiers.command() => {
                    let i = match (k.key.to_latin(k.physical_key).map(|c| c.to_ascii_lowercase()), k.modifiers.shift())
                    {
                        (Some('z'), redo) => {
                            return Some(text_editor::Binding::Custom(UiMessage::Composer(Msg::Undo(redo))))
                        }
                        (Some('y'), false) => {
                            return Some(text_editor::Binding::Custom(UiMessage::Composer(Msg::Undo(true))))
                        }
                        (Some('v'), false) => {
                            return Some(text_editor::Binding::Custom(UiMessage::Composer(Msg::Paste)))
                        }
                        (Some('b'), false) => 0,
                        (Some('i'), false) => 1,
                        (Some('x'), true) => 2,
                        _ => return text_editor::Binding::from_key_press(k),
                    };
                    Some(text_editor::Binding::Custom(UiMessage::Composer(Msg::Format(i))))
                }
                _ => text_editor::Binding::from_key_press(k),
            })
            .highlight_with::<format::Marks>(marks, |f, _| *f)
            .size(13)
            .line_height(Pixels(16.0))
            .padding([10, 2])
            .height(Shrink)
            .max_height(self.pane().size.height / 2.0)
            .style(move |_, _| text_editor::Style {
                background: CLEAR.into(),
                border: iced::Border::default(),
                placeholder: p.subtle,
                value: p.strong,
                selection: p.accent.scale_alpha(0.45),
            });
        let empty = self.pane().composer.is_empty();
        // Desktop's `AccountAvatar` takes the place of the network icon when you have more accounts of the bridge.
        let switch =
            switch_targets(st, chat).filter(|_| self.pane().thread_root.is_none() && self.drawing.get().is_none());
        let net: Option<El<'a>> = match switch {
            Some(targets) => {
                let r = st.bridge_accounts.iter().find(|b| b.account_id == chat.account_id);
                let url = r.and_then(|b| b.remote_profile.as_ref()?.avatar.as_deref());
                let face = self.face(url, &account_user(st, &chat.account_id), 14.0, 0.0);
                Some(match targets.len() {
                    1 => {
                        let accounts: Vec<String> = st.bridge_accounts.iter().map(|b| b.account_id.clone()).collect();
                        let name = account_display_name(st, &chat.account_id, &accounts);
                        self.tip(face, name, vec![], tooltip::Position::Top).into()
                    }
                    _ => mouse_area(face).on_enter(UiMessage::SwitchCard).into(),
                })
            }
            None => (st.settings.on("showNetworkIcons") && empty && !several_accounts(st, &chat.account_id))
                .then(|| svg(brand_svg(brand_key(&chat.account_id))).width(14).height(14).into()),
        };
        let inline = Row::with_children([pri, sec].into_iter().flatten().map(&action_btn)).spacing(2).align_y(Center);
        let net = Row::new().push(net.map(|n| container(n).padding(Padding::ZERO.right(2.0)).center_y(36)));
        let inline = container(inline).padding(Padding::ZERO.left(2.0)).center_y(36);
        let input = self.with_formatting(input.into(), if st.settings.on("formattingMenu") { formats } else { 0 });
        let line = row![net, input, inline].align_y(Bottom).padding(Padding::new(0.0).left(14.0).right(6.5));
        let plus_icon = tinted(&COMPOSER_ICONS[usize::from(self.pane().show_attach_menu)], 20.0, p.muted);
        let toggle = button(container(plus_icon).center(36))
            .padding(0)
            .style(btn_style([p.input, p.hover, p.selected], [p.text; 3], 18.0))
            .on_press(UiMessage::Composer(Msg::ToggleAttachMenu));
        let unveiled = self.pane().show_attach_menu.then(|| {
            Row::with_children(hidden_actions(st).into_iter().map(&action_btn))
                .spacing(2)
                .padding([0, 1])
                .align_y(Center)
        });
        let plus = row![toggle].push(unveiled).spacing(6).align_y(Center);
        let ready = !self.pane().composer.text().trim().is_empty()
            || !self.pane().attachments.is_empty()
            || self.recorder.is_some();
        let pending = (!self.pane().attachments.is_empty()).then(|| {
            let items = self.pane().attachments.iter().enumerate().map(|(i, pm)| {
                let vcard = pm.name.ends_with(".vcf") || pm.mime == "text/x-vcard";
                let preview: El<'a> = if pm.frames.is_empty() && vcard {
                    contact_card(p, &parse_vcard(&String::from_utf8_lossy(&pm.bytes)))
                } else if pm.frames.is_empty() {
                    let ext = std::path::Path::new(&pm.name).extension().map(|e| e.to_string_lossy().to_uppercase());
                    let meta = ext.into_iter().chain([human_size(pm.bytes.len() as u64)]).collect::<Vec<_>>();
                    let name = clamp(&pm.name, 190.0, 2, 11.0, Weight::Semibold).line_height(Pixels(14.0));
                    let info = column![name, txt(meta.join(" ・ "), 11.0)].spacing(4);
                    let icon = &FILE_ICONS[if pm.mime.starts_with("audio") { 3 } else { 0 }];
                    let line = row![tinted(icon, 20.0, p.strong), info].spacing(8).align_y(Center);
                    container(column![line, Space::new().width(158)])
                        .max_width(240)
                        .padding([8, 11])
                        .style(fill_box(p.received, 17.0))
                        .into()
                } else {
                    let fg = Color { a: 0.1, ..p.text };
                    let fit = |f| image(f).height(66).content_fit(iced::ContentFit::Contain);
                    // Desktop's pending `BrandVideo`: the MP4 of a GIF loops with "Autoplay GIFs", and a video is a
                    // thumbnail with a play button.
                    let video = pm.mime.starts_with("video/");
                    let frames = anim::playing(&pm.frames, !video || st.settings.on("autoplayGifs"));
                    let img = container(anim::gif(frames, self.started, fit)).max_width(120).clip(true);
                    let img = img.style(bordered_box(CLEAR, fg, 1.0, 8.0));
                    let play = container(tinted(&voice::ICONS[0], 20.0, p.strong))
                        .center(36)
                        .style(fill_box(p.received, 18.0));
                    match video && !pm.is_gif {
                        true => stack![img, container(play).center(Fill)].into(),
                        false => img.into(),
                    }
                };
                removable(p, preview, UiMessage::Composer(Msg::DiscardAttachment(i)))
            });
            Row::with_children(items).spacing(8).align_y(iced::Bottom)
        });
        let send: El<'a> = match self.pane().editing_message_id {
            // Desktop's "Edit Message" button in place of the send button.
            Some(_) => {
                let edit = button(container(ic(icon::CHECK, 20.0).color(Color::WHITE)).center(36))
                    .padding(0)
                    .style(btn_style([p.accent; 3], [Color::WHITE; 3], 18.0))
                    .on_press(UiMessage::Composer(Msg::Submit));
                let label = tr("Edit Message");
                self.tip(edit.into(), label, vec!["⏎"], tooltip::Position::Top).into()
            }
            None => button(container(tinted(&SEND_ARROW, 20.0, Color::WHITE)).center(36))
                .padding(0)
                .style(btn_style([if ready { p.accent } else { p.rule }, p.accent, p.accent], [Color::WHITE; 3], 18.0))
                .on_press(UiMessage::Composer(Msg::Submit))
                .into(),
        };
        let text = self.pane().composer.text();
        let urls = text_urls(&text);
        let links =
            self.pane().composer_links.iter().filter(|(u, _)| urls.contains(&u.as_str())).filter_map(|(u, s)| {
                let card: El<'a> = match s {
                    LinkState::Discarded => return None,
                    LinkState::Loading => {
                        let bar = |w: f32| container(Space::new()).width(w).height(12).style(fill_box(p.hover, 12.0));
                        let bars = column![bar(111.0), bar(47.0)].spacing(8);
                        container(bars).width(180).padding([8, 11]).style(fill_box(p.input, 17.0)).into()
                    }
                    LinkState::Loaded(v, img) => {
                        let title = v["og:title"].as_str().expect("previews have a title");
                        let host = u.split("://").last().and_then(|u| u.split('/').next()).unwrap_or(u).to_string();
                        let img = img
                            .as_ref()
                            .map(|(_, h)| image(h.clone()).width(180).height(101).content_fit(iced::ContentFit::Cover));
                        let wide = if img.is_some() { 158.0 } else { 338.0 };
                        let link = row![ic(icon::LINK, 12.0).color(p.weak), txt(host, 11.0).color(p.weak)];
                        let text = column![
                            clamp(title, wide, 3, 13.0, Weight::Semibold).line_height(Pixels(16.0)).color(p.strong),
                            link.spacing(4).align_y(Center)
                        ];
                        let card = container(column![].push(img).push(text.spacing(8).padding([12, 11])));
                        card.width(if wide < 300.0 { Length::Fixed(180.0) } else { Shrink })
                            .clip(true)
                            .style(fill_box(p.link_card, 17.0))
                            .into()
                    }
                };
                Some(removable(p, card, UiMessage::Composer(Msg::RemoveLinkPreview(u.clone()))))
            });
        let links: Vec<El<'a>> = links.collect();
        let links = (!links.is_empty()).then(|| Column::with_children(links).spacing(4));
        // Desktop's "Also send to chat" of the thread composer of Slack and Beeper chats, once it has a draft.
        let pane = self.pane();
        let draft = !pane.composer.text().trim().is_empty() || !pane.attachments.is_empty();
        let shown = pane.thread_root.is_some() && pane.editing_message_id.is_none() && (draft || pane.broadcast);
        let broadcast = (shown && can_broadcast(st, chat)).then(|| -> El<'a> {
            let label = txt(tr("Also send to chat"), 13.0).color(p.sub);
            let line = row![checkbox(p, pane.broadcast), label].spacing(4).align_y(Center);
            let toggle = mouse_area(container(line).padding(Padding::ZERO.left(1.0).bottom(6.0)));
            toggle.on_press(UiMessage::Thread(threads::Msg::SetBroadcast(!pane.broadcast))).into()
        });
        // Desktop's `messageInputExternal`: the quote, the attachments and the link previews inside the pill.
        let external: Vec<El<'a>> =
            [broadcast, quote, pending.map(El::from), links.map(El::from)].into_iter().flatten().collect();
        let external = (!external.is_empty()).then(|| {
            let inner = Column::with_children(external).spacing(8);
            container(inner).padding(Padding { top: 8.0, right: 8.0, bottom: 0.0, left: 12.0 })
        });
        let pill: El<'a> = match self.pane().send_later {
            None => container(column![].push(external).push(line)).width(Fill).style(fill_box(p.input, 18.0)).into(),
            Some((at, no_reply)) => {
                let when = send_day(at, st.settings.pref("timeFormat") == 1);
                let when = if no_reply { trf("%s if no reply", &[&when]) } else { when };
                let details = row![
                    tinted(&COMPOSER_ICONS[6], 20.0, p.accent),
                    semi(when, 11.0).color(p.accent),
                    tinted(&CHEVRON_DOWN, 14.0, p.accent)
                ];
                let details = button(details.spacing(4).align_y(Center))
                    .bare(p.accent)
                    .on_press(UiMessage::Scheduled(scheduled::Msg::Open));
                let close =
                    icon_btn(p, icon::CLOSE, 14.0, 20.0, p.accent, UiMessage::Scheduled(scheduled::Msg::Set(None)));
                let blue = if p.card.r < 0.5 { hex(0x1b43aa) } else { hex(0xcbd9ff) };
                let banner = container(row![details, Space::new().width(Fill), close].spacing(2).align_y(Center))
                    .padding(2)
                    .style(fill_box(blue, 20.0));
                let banner = container(banner).padding(Padding::new(8.0).bottom(0.0).left(12.0));
                let body = column![banner].push(external).push(line);
                let body = container(body).width(Fill).style(fill_box(p.card, 18.0));
                stack![body, dashed::dashes(hex(0x808080), 18.0)].into()
            }
        };

        let stickers = (self.pane().picker == Some(Picker::Sticker)).then(|| self.view_sticker_picker(st));
        let extras = Column::new().push(gif_picker).push(stickers);
        let line: Row<'a, UiMessage> = match (&self.recorder, &self.talk) {
            (Some(r), _) => row![self.view_recorder(r), send],
            (None, Some(t)) => row![self.view_talk(t), send],
            (None, None) => row![plus, pill, send],
        };
        column![extras, line.spacing(8).align_y(Center).padding(8)].into()
    }
}

#[derive(Clone)]
pub enum Msg {
    Changed(String),
    Edit(text_editor::Action),
    /// Ctrl+Backspace and Ctrl+Delete: delete the selection, or the text from the caret to the motion.
    DeleteTo(text_editor::Motion, bool),
    /// Mod-z undoes the last composer edit group; Shift-Mod-z and Mod-y redo it.
    Undo(bool),
    /// Toggles a `format::LABELS` button on the composer selection.
    Format(usize),
    LinkDialog(Option<(String, String)>),
    SaveLink,
    Submit,
    SendAndArchive,
    CancelMode,
    StartReply(String),
    StartEdit(String, String),
    ToggleAttachMenu,
    TogglePicker(Picker),
    /// Desktop's `undoSend` in the open chat.
    UndoSend,
    /// Desktop's `sendQueuedMessages`: the sends of a chat, or the sends that are due.
    SendQueued(Option<String>),
    /// Chromium's composer context menu, on a misspelled word or not.
    EditMenu(Option<std::ops::Range<usize>>),
    MenuEdit(MenuEdit),
    /// Ctrl+V and "Paste": Desktop's `paste` handler reads the clipboard.
    Paste,
    Pasted(Result<Clip, String>),
    DiscardAttachment(usize),
    /// Desktop's `openAttachmentPicker`.
    PickFiles,
    FilesPicked(Result<Vec<std::path::PathBuf>, String>),
    /// The files to attach, or the toast text of each file that did not load.
    FilesLoaded(Vec<Result<PendingMedia, String>>),
    /// A file drag enters (true) or leaves a window.
    FileHover(window::Id, bool),
    FileDropped(window::Id, std::path::PathBuf),
    LinkPreviewLoaded(String, Result<Option<beeper_core::cloud::PagePreview>, String>),
    RemoveLinkPreview(String),
    /// Picks a row of the composer autocomplete.
    PickSuggestion(usize),
    /// The Cut (true) and Copy items of the Edit menu, on the composer.
    Clip(bool),
    /// Desktop's type-to-focus: the composer of the pane takes the text at its end.
    Type(String),
}

impl BeeperGuiApp {
    pub(crate) fn composer(&mut self, msg: Msg) -> Task<UiMessage> {
        match msg {
            Msg::EditMenu(word) => {
                let text = self.pane.composer.text();
                if let Some(r) = &word {
                    self.pane.composer = format::content(&text, r.clone());
                }
                let word = word.map(|r| (r.clone(), spell::suggest(&text[r])));
                self.pane.edit_menu = Some((self.cursor.get(), word));
            }
            Msg::MenuEdit(edit) => {
                let (_, word) = self.pane.edit_menu.take().expect("the edit menu is open");
                let old = self.pane.composer.text();
                let sel = format::selection(&self.pane.composer).map_or("", |r| &old[r]).to_owned();
                let focus = operation::focus("composer-input");
                return match edit {
                    MenuEdit::Replace(fix) => {
                        let (r, _) = word.expect("a suggestion has a word");
                        let text = format!("{}{fix}{}", &old[..r.start], &old[r.end..]);
                        self.pane.composer = format::content(&text, r.start + fix.len()..r.start + fix.len());
                        Task::batch([self.composer_changed(old), focus])
                    }
                    MenuEdit::Learn => match spell::learn(&old[word.expect("Learn Spelling has a word").0]) {
                        Ok(()) => focus,
                        Err(e) => {
                            self.status_banner = e.to_string();
                            focus
                        }
                    },
                    MenuEdit::Copy => Task::batch([iced::clipboard::write(sel), focus]),
                    MenuEdit::Cut => {
                        self.pane.composer.perform(text_editor::Action::Edit(text_editor::Edit::Delete));
                        Task::batch([iced::clipboard::write(sel), self.composer_changed(old), focus])
                    }
                    MenuEdit::Paste => Task::batch([self.update(UiMessage::Composer(Msg::Paste)), focus]),
                };
            }
            Msg::Edit(action) if !action.is_edit() => {
                self.pane.emoticon = None;
                self.pane.composer.perform(action);
            }
            Msg::Edit(action) => {
                use text_editor::{Action, Edit};
                let old = self.pane.composer.text();
                match (action, self.pane.emoticon.take().filter(|(after, ..)| *after == old)) {
                    (Action::Edit(Edit::Backspace), Some((_, typed, at))) => {
                        self.pane.composer = format::content(&typed, at..at);
                    }
                    (action, _) => {
                        let typed = matches!(action, Action::Edit(Edit::Insert(_)));
                        self.pane.composer.perform(action);
                        let text = self.pane.composer.text();
                        let at = format::offset(&text, self.pane.composer.cursor().position);
                        let rules = typed && !self.store.read().settings.on("keepEmoticons");
                        if let Some((new, caret)) = rules.then(|| emoji::replace_emoticon(&text, at)).flatten() {
                            self.pane.composer = format::content(&new, caret..caret);
                            self.pane.emoticon = Some((new, text, at));
                        }
                    }
                }
                return self.composer_changed(old);
            }
            Msg::DeleteTo(to, back) => {
                if self.pane.composer.selection().is_none() {
                    self.pane.composer.perform(text_editor::Action::Select(to));
                }
                let edit = if back { text_editor::Edit::Backspace } else { text_editor::Edit::Delete };
                return self.update(UiMessage::Composer(Msg::Edit(text_editor::Action::Edit(edit))));
            }
            Msg::Undo(false) if let Some(undo) = self.shown_undo() => {
                return self.update(UiMessage::Toast(toasts::Msg::Action(None, Box::new(undo))));
            }
            Msg::Undo(false) if self.queued_here() => return self.update(UiMessage::Composer(Msg::UndoSend)),
            Msg::Undo(true) if self.queued_here() => {
                return self.update(UiMessage::Composer(Msg::SendQueued(self.pane.selected_chat_id.clone())));
            }
            Msg::Undo(redo) => {
                let h = &mut self.pane.history;
                let (from, to) = if redo { (&mut h.redo, &mut h.undo) } else { (&mut h.undo, &mut h.redo) };
                let Some(text) = from.pop() else { return Task::none() };
                let now = self.pane.composer.text();
                let at = format::diff(&text, &now).1;
                to.push(now);
                h.last = None;
                self.pane.emoticon = None;
                self.pane.composer = format::content(&text, at..at);
                return self.composer_effects();
            }
            Msg::Changed(v) => {
                let old = self.pane.composer.text();
                self.pane.composer = composer_with(&v);
                return self.composer_changed(old);
            }
            Msg::Format(format::CODE) if format::selection(&self.pane.composer).is_none() => {
                let insert = text_editor::Action::Edit(text_editor::Edit::Insert('`'));
                return self.update(UiMessage::Composer(Msg::Edit(insert)));
            }
            Msg::Format(i) => {
                let old = self.pane.composer.text();
                let caret = format::offset(&old, self.pane.composer.cursor().position);
                let r = format::selection(&self.pane.composer).unwrap_or(caret..caret);
                if i == format::LINK && !format::active(i, &old, &r) {
                    self.pane.link_dialog = Some((String::new(), old[r].to_owned()));
                    return operation::focus("link-url");
                }
                let (text, r) = format::toggle(i, &old, r);
                self.pane.composer = format::content(&text, r);
                return Task::batch([self.composer_changed(old), operation::focus("composer-input")]);
            }
            Msg::LinkDialog(d) => {
                let closed = d.is_none();
                self.pane.link_dialog = d;
                if closed {
                    return operation::focus("composer-input");
                }
            }
            Msg::SaveLink => {
                let (url, label) = self.pane.link_dialog.take().expect("the link dialog is open");
                let old = self.pane.composer.text();
                let caret = format::offset(&old, self.pane.composer.cursor().position);
                let r = format::selection(&self.pane.composer).unwrap_or(caret..caret);
                let (text, r) = format::link(&old, r, url.trim(), label.trim());
                self.pane.composer = format::content(&text, r);
                return Task::batch([self.composer_changed(old), operation::focus("composer-input")]);
            }
            Msg::Submit if self.suggestions(&self.store.read()).is_some() => {
                return self.update(UiMessage::Composer(Msg::PickSuggestion(self.pane.suggest)));
            }
            Msg::Submit if self.recorder.is_some() => return self.send_take(),
            Msg::Submit if !self.can_compose() => {}
            Msg::Submit => {
                self.submit_composer(false);
                return self.typing(false);
            }
            Msg::SendAndArchive => {
                let cid = self.pane.selected_chat_id.clone().expect("a chat is selected");
                let st = self.store.read();
                let at = self.list_ids(&st).iter().position(|c| *c == cid);
                drop(st);
                let draft = !self.pane.composer.text().trim().is_empty() || !self.pane.attachments.is_empty();
                self.submit_composer(true);
                let stop = self.typing(false);
                let list = self.list_ids(&self.store.read());
                if let Some(at) = at.filter(|_| draft && !list.contains(&cid)) {
                    return stop.chain(self.after_action("afterArchive", &list, at, true));
                }
                return stop;
            }
            Msg::PickSuggestion(i) => {
                // The second click of a double click hits the list that the first pick closed.
                let Some((at, picks)) = self.suggestions(&self.store.read()) else { return Task::none() };
                let mut text = self.pane.composer.text();
                let caret = format::offset(&text, self.pane.composer.cursor().position);
                let pick = match &picks[i.min(picks.len() - 1)] {
                    Suggestion::Emoji(c, _) => format!("{c} "),
                    Suggestion::Mention(label, id, _) => {
                        self.pane.mentions.push((label.clone(), id.clone()));
                        format!("@{label} ")
                    }
                };
                text.replace_range(at..caret, &pick);
                self.pane.composer = format::content(&text, at + pick.len()..at + pick.len());
                return operation::focus("composer-input");
            }
            Msg::CancelMode if self.pane.editing_message_id.is_some() => self.pane.end_edit(),
            Msg::CancelMode => self.pane.reply_to_message_id = None,
            Msg::StartReply(mid) => {
                if self.pane.editing_message_id.is_some() {
                    self.pane.end_edit();
                }
                self.pane.reply_to_message_id = Some(mid);
                return operation::focus("composer-input");
            }
            Msg::StartEdit(mid, old_txt) => {
                // The mention pills of the message come back from its `matrix.to` links.
                let st = self.store.read();
                let html = st.loaded(&mid).and_then(|m| m.html.clone());
                drop(st);
                // Desktop scrolls the edited message into view (`scrollIntoView` of the editing target).
                self.pane.timeline.reveal(vlist::key(&mid));
                let shown = locate(format!("msg-{mid}"), self.pane.scroll_id(), false);
                self.pane.start_edit(mid, &old_txt, html.map_or(Vec::new(), |h| html_pills(&h)));
                return Task::batch([operation::focus("composer-input"), shown]);
            }
            Msg::Type(text) => {
                let open = self.can_compose();
                let quiet = self.confirm.is_none()
                    && !self.show_command_palette
                    && self.forward.is_none()
                    && self.creating_chat.is_none()
                    && self.carousel.is_none()
                    && self.recorder.is_none();
                if !(open && quiet) {
                    return Task::none();
                }
                let old = self.pane.composer.text();
                self.pane.composer.perform(text_editor::Action::Move(text_editor::Motion::DocumentEnd));
                for c in text.chars() {
                    self.pane.composer.perform(text_editor::Action::Edit(text_editor::Edit::Insert(c)));
                }
                return Task::batch([self.composer_changed(old), operation::focus("composer-input")]);
            }
            Msg::Paste => return Task::perform(clipboard_paste(), |r| UiMessage::Composer(Msg::Pasted(r))),
            // Desktop's `paste` handler: copied files go to the attachments, and a text goes into the composer.
            Msg::Pasted(r) => match r {
                Ok(Clip::Files(paths)) => return self.update(UiMessage::Composer(Msg::FilesPicked(Ok(paths)))),
                Ok(Clip::Image((png, size))) => self.attach_files(vec![Ok(PendingMedia {
                    name: "image.png".into(),
                    mime: "image/png".into(),
                    is_gif: false,
                    size,
                    frames: vec![(image::Handle::from_bytes(png.clone()), 0)],
                    bytes: png,
                    duration: None,
                    sticker: None,
                })]),
                Ok(Clip::Text) => {
                    let id = if self.pane.thread_root.is_some() { "thread-composer-input" } else { "composer-input" };
                    let paste = iced::clipboard::read().map(|t| match t {
                        Some(t) => UiMessage::Composer(Msg::Edit(text_editor::Action::Edit(text_editor::Edit::Paste(
                            t.into(),
                        )))),
                        None => UiMessage::Noop,
                    });
                    return Task::batch([paste, operation::focus(id)]);
                }
                Err(e) => self.status_banner = e,
            },
            Msg::DiscardAttachment(i) => drop(self.pane.attachments.remove(i)),
            Msg::PickFiles if self.pane.editing_message_id.is_some() => {
                self.status_banner = tr("Attachments can't be added while editing a message").into();
            }
            Msg::PickFiles => {
                let title = tr("Open").to_string();
                return Task::perform(portal::open_files(portal::PORTAL.into(), title, portal::Pick::Files), |a0| {
                    UiMessage::Composer(Msg::FilesPicked(a0))
                });
            }
            Msg::FilesPicked(r) => match r {
                Ok(paths) => {
                    let st = self.store.read();
                    let chat = &st.chats[st
                        .resolve_chat_idx(self.pane.selected_chat_id.as_deref().expect("an open chat"))
                        .expect("chat exists")];
                    let (rules, resizes) = (chat.file.clone(), chat.room_features);
                    drop(st);
                    // Desktop's `attachFiles` logs the error of a file and shows "Couldn't attach %s". A file that
                    // `TBe` takes goes through the codec check of `_Be` here, away from the window.
                    let load = move || {
                        let load = |p: &std::path::PathBuf| {
                            let name = p.file_name().map_or(p.display().to_string(), |n| n.to_string_lossy().into());
                            let f = portal::load_file(p).map_err(|e| {
                                eprintln!("attachFiles: failed to attach {name}: {e}");
                                format!("Couldn't attach {name}")
                            })?;
                            match portal::check(&f, &rules, resizes) {
                                Ok(()) => portal::decodable(&f, p, &rules).map(|()| f),
                                Err(_) => Ok(f),
                            }
                        };
                        paths.iter().map(load).collect()
                    };
                    let load = async { tokio::task::spawn_blocking(load).await.expect("file load thread") };
                    return Task::perform(load, |a0| UiMessage::Composer(Msg::FilesLoaded(a0)));
                }
                Err(e) => self.status_banner = e,
            },
            Msg::FilesLoaded(_) if self.pane.editing_message_id.is_some() => {
                self.status_banner = tr("Attachments can't be added while editing a message").into();
            }
            Msg::FilesLoaded(files) => {
                self.attach_files(files);
                if self.drop_send {
                    self.submit_composer(false);
                }
            }
            Msg::FileHover(id, on) if Some(id) == self.main_window => {
                self.dropping = on && self.pane.selected_chat_id.is_some();
                self.drop_send &= !on;
            }
            Msg::FileDropped(id, path) if Some(id) == self.main_window && self.pane.selected_chat_id.is_some() => {
                self.dropping = false;
                self.drop_send |= self.mods.shift();
                return self.update(UiMessage::Composer(Msg::FilesPicked(Ok(vec![path]))));
            }
            Msg::FileHover(..) | Msg::FileDropped(..) => {}
            Msg::LinkPreviewLoaded(url, r) => {
                let entry =
                    self.pane.composer_links.iter_mut().find(|(u, s)| *u == url && matches!(s, LinkState::Loading));
                if let Some((_, s)) = entry {
                    *s = match r {
                        Ok(Some((mut v, bytes))) => {
                            let image = bytes.and_then(|b| {
                                let format = ::image::guess_format(&b).ok()?;
                                let reader = ::image::ImageReader::with_format(std::io::Cursor::new(&b), format);
                                let (w, h) = reader.into_dimensions().ok()?;
                                v["og:image:width"] = w.into();
                                v["og:image:height"] = h.into();
                                v["og:image:type"] = format.to_mime_type().into();
                                Some((b.clone(), image::Handle::from_bytes(b)))
                            });
                            LinkState::Loaded(v, image)
                        }
                        Ok(None) => LinkState::Discarded,
                        Err(e) => {
                            eprintln!("link preview failed: {e}");
                            LinkState::Discarded
                        }
                    };
                }
            }
            Msg::RemoveLinkPreview(url) => {
                self.pane
                    .composer_links
                    .iter_mut()
                    .filter(|(u, _)| *u == url)
                    .for_each(|(_, s)| *s = LinkState::Discarded);
            }
            Msg::UndoSend => {
                let open = self.pane.selected_chat_id.as_ref();
                let Some(i) = self.queued.iter().position(|q| Some(&q.chat) == open) else { return Task::none() };
                let q = self.queued.remove(i);
                let mut st = self.store.write();
                for (id, _) in &q.sends {
                    st.remove_message(&q.chat, id).expect("a queued message is in its chat");
                }
                drop(st);
                self.pane.composer = composer_with(&q.text);
                (self.pane.reply_to_message_id, self.pane.attachments) = (q.reply_to, q.files);
                return Task::batch([self.composer_effects(), operation::focus("composer-input")]);
            }
            Msg::SendQueued(chat) => {
                let now = now_unix_millis();
                let (send, keep) = std::mem::take(&mut self.queued)
                    .into_iter()
                    .partition(|q| chat.as_ref().map_or(q.due <= now, |c| q.chat == *c));
                self.queued = keep;
                let mut st = self.store.write();
                for q in send.into_iter().flat_map(|q: Queued| q.sends.into_iter().map(move |s| (q.chat.clone(), s.1)))
                {
                    st.queue(&q.0, q.1);
                }
            }
            Msg::TogglePicker(kind) => {
                let open = self.pane.picker != Some(kind) && self.supports(kind);
                self.pane.picker = open.then_some(kind);
                if open && kind == Picker::Sticker {
                    return self.load_sticker_packs();
                }
                if open && kind == Picker::Gif {
                    self.pane.show_attach_menu = true;
                    let focus = operation::focus("gif-search-input");
                    if self.gifs.is_empty() && !self.gif_loading {
                        self.gif_loading = true;
                        self.gif_error = None;
                        return Task::batch([
                            focus,
                            self.update(UiMessage::Gifs(gifs::Msg::Search(self.gif_query.clone()))),
                        ]);
                    }
                    return focus;
                }
            }
            Msg::ToggleAttachMenu => {
                self.pane.show_attach_menu = !self.pane.show_attach_menu;
                if !self.pane.show_attach_menu {
                    self.pane.picker = None;
                }
            }
            Msg::Clip(cut) => {
                let Some(text) = self.pane.composer.selection() else { return Task::none() };
                let copy = iced::clipboard::write(text);
                if !cut {
                    return copy;
                }
                let delete =
                    self.update(UiMessage::Composer(Msg::Edit(text_editor::Action::Edit(text_editor::Edit::Delete))));
                return Task::batch([copy, delete]);
            }
        }
        self.settle()
    }
}
