//! The timeline of a chat: the rows of its messages, a message with its bubble, quote and reactions, the message
//! menu, and the dividers and the scroll button of the timeline.

use crate::blocking;
use crate::chat_list;
use crate::composer;
use crate::emoji::{with_emoji_font, EMOJI_FONT};
use crate::forward;
use crate::i18n::{tr, trf, trn, trnx, trx, trxf};
use crate::loads;
use crate::media_view;
use crate::members;
use crate::reminders::reminder_day;
use crate::style::{
    bold, bordered_box, bordered_btn, btn_style, clamp, fill_box, fw, hex, ic, icon, med, mix, pane_card, pick,
    scroll_style, semi, shadow_box, stable_bar, txt, Bare, CLEAR, GUTTER,
};
use crate::threads;
use crate::timeline;
use crate::windows;
use crate::{
    anim, badged, brightness, can_thread, chat_theme, clock, commands, copy_text, day_label, day_num, day_of,
    delete_dialog, downloads_dir, ease_out, edit_open, editable, email, enter_key, feedback_btn, file_icon, fmt_phone,
    format_short_ts, gallery, handle, has_older, human_size, lift, locale_string, locate, member_name, membership,
    menu_item, menu_rule, network_bot, pending, pill_btn, portal, profile_link, reply_root, safe_file_name, send_rise,
    sender_label, short_title, still_sending_at, stored, tinted, today, unavailable, unused_path, video, vlist,
    BeeperGuiApp, El, PillLook, UiMessage, ASK_DELETE_MESSAGE, BEEPER_ICON, BOOKMARK_ICON, BUBBLE_PAD_X, CHEVRON_DOWN,
    ERROR_OUTLINE, ISSUES_URL, MENU_ITEM, MESSAGE_MENU_W, ORANGE, TRASH, VIEW_SIDEBAR,
};
use beeper_core::config::{millis_from_rfc3339, now_unix_millis};
use beeper_core::models::*;
use beeper_core::prefs;
use beeper_core::store::BeeperStore;
use iced::font::Weight;
use iced::widget::operation;
use iced::widget::{
    button, column, container, image, mouse_area, pin, rich_text, row, scrollable, span, stack, svg, tooltip, Column,
    Row, Space, Stack,
};
use iced::{
    border, mouse, window, Bottom, Center, Color, Fill, Length, Padding, Pixels, Point, Right, Shadow, Size, Task,
    Theme, Vector,
};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

static REPLY_CURVE: std::sync::LazyLock<[svg::Handle; 2]> = std::sync::LazyLock::new(|| {
    ["", r#" transform="matrix(-1 0 0 1 26 0)""#].map(|flip| {
        svg::Handle::from_memory(format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0.5 0.5 25 19"><path{flip} fill="currentColor" fill-rule="evenodd" d="M17 15.5C9.544 15.5 3.5 9.456 3.5 2V.5h-3V2c0 9.113 7.387 16.5 16.5 16.5h6a1.5 1.5 0 0 0 0-3z"/></svg>"#).into_bytes())
    })
});

/// Whether Desktop's `bfe` shows an attachment of a quote as a picture: an image, or a video.
fn quote_tile(a: &Attachment) -> bool {
    matches!(a.attachment_type.as_str(), "img" | "video")
        || a.mime_type.as_deref().is_some_and(|m| m.starts_with("image/"))
}

/// The name of a contact card in a quote: the file name without a `1.name.` prefix and without `.vcf`, like Desktop's
/// `replace(/^\d\.\w+\./, "").replace(".vcf", "")`.
fn contact_name(file: &str) -> String {
    let mut parts = file.splitn(3, '.');
    let prefixed = match (parts.next(), parts.next(), parts.next()) {
        (Some(d), Some(w), Some(rest))
            if d.len() == 1
                && d.as_bytes()[0].is_ascii_digit()
                && !w.is_empty()
                && w.chars().all(|c| c.is_alphanumeric() || c == '_') =>
        {
            Some(rest)
        }
        _ => None,
    };
    prefixed.unwrap_or(file).replacen(".vcf", "", 1)
}

/// A surface of Desktop's header card: its fill, its radius, an outline, and a black shadow `(y, blur, alpha byte)`.
fn raised(
    bg: Color,
    radius: f32,
    outline: Option<Color>,
    (y, blur, a): (f32, f32, u8),
) -> impl Fn(&Theme) -> container::Style + Copy {
    move |_| container::Style {
        background: Some(bg.into()),
        border: iced::Border {
            color: outline.unwrap_or(CLEAR),
            width: if outline.is_some() { 1.0 } else { 0.0 },
            radius: radius.into(),
        },
        shadow: iced::Shadow {
            color: Color::from_rgba8(0, 0, 0, f32::from(a) / 255.0),
            offset: iced::Vector::new(0.0, y),
            blur_radius: blur,
        },
        ..Default::default()
    }
}

/// Desktop's file type label: the extension in capitals, or a name for some MIME types.
fn file_label(a: &Attachment) -> Option<String> {
    let ext = a.file_name.as_deref().and_then(|n| n.rsplit_once('.')).filter(|(stem, e)| {
        !stem.is_empty() && (1..=10).contains(&e.len()) && e.bytes().all(|b| b.is_ascii_alphanumeric())
    });
    let named = || match a.mime_type.as_deref()? {
        "application/pdf" => Some("PDF"),
        "application/zip" | "application/x-zip-compressed" | "multipart/x-zip" => Some("ZIP"),
        "text/csv" => Some("CSV"),
        "text/plain" => Some("TXT"),
        "video/quicktime" => Some("MOV"),
        "video/x-msvideo" => Some("AVI"),
        _ => None,
    };
    ext.map(|(_, e)| e.to_ascii_uppercase()).or_else(|| named().map(String::from))
}

/// Desktop's click on a reaction of a message or a gallery: it removes your reaction from all the messages that have
/// it, or adds it to all the messages that have the reaction.
fn toggle_reaction(st: &BeeperStore, chat: &str, ms: &[&Message], key: &str) -> UiMessage {
    let has = |m: &&&Message, mine: bool| {
        m.reactions.iter().any(|r| r.reaction_key == key && (!mine || st.is_me(&r.participant_id)))
    };
    let mine = ms.iter().any(|m| has(&m, true));
    let mut toggles = ms
        .iter()
        .filter(|m| has(m, mine))
        .map(|m| UiMessage::Message(Msg::ToggleReaction(chat.into(), m.id.clone(), key.into())));
    match ms.len() {
        1 => toggles.next().expect("the message has the reaction"),
        _ => UiMessage::Commands(toggles.collect()),
    }
}

/// The reaction keys of messages `ms`, sorted and each once.
fn reaction_keys<'a>(ms: &[&'a Message]) -> Vec<&'a str> {
    let mut keys: Vec<&str> = ms.iter().flat_map(|m| &m.reactions).map(|r| r.reaction_key.as_str()).collect();
    keys.sort_unstable();
    keys.dedup();
    keys
}

fn gap_below(m: &Message, next: &Message) -> f32 {
    let ms = |m: &Message| millis_from_rfc3339(&m.timestamp).expect("message timestamp");
    match (ms(next) - ms(m)).abs() / 60_000 {
        _ if day_num(&m.timestamp) != day_num(&next.timestamp) => 0.0,
        241.. => 26.0,
        6.. => 13.0,
        2.. => 4.0,
        _ if m.sender_id == next.sender_id => 2.0,
        _ => 4.0,
    }
}

/// Desktop's `--color-on-error-container`, the same in the light and dark themes.
const ON_ERROR_CONTAINER: Color = Color::from_rgb8(0xdc, 0x36, 0x2e);

/// Desktop's `kf`: the box of an inline image or video, at most `share` of the window wide and 400 px high and never
/// upscaled. A very tall one shows 160×400 and a very wide one an 80 px high strip, both cropped (`cover`), and one of
/// unknown size is 400 px high. Gives the width, the height and whether it crops.
pub(crate) fn media_box(window_w: f32, size: Option<(f32, f32)>, share: f32) -> (f32, f32, bool) {
    const TALL_W: f32 = 160.0;
    const HIGH: f32 = 400.0;
    const STRIP: f32 = 80.0;
    let Some((w, h)) = size.filter(|s| s.0 > 0.0 && s.1 > 0.0) else { return (share * window_w, HIGH, true) };
    let (o, r) = ((share * window_w).min(w), w / h);
    let fitted = match w / o > h / HIGH {
        true => (o, (o / r).min(HIGH), false),
        false => ((HIGH * r).min(o), HIGH, false),
    };
    match r {
        _ if r > o / STRIP => (o, STRIP, true),
        _ if r > TALL_W / HIGH => fitted,
        _ => (TALL_W, HIGH, true),
    }
}

/// Desktop's `--cv-avatar-size` and the margin of `.sender-img-container`: the sender avatar of a group message.
const FACE: f32 = 34.0;
const FACE_GAP: f32 = 8.0;

/// Desktop's `.message-text` line clamp.
const MESSAGE_LINES: usize = 40;

/// The clamp of a message over [`MESSAGE_LINES`] lines at `max` wide: the blocks that show whole, and the lines left for
/// the next one. `None` when the message fits.
fn clamp_blocks<'s>(blocks: impl Iterator<Item = &'s str>, max: f32) -> Option<(usize, usize)> {
    let mut used = 0;
    for (i, b) in blocks.enumerate() {
        let n = crate::style::line_count(b, max, 13.0).max(1);
        if used + n > MESSAGE_LINES {
            return Some((i, MESSAGE_LINES - used));
        }
        used += n;
    }
    None
}

/// The first `lines` lines of `t` at `max` wide, keeping its line breaks, with "…" where a line is cut.
fn cut_lines(t: &str, max: f32, lines: usize) -> String {
    let (mut out, mut used) = (Vec::new(), 0);
    for line in t.split('\n') {
        let n = crate::style::line_count(line, max, 13.0).max(1);
        if used + n > lines {
            if lines > used {
                out.push(crate::style::clamped(line, max, lines - used, 13.0, Weight::Normal).to_string());
            }
            break;
        }
        used += n;
        out.push(line.to_string());
    }
    out.join("\n")
}

/// Desktop's `rWe`: a message hides its time when the next one comes from the same sender in the same minute, and is
/// not hidden or a jumbo emoji.
pub(crate) fn time_hidden(m: &Message, next: Option<&Message>, h24: bool) -> bool {
    next.is_some_and(|n| {
        n.sender_id == m.sender_id
            && n.message_type.as_deref() != Some("HIDDEN")
            && !jumbo(n.text.as_deref().unwrap_or_default())
            && format_short_ts(&n.timestamp, h24) == format_short_ts(&m.timestamp, h24)
    })
}

fn jumbo(text: &str) -> bool {
    let modifier = |c: char| matches!(c, '\u{200d}' | '\u{fe0f}' | '\u{20e3}' | '\u{1f3fb}'..='\u{1f3ff}' | '\u{e0020}'..='\u{e007f}');
    let emoji = |c: char| {
        matches!(c, '\u{1f000}'..='\u{1faff}' | '\u{2600}'..='\u{27bf}' | '\u{2300}'..='\u{23ff}' | '\u{2b00}'..='\u{2bff}'
            | '\u{2190}'..='\u{21ff}' | '\u{3030}' | '\u{303d}' | '\u{3297}' | '\u{3299}' | '\u{a9}' | '\u{ae}' | '\u{2122}')
    };
    let mut glyphs = 0;
    let mut joined = false;
    for c in text.chars().filter(|c| !c.is_whitespace()) {
        match (modifier(c), emoji(c)) {
            (true, _) => joined = c == '\u{200d}',
            (false, true) => {
                glyphs += usize::from(!joined);
                joined = false;
            }
            (false, false) => return false,
        }
    }
    (1..10).contains(&glyphs)
}

/// The values of the timeline rows that depend on all messages of the list.
pub(crate) struct Rows {
    /// The messages, as indices of `messages`.
    pub(crate) msgs: Vec<usize>,
    /// The first message of the membership event run of each message.
    run_start: Vec<usize>,
    last_live: Option<usize>,
    any_pending: bool,
    /// For each person other than you and the sender, the newest message that the person saw before a reply of the
    /// person, and the time.
    last_seen: HashMap<String, (usize, String)>,
    /// The first unread message.
    unread: Option<usize>,
    /// Whether each message is in the reply chain of the message before it.
    chained: Vec<bool>,
    /// Desktop's `replyCounts`: the messages of the same reply chain after each message.
    counts: Vec<usize>,
    /// The first message of the gallery of each message.
    gallery: Vec<usize>,
    /// The virtual list key of each message.
    keys: Vec<u64>,
    /// The messages that start a day, which show its date.
    days: Vec<usize>,
}

impl Rows {
    fn of(st: &BeeperStore, msgs: &[&Message]) -> Self {
        let n = msgs.len();
        let mut run_start: Vec<usize> = (0..n).collect();
        for i in 1..n {
            let (a, b) = (msgs[i - 1], msgs[i]);
            if membership(a) && membership(b) && day_num(&a.timestamp) == day_num(&b.timestamp) {
                run_start[i] = run_start[i - 1];
            }
        }
        let mut last_seen = HashMap::new();
        let mut replied: HashSet<&str> = HashSet::new();
        for (k, m) in msgs.iter().enumerate().rev() {
            for (uid, ts) in &m.seen {
                if !st.is_me(uid) && uid != &m.sender_id && !replied.contains(uid.as_str()) {
                    last_seen.entry(uid.clone()).or_insert((k, ts.clone()));
                }
            }
            if m.is_deleted != Some(true) && m.is_sender != Some(true) {
                replied.insert(m.sender_id.as_str());
            }
        }
        let roots: Vec<&str> = msgs.iter().map(|m| reply_root(st, m)).collect();
        let mut later: HashMap<&str, usize> = HashMap::new();
        let mut counts = vec![0; n];
        for i in (0..n).rev() {
            let k = later.entry(roots[i]).or_insert(0);
            (counts[i], *k) = (*k, *k + 1);
        }
        Self {
            msgs: msgs.iter().map(|m| st.message_idx(&m.id)).collect(),
            run_start,
            last_live: msgs.iter().rposition(|m| m.is_deleted != Some(true)),
            any_pending: msgs.iter().any(|m| pending(st, m)),
            last_seen,
            unread: msgs.iter().position(|m| m.is_deleted != Some(true) && m.is_unread == Some(true)),
            chained: (0..n).map(|i| i > 0 && roots[i - 1] == roots[i]).collect(),
            gallery: gallery::starts(st, msgs, &counts),
            counts,
            keys: msgs.iter().map(|m| vlist::key(&m.id)).collect(),
            days: (0..n)
                .filter(|&i| i == 0 || day_num(&msgs[i - 1].timestamp) != day_num(&msgs[i].timestamp))
                .collect(),
        }
    }
}

/// Desktop's `TopOfThread`, which loads older messages while the top of the timeline is on screen.
#[derive(Default)]
pub struct TopOfThread {
    pub(crate) shown: bool,
    /// The start of the running load, its chat, and the message count of the chat before it.
    pub(crate) loading: Option<(std::time::Instant, String, usize)>,
    /// A retry waits for its backoff.
    pub(crate) waiting: bool,
    /// The loads since the message count changed.
    pub(crate) tries: u32,
}

impl BeeperGuiApp {
    /// Desktop's `P1t`: loads each quoted message that the timeline does not have, once.
    pub(crate) fn load_quotes(&mut self) -> Task<UiMessage> {
        let wanted = std::mem::take(&mut *self.wants.quotes.borrow_mut());
        let new: Vec<_> = wanted.into_iter().filter(|(_, id)| !self.quotes.contains_key(id)).collect();
        self.quotes.extend(new.iter().map(|(_, id)| (id.clone(), None)));
        Task::batch(new.into_iter().map(|(chat, id)| {
            let fetch = beeper_core::matrix::fetch_message(self.store.clone(), chat, id.clone());
            Task::perform(fetch, move |r| UiMessage::Loaded(loads::Msg::Quote(id.clone(), r.map(Box::new))))
        }))
    }

    fn typing_bubble<'a>(&self, chat: &Chat, ids: &[String]) -> El<'a> {
        let p = &self.pal;
        let dot = |delay| anim::dot(6.0, p.text, self.started, delay, |t| 0.2 + 0.2 * (1.0 - (2.0 * t - 1.0).abs()));
        let bubble =
            container(row![dot(0), dot(340), dot(670)].spacing(4)).padding([12, 11]).style(fill_box(p.received, 16.0));
        let faces = (chat.chat_type == "group").then(|| {
            let users = ids.iter().filter_map(|id| chat.participants.items.iter().find(|u| u.id == *id));
            let layers = users.enumerate().map(|(i, u)| {
                pin(self.face(u.img_url.as_deref(), u.full_name.as_deref().unwrap_or_default(), 20.0, 0.0))
                    .x(i as f32 * 10.0)
                    .into()
            });
            container(Stack::with_children(layers)).center_x(34)
        });
        container(row![].push(faces).push(bubble).spacing(8).align_y(Center))
            .padding(Padding::ZERO.left(8.0).top(8.0))
            .into()
    }

    pub(crate) fn view_message_menu(&self, mid: &str) -> (El<'_>, f32, f32) {
        if let Some(menu) = self.view_gallery_menu(mid) {
            return menu;
        }
        let p = &self.pal;
        let st = self.store.read();
        let m = st.message(mid);
        let (cid, mid, mine) = (m.chat_id.clone(), m.id.clone(), m.is_sender == Some(true));
        let text = m.text.clone().unwrap_or_default();
        let link = m.links.first().map(|l| l.url.clone());
        let address = text.split_whitespace().find_map(email).map(str::to_string);
        let entire = link.is_some();
        let rule = || (menu_rule(p), 11.0);
        let item = |label: String, hotkey, msg: Option<UiMessage>| {
            (menu_item(p, None, label, hotkey, msg.map(|m| UiMessage::Message(Msg::Menu(Box::new(m))))), MENU_ITEM)
        };
        let bookmark = tr(if m.is_bookmarked == Some(true) { "Remove Bookmark" } else { "Bookmark" });
        let at = trf(if mine { "Sent at %s" } else { "Received at %s" }, &[&locale_string(&m.timestamp)]);
        let stamp = st.settings.on("copyTimestamps");
        let files = m.attachments.len();
        let image = m.attachments.first().is_some_and(|a| a.attachment_type == "img");
        let only_image = image && m.text.is_none();
        let download = match m.attachments.first().and_then(|a| a.file_size).filter(|_| files == 1) {
            Some(n) => trf(if image { "Download Image (%s)" } else { "Download Attachment (%s)" }, &[&human_size(n)]),
            None => tr(match (image, files == 1) {
                (true, true) => "Download Image",
                (true, false) => "Download Images",
                (false, true) => "Download Attachment",
                (false, false) => "Download Attachments",
            })
            .into(),
        };
        let chat = st.chat(&cid);
        let (rules, live) = (chat.messages, m.is_deleted != Some(true));
        let others = chat.participants.items.iter().filter(|u| u.is_self != Some(true)).count();
        let sender = chat.participants.items.iter().any(|u| u.id == m.sender_id);
        let privately =
            !mine && sender && others > 1 && m.is_deleted != Some(true) && self.can(&m.account_id, "create_dm");
        let head: Vec<(El, f32)> = [
            self.video_menu_rows(&mid),
            address.map(|e| {
                vec![
                    item(
                        trxf("Copy %s", "message menu action to copy an email address", &[&e]),
                        "",
                        Some(UiMessage::Message(Msg::CopyText(e.clone()))),
                    ),
                    item(trf("Email %s…", &[&e]), "", Some(UiMessage::OpenUrl(format!("mailto:{e}")))),
                    rule(),
                ]
            }),
            link.map(|u| vec![item(tr("Copy Link").into(), "", Some(UiMessage::Message(Msg::CopyText(u)))), rule()]),
            m.unresolved_media.as_ref().map(|raw| {
                let state = self.resolving_media.get(&mid);
                let label = match state {
                    Some(false) => trx("Try Loading Media Again", "message menu action"),
                    _ => trx("Load Media", "message menu action"),
                };
                let load = UiMessage::Loaded(loads::Msg::ResolveMedia(m.account_id.clone(), mid.clone(), raw.clone()));
                vec![item(label.into(), "", (state != Some(&true)).then_some(load)), rule()]
            }),
            (editable(m, rules) && !text.is_empty()).then(|| {
                let edit =
                    edit_open(m, rules).then(|| UiMessage::Composer(composer::Msg::StartEdit(mid.clone(), text)));
                vec![item(tr("Edit…").into(), "Ctrl T", edit)]
            }),
            (rules.reply && live).then(|| {
                vec![item(
                    tr("Reply…").into(),
                    "Ctrl R",
                    Some(UiMessage::Composer(composer::Msg::StartReply(mid.clone()))),
                )]
            }),
            (self.pane().thread_root.is_none() && can_thread(&st, chat, m)).then(|| {
                let label = tr("Reply in thread…").into();
                vec![item(label, "Ctrl ↓", Some(UiMessage::Thread(threads::Msg::Show(mid.clone()))))]
            }),
            (self.pane().thread_root.is_none()
                && live
                && st.threaded(chat)
                && st.thread_replies(&mid).next().is_some())
            .then(|| vec![(self.follow_item(&st, chat, &mid), MENU_ITEM)]),
            privately.then(|| {
                vec![item(
                    tr("Reply Privately…").into(),
                    "",
                    Some(UiMessage::ChatList(chat_list::Msg::ReplyPrivately(mid.clone()))),
                )]
            }),
            Some(vec![
                item(
                    tr("React…").into(),
                    "",
                    (rules.react && m.reactable()).then(|| UiMessage::Message(Msg::React(mid.clone()))),
                ),
                item(bookmark.into(), "", Some(UiMessage::Message(Msg::ToggleBookmark(mid.clone())))),
            ]),
            (m.is_unread == Some(true)).then(|| {
                let read = UiMessage::Message(Msg::MarkReadThrough(cid.clone(), mid.clone()));
                vec![item(tr("Mark as Read").into(), "", Some(read))]
            }),
            (m.is_unread != Some(true) && m.account_id != "hungryserv").then(|| {
                vec![item(
                    tr("Mark as Unread").into(),
                    "",
                    Some(UiMessage::Message(Msg::MarkUnreadFromHere(cid.clone(), mid.clone()))),
                )]
            }),
            (mine
                && m.poll.as_ref().is_some_and(|p| !p.ended)
                && m.is_deleted != Some(true)
                && st.feature("enable-polls").on
                && st.chat(&cid).polls.is_some_and(|r| r.end))
            .then(|| {
                vec![item("End Poll".into(), "", Some(UiMessage::Message(Msg::EndPoll(cid.clone(), mid.clone()))))]
            }),
            self.pane().reply_thread.is_some().then(|| {
                let jump = UiMessage::Commands(vec![
                    UiMessage::CommandBar(commands::Msg::JumpTo(cid, mid.clone())),
                    UiMessage::Message(Msg::Select(vec![mid.clone()])),
                ]);
                vec![item(tr("Jump to Message").into(), "", Some(jump))]
            }),
        ]
        .into_iter()
        .flatten()
        .flatten()
        .collect();
        let top = head.iter().map(|r| r.1).sum::<f32>() + 11.0;
        // Desktop's `Zit`: no Delete… for the message that you edit.
        let editing = self.pane().editing_message_id.as_ref() == Some(&mid);
        let delete = (!editing && delete_dialog(&st, &[m]).is_some())
            .then(|| item(tr("Delete…").into(), "", Some(UiMessage::Message(Msg::AskDelete(vec![mid.clone()])))));
        let tail: Vec<(El, f32)> = self.voice_menu_rows(&st, m).into_iter().chain(delete).collect();
        let rows: Vec<(El, f32)> = [
            Some(head),
            (!tail.is_empty()).then(|| std::iter::once(rule()).chain(tail).collect()),
            Some(vec![
                rule(),
                item(
                    tr(if entire { "Copy Entire Message" } else { "Copy" }).into(),
                    if stamp { "" } else { "Ctrl C" },
                    Some(match only_image {
                        true => UiMessage::Message(Msg::CopyImage(mid.clone(), 0)),
                        false => UiMessage::Message(Msg::CopyText(copy_text(m, false, false))),
                    }),
                ),
            ]),
            (!image).then(|| {
                let copy = UiMessage::Message(Msg::CopyText(copy_text(m, true, false)));
                let label = tr(if entire { "Copy Entire Message with Timestamp" } else { "Copy with Timestamp" });
                vec![item(label.into(), if stamp { "Ctrl C" } else { "" }, Some(copy))]
            }),
            (image && !only_image).then(|| {
                vec![item(tr("Copy Image").into(), "", Some(UiMessage::Message(Msg::CopyImage(mid.clone(), 0))))]
            }),
            (m.is_deleted != Some(true)).then(|| {
                vec![item(
                    tr("Forward…").into(),
                    "Ctrl Shift F",
                    Some(UiMessage::Forward(forward::Msg::Open(vec![mid.clone()]))),
                )]
            }),
            (files > 0)
                .then(|| vec![item(download, "Ctrl D", Some(UiMessage::Message(Msg::Download(mid.clone(), None))))]),
            Some(vec![
                item(at, "", None),
                rule(),
                item(tr("Report a Problem").into(), "Ctrl Shift P", Some(UiMessage::OpenUrl(ISSUES_URL.into()))),
            ]),
        ]
        .into_iter()
        .flatten()
        .flatten()
        .collect();
        let h = 8.0 + rows.iter().map(|r| r.1).sum::<f32>();
        let col = Column::with_children(rows.into_iter().map(|r| r.0));
        (
            container(col.padding([4, 0]))
                .width(MESSAGE_MENU_W)
                .style(shadow_box(p.menu, p.menu_border, 6.0, 16.0))
                .into(),
            h,
            top,
        )
    }

    /// Desktop's `F2t` at the top of a request: the chat, the username of the other person, the description and
    /// View Profile.
    fn view_request_head<'a>(&self, st: &BeeperStore, chat: &Chat) -> Option<El<'a>> {
        if chat.is_request != Some(true) {
            return None;
        }
        let p = &self.pal;
        let other =
            chat.participants.items.iter().find(|u| u.is_self != Some(true)).filter(|_| chat.chat_type != "group");
        let username = other
            .and_then(|u| handle(u).or_else(|| u.phone_number.as_deref().map(fmt_phone)).or_else(|| u.email.clone()));
        let mono = (!st.settings.on("colorizeNetworkIcons")).then_some(p);
        let avatar = badged(self.chat_avatar(chat, 46.0, 0.0), &chat.account_id, 46.0, 16.0, mono, p.card);
        let title = semi(chat.title.clone(), 15.0).line_height(Pixels(20.0)).color(p.strong);
        let username = username.map(|u| txt(u, 12.0).color(p.sub));
        let about = chat.description.clone().filter(|d| !d.is_empty());
        let about = about.map(|d| txt(d, 13.0).color(p.sub).align_x(Center));
        let profile = profile_link(chat).map(|url| {
            button(semi(tr("View Profile"), 13.0).color(p.accent)).bare(p.accent).on_press(UiMessage::OpenUrl(url))
        });
        let col = column![avatar, column![title].push(username).align_x(Center)].push(about).push(profile);
        let col = col.spacing(10).align_x(Center).max_width(480).padding(Padding::new(24.0).bottom(32.0));
        Some(container(col).center_x(Fill).into())
    }

    /// Desktop's `InviteDetailsPane` (`P$t`), which shows in place of the chat for an invite to a Matrix room: the
    /// room, who invited the user, the topic, and Decline and Accept.
    fn view_invite_details<'a>(&self, chat: &Chat) -> El<'a> {
        let p = &self.pal;
        let inviter = chat.inviter().map(|u| {
            let name = semi(u.full_name.clone().unwrap_or_default(), 14.0).line_height(Pixels(18.0)).color(p.strong);
            self.user_trigger(chat, &u.id, column![name, txt(format!("({})", u.id), 13.0).color(p.sub)].align_x(Center))
        });
        let group = chat.chat_type == "group";
        // Desktop does not translate these.
        let text = if group { "invited you to the group:" } else { "invited you to chat" };
        let text = txt(text, 13.0).color(p.text);
        let title = group.then(|| semi(chat.title.clone(), 15.0).line_height(Pixels(20.0)).color(p.strong));
        let title = title.map(|t| container(t.align_x(Center)).padding(Padding::ZERO.top(10.0)));
        let about = chat.description.clone().filter(|d| !d.is_empty());
        let about = about.map(|d| container(txt(d, 13.0).color(p.text).align_x(Center)).padding([13, 0]));
        let (accept, decline) = crate::request_answers(chat);
        let pill = |label: &'static str, fg: Color, msg| {
            let label = semi(label, 12.0);
            button(label).padding([4, 8]).style(bordered_btn([p.card; 3], [fg; 3], p.chip, 1.0, 100.0)).on_press(msg)
        };
        let id = &chat.id;
        let actions = row![]
            .push(decline.then(|| {
                pill("Decline", hex(0xdc362e), UiMessage::ChatList(chat_list::Msg::Delete(id.clone(), false)))
            }))
            .push(
                accept
                    .then(|| pill("Accept", p.strong, UiMessage::ChatList(chat_list::Msg::AcceptRequest(id.clone())))),
            )
            .spacing(8);
        let avatar = container(self.chat_avatar(chat, 40.0, 0.0)).padding(Padding::ZERO.top(14.0).bottom(8.0));
        let content = column![avatar].push(column![].push(inviter).push(text).align_x(Center)).push(title).push(about);
        let content = content.push(container(actions).padding(Padding::ZERO.top(12.0))).align_x(Center).max_width(480);
        container(content).center(Fill).style(pane_card(p.card)).into()
    }

    /// Desktop's `IncognitoMarkAsRead`: a link below an unread chat that incognito mode keeps unread.
    fn incognito_mark_read<'a>(&self, st: &BeeperStore, chat: &Chat) -> Option<El<'a>> {
        let unread = chat.is_unread();
        (unread && st.incognito_indicator()).then(|| {
            let p = &self.pal;
            let link = button(semi(tr("Mark as Read"), 11.0).color(p.accent))
                .bare(p.accent)
                .on_press(UiMessage::ChatList(chat_list::Msg::MarkRead(chat.id.clone())));
            let tip = self.tip(link.into(), tr("Mark as Read"), vec!["Ctrl", "Shift", "U"], tooltip::Position::Top);
            container(tip).center_x(Fill).padding(Padding::new(10.0).bottom(6.5).left(0.0).right(0.0)).into()
        })
    }

    pub(crate) fn view_conversation(&self) -> El<'_> {
        let p = &self.pal;
        let st = self.store.read();
        let cid = self.pane().selected_chat_id.as_deref().expect("a chat is selected");
        let chat = &st.chats[st.resolve_chat_idx(cid).expect("selected chat exists")];
        // Desktop's `isMatrixRequest`.
        if chat.is_request == Some(true) && chat.account_id == "hungryserv" {
            return self.view_invite_details(chat);
        }
        let rows = self.conversation_rows(&st, chat);
        let msgs: Vec<&Message> = rows.msgs.iter().map(|&i| &st.messages[i]).collect();
        // Desktop's `ThreadedRepliesView` hides the typing indicator, and its list has no scheduled messages.
        let in_chat = self.pane().thread_root.is_none();
        let col = self.view_messages(&st, chat, &msgs, &rows, false);
        let col = col.push(st.typing.get(cid).filter(|_| in_chat).map(|ids| self.typing_bubble(chat, ids)));
        let col = col.push(self.incognito_mark_read(&st, chat));
        let col = col.push(in_chat.then(|| self.view_scheduled(&st, chat)));
        let col = col.push(in_chat.then(|| self.view_request(&st, chat)));
        let col = col.push(
            unavailable(&st, chat)
                .map(|t| container(txt(t, 13.0).color(p.sub).align_x(Center)).center_x(Fill).center_y(52)),
        );
        // Desktop's `messages-list` keeps the messages at the bottom when they do not fill the view.
        let scroller = |col| {
            let col = container(col).align_y(Bottom);
            scrollable(col).direction(stable_bar()).anchor_bottom().height(Fill).style(scroll_style(p.scroll))
        };
        // Desktop's `processScroll`: the timeline is pinned to its bottom until you scroll up. A sensor at the bottom
        // tells it without a message for each scroll step, which would build the view again.
        let bottom = iced::widget::sensor(Space::new()).key(chat.id.clone()).anticipate(1);
        let col = col.push(
            bottom
                .on_show(|_| UiMessage::Timeline(timeline::Msg::Scrolled(false)))
                .on_hide(UiMessage::Timeline(timeline::Msg::Scrolled(true))),
        );
        let list = lift::tight(scroller(col).id(self.pane().scroll_id()));
        let replies = self.pane().reply_thread.as_ref().map(|id| {
            let root = st.loaded_idx(id).map_or(id.as_str(), |i| reply_root(&st, &st.messages[i]));
            let chain: Vec<&Message> = msgs.iter().copied().filter(|m| reply_root(&st, m) == root).collect();
            let list = lift::tight(scroller(self.view_messages(&st, chat, &chain, &Rows::of(&st, &chain), true)));
            // Desktop blurs the chat 8px under a 0.6 background; iced has no backdrop blur.
            mouse_area(container(list).width(Fill).height(Fill).style(fill_box(p.card.scale_alpha(0.85), 0.0)))
                .on_press(UiMessage::Message(Msg::HideReplies))
        });
        let close = replies.is_some().then(|| -> El<'_> {
            let close = button(ic(icon::CLOSE, 20.0).color(p.weak))
                .padding(10)
                .style(|_, _| button::Style::default())
                .on_press(UiMessage::Message(Msg::HideReplies));
            container(close).align_right(Fill).padding(16).into()
        });
        // Desktop's `thread-header-card`: the avatar on its disc over the name pill, 4 px into it. The card shows no
        // hover; the pointer on it shows the members (`kmt`) instead.
        let name_row = row![
            semi(short_title(chat).to_string(), 13.0).line_height(Pixels(16.0)).color(p.fg),
            ic(icon::CHEVRON_RIGHT, 12.0).color(p.tertiary)
        ]
        .spacing(2)
        .align_y(Center);
        // Desktop's `thread-header-status`: the activity of the other person of a 1:1 chat under the name.
        let typing = st.typing.get(cid).filter(|ids| chat.chat_type == "single" && ids.len() == 1);
        let status = typing.map(|_| txt("typing", 10.0).color(p.sub));
        let info = container(column![name_row].push(status).spacing(2).align_x(Center))
            .padding(Padding { top: 6.0, right: 6.0, bottom: 6.0, left: 10.0 })
            .style(raised(p.elevated, 14.0, Some(p.strong.scale_alpha(0.025)), (4.0, 10.0, 0x16)));
        let avatar = container(self.chat_avatar(chat, 40.0, 0.0)).style(raised(p.card, 20.0, None, (1.0, 4.0, 0x1a)));
        // The pill sets the width of the card, and the avatar sits over its middle.
        let card = stack![column![Space::new().height(36.0), info].align_x(Center), container(avatar).center_x(Fill)];
        let card = mouse_area(card)
            .interaction(mouse::Interaction::Pointer)
            .on_press(UiMessage::ToggleRightPanel)
            .on_right_press(UiMessage::ChatList(chat_list::Msg::OpenMenu(cid.into())))
            .on_enter(UiMessage::HeaderHover(true))
            .on_exit(UiMessage::HeaderHover(false));
        let header = container(card).center_x(Fill).padding(Padding::ZERO.top(8.0));
        let feedback = feedback_btn(p);
        let wallpaper = chat_theme::resolve(&st.settings.chat_appearance, Some(cid), self.dark, &st.config.data_dir)
            .0
            .map(|(paint, _)| paint)
            .filter(|_| in_chat);
        let paint = wallpaper.clone().unwrap_or(chat_theme::Paint::Flat(p.card));
        let fade: El<'_> = iced::widget::responsive(move |size| {
            container(chat_theme::fade(paint.clone(), size)).height(chat_theme::FADE).into()
        })
        .into();
        let reminder = chat.reminder.as_ref().filter(|_| !self.expired(chat)).map(|r| {
            let (day, time) =
                (reminder_day(r.remind_at_ms), clock(r.remind_at_ms, st.settings.pref("timeFormat") == 1));
            let at = if day_of(r.remind_at_ms) == today() { time.clone() } else { format!("{time} {day}") };
            let tip = match r.dismiss_on_incoming_message {
                true => trf("Set to remind at %s if no reply", &[&at]),
                false => trf("Set to remind at %s", &[&at]),
            };
            let label = row![ic(icon::SCHEDULE, 16.0).color(ORANGE), semi(format!("{day} {time}"), 11.0).color(ORANGE)];
            let pill = container(label.spacing(4).align_y(Center))
                .padding(Padding { top: 2.0, right: 8.0, bottom: 2.0, left: 4.0 })
                .style(fill_box(mix(p.card, ORANGE, 0.1), 30.0));
            // Desktop's `thread-header-reminder` button opens the Remind Me command bar. A chat with a reminder can
            // always change it.
            let pill = button(self.tip(pill.into(), tip, vec![], tooltip::Position::Bottom))
                .padding(0)
                .style(btn_style([CLEAR; 3], [ORANGE; 3], 30.0))
                .on_press(UiMessage::CommandBar(commands::Msg::OpenSubMenu(commands::SubMenu::Remind(cid.into()))));
            container(pill).padding(Padding::ZERO.top(9.0).left(8.0))
        });
        let feedback = st
            .settings
            .on("feedbackWidget")
            .then(|| container(feedback).align_right(Fill).padding(Padding::ZERO.top(6.0).right(5.0)));
        // Desktop's `thread-header` bar takes the pointer for its 36 px, and below it only the card does.
        let bar = mouse_area(container(Space::new()).width(Fill).height(36.0))
            .interaction(mouse::Interaction::Idle)
            .on_double_click(UiMessage::Window(windows::Msg::ToggleMaximize(self.drawing.get())))
            .on_right_press(UiMessage::ChatList(chat_list::Msg::OpenMenu(cid.into())));
        // An iced `Stack` takes the size of its first layer, so a spacer as tall as the card goes first.
        let header = stack![Space::new().width(Fill).height(84.0), bar, header];
        let members = self.header_hover.0.then(|| self.header_members(chat)).flatten();
        let members = members.filter(|_| in_chat).map(|m| container(m).center_x(Fill).padding(Padding::ZERO.top(84.0)));
        // Desktop's `sticky-date-container`: while the timeline scrolls, the date of the day at its top floats under the
        // header.
        let sticky = self
            .pane()
            .timeline
            .sticky()
            .filter(|_| in_chat)
            .and_then(|i| msgs.get(i))
            .map(|m| self.day_pill(&m.timestamp).padding(Padding::ZERO.top(80.0).left(GUTTER)));
        let overlay = match in_chat {
            true => stack![fade, header].push(feedback).push(reminder),
            false => stack![fade],
        };
        let verify = (in_chat && st.matrix.is_some() && !st.beeper_state.verified).then(|| {
            let note = txt("Verify this device to read your encrypted messages.", 12.0).color(p.text);
            let go = pill_btn(p, tr("Verify"), enter_key::KeyMsg::Open.into());
            let bar = row![note, Space::new().width(Fill), go].align_y(Center);
            container(bar).padding([8, 14]).style(bordered_box(p.elevated, p.chip, 1.0, 10.0))
        });
        let verify = verify.map(|v| container(v).padding([0, 12]));
        // Desktop's `ThreadComposerDisabled` in place of the composer: a link to the new chat of an upgraded chat, or
        // "You blocked __NAME__. Unblock".
        let disabled = |lead: String, link: &str, end: &str, msg| -> El<'_> {
            let part = |t: String, c: Color| txt(t, 13.0).color(c);
            let link = row![part(lead, p.sub), part(link.into(), p.text), part(end.into(), p.sub)];
            let link = button(container(link).center_x(Fill).center_y(36))
                .padding(8)
                .style(btn_style([CLEAR; 3], [p.sub; 3], 0.0))
                .on_press(msg);
            container(link).style(fill_box(p.card, 0.0)).into()
        };
        let upgraded = chat.replacement_room.as_deref().and_then(|r| st.resolve_chat_idx(r)).map(|i| {
            let lead = format!("{} ", tr("Chat was upgraded."));
            let msg = UiMessage::SelectChat(st.chats[i].id.clone());
            disabled(lead, tr("Click here to follow the conversation"), ".", msg)
        });
        let blocked = (st.user_blocked(chat) && chat.block & NO_UNBLOCK == 0).then(|| {
            let lead = tr("You blocked __NAME__. __UNBLOCK__").replace("__NAME__", &chat.title);
            let lead = lead.trim_end_matches("__UNBLOCK__").to_string();
            let unblock = trx("Unblock", "action to unblock the other person in a DM");
            disabled(lead, unblock, "", UiMessage::Blocking(blocking::Msg::Ask(chat.id.clone(), false)))
        });
        // Desktop's `ThreadComposer` draws nothing once the user left the chat or was removed from it.
        let composer = match upgraded.or(blocked) {
            _ if chat.exited() => None,
            Some(link) => Some(link),
            None => unavailable(&st, chat).is_none().then(|| self.view_composer(&st, chat)),
        };
        let jump = self.view_scroll_to_bottom(&st, chat);
        let pane = column![stack![list, overlay]
            .push(sticky)
            .push(members)
            .push(jump)
            .push(replies)
            .push(close)
            .width(Fill)
            .height(Fill)]
        .push(verify)
        .push(composer);
        let secondary = st.settings.pref("secondaryComposerButton") != 6;
        let picker = self
            .view_emoji_picker(&st)
            .filter(|_| self.pane().emoji.as_ref().is_some_and(|pk| pk.react.is_none() && pk.pref.is_none()))
            .map(|e| {
                // Desktop's `.ComposeMessage-module__emojiPopup` is `right: 0` over the emoji button.
                let right = if secondary { 88.0 } else { 58.0 };
                container(e).align_right(Fill).align_bottom(Fill).padding(Padding::ZERO.right(right).bottom(36.0))
            });
        let dropping = self.dropping.then(|| {
            let hint =
                column![bold(tr("Drop files here to attach"), 15.0), bold(tr("Hold Shift to send immediately"), 13.0)];
            // Desktop blurs the chat 10px under a 0.2 background; iced has no backdrop blur.
            let zone = container(hint.spacing(12).align_x(Center)).center(Fill).style(bordered_box(
                Color { a: 0.85, ..p.card },
                Color { a: 0.2, ..p.text },
                10.0,
                0.0,
            ));
            container(zone).padding(Padding { top: 39.0, right: 13.0, bottom: 6.5, left: 6.5 })
        });
        let wallpaper =
            wallpaper.map(|paint| iced::widget::responsive(move |size| chat_theme::fill(paint.clone(), size, 10.0)));
        let pane = stack![].push(wallpaper).push(pane).push(picker).push(self.view_suggestions(&st)).push(dropping);
        container(pane).width(Fill).height(Fill).clip(true).style(pane_card(p.card)).into()
    }

    /// Desktop's `jumpToMessage` and `revealMessage`: shows message `mid` of chat `cid` in the timeline of the pane,
    /// and a reply only in a thread in the thread pane. The thread pane jumps in its own thread.
    pub(crate) fn jump_to(&mut self, cid: String, mid: String) -> Task<UiMessage> {
        (self.show_command_palette, self.command_palette_query, self.chat_menu) = (false, String::new(), None);
        if self.pane.thread_root.is_none() {
            self.select_chat(&cid);
            let st = self.store.read();
            let threads = st.threaded(&st.chats[st.resolve_chat_idx(&cid).expect("a known chat")]);
            let root = st.loaded(&mid).filter(|m| m.thread_only(threads));
            let root = root.map(|m| m.thread_root.clone().expect("a thread reply"));
            drop(st);
            if let Some(root) = root {
                self.open_thread(cid.clone(), root);
                return self.in_thread(UiMessage::CommandBar(commands::Msg::JumpTo(cid, mid)));
            }
            self.pane.thread_focus = false;
        }
        let msg = format!("msg-{mid}");
        self.pane.timeline.reveal(vlist::key(&mid));
        self.flash = Some((mid, std::time::Instant::now()));
        locate(msg, self.pane.scroll_id(), false)
    }

    /// Desktop's `UnreadDivider`: "New", or the incognito line with the days left of a free trial.
    fn unread_divider<'a>(&self, st: &BeeperStore) -> El<'a> {
        let p = &self.pal;
        let hush = st.incognito_indicator();
        let color = if hush { p.rule } else { p.accent.scale_alpha(0.35) };
        let line = || container(Space::new()).width(Fill).height(1).style(fill_box(color, 0.0));
        if !hush {
            return row![line(), semi(tr("New"), 11.0).color(p.accent), line()]
                .spacing(8)
                .align_y(Center)
                .padding([6, 0])
                .into();
        }
        let label = txt(format!("🫣 {}", tr("Unread until you reply")), 11.0);
        let days = st.features.trial_days_left().map(|n| {
            let n = n as usize;
            let days = trn("%d more day of free incognito mode", "%d more days of free incognito mode", n, &[&n]);
            txt(days, 11.0).color(p.sub)
        });
        let label = column![label.color(p.on_secondary)].push(days).spacing(2).align_x(Center);
        row![line(), label, line()].spacing(10).align_y(Center).padding([12, 0]).into()
    }

    /// Desktop's `TopOfThread`: "Loading..." while older messages load, else the Beeper icon at the start of the
    /// room, and the request header of a message request. It tells the app when it comes on or off screen.
    fn top_of_thread<'a>(&'a self, st: &BeeperStore, chat: &Chat) -> El<'a> {
        let fg = self.wallpaper_fg(st, chat, false).unwrap_or(self.pal.text);
        // Desktop fades the text in after 0.2 s over 0.24 s, and moves a 25% to 50% gradient over it each second.
        let loading = self.pane().top.loading.as_ref().map(|(at, ..)| {
            let t = at.elapsed().as_secs_f32();
            let alpha = ((t - 0.2) / 0.24).clamp(0.0, 1.0) * (0.5 - 0.25 * (2.0 * t.fract() - 1.0).abs());
            container(txt("Loading...", 13.0).color(fg.scale_alpha(alpha))).center_x(Fill).padding(13)
        });
        let request = self.view_request_head(st, chat);
        let icon = (loading.is_none() && !has_older(st, chat) && request.is_none()).then(|| {
            let icon = svg(BEEPER_ICON.clone()).width(32.0 * 14.0 / 15.0).height(32);
            container(icon.style(move |_, _| svg::Style { color: Some(fg.scale_alpha(0.1)) })).center_x(Fill)
        });
        let top = column![].push(loading).push(icon).push(request).width(Fill);
        iced::widget::sensor(top)
            .key(chat.id.clone())
            .on_show(|_| UiMessage::Timeline(timeline::Msg::TopShown(true)))
            .on_hide(UiMessage::Timeline(timeline::Msg::TopShown(false)))
            .into()
    }

    /// Starts the fade of Desktop's `ScrollToBottom` when the timeline leaves its bottom (`up`) or comes back to it.
    pub(crate) fn timeline_scrolled(&mut self, up: bool) {
        if self.pane.scrolled_up.is_none_or(|s| s.0 != up) {
            self.pane.scrolled_up = Some((up, std::time::Instant::now()));
        }
    }

    /// Desktop's `ScrollToBottom` over the bottom of the timeline when you scrolled up: a circle with a chevron, or
    /// the count of unread messages with the hotkey. It fades in 50 ms after you scroll up, over 0.15 s.
    fn view_scroll_to_bottom<'a>(&'a self, st: &BeeperStore, chat: &Chat) -> Option<El<'a>> {
        let p = &self.pal;
        let (up, since) = self.pane().scrolled_up?;
        let t = since.elapsed().as_secs_f32();
        let shown = up && self.pane().editing_message_id.is_none();
        let alpha = if shown { ((t - 0.05) / 0.15).clamp(0.0, 1.0) } else { 1.0 - (t / 0.15).clamp(0.0, 1.0) };
        if alpha == 0.0 {
            return None;
        }
        let fade = move |c: Color| c.scale_alpha(alpha);
        let n = chat.unread_count as usize;
        let body: El<'a> = if n > 0 {
            let count = if n >= 1000 { "999+".to_string() } else { n.to_string() };
            let count = semi(count, 13.0).color(fade(Color::WHITE)).center();
            let count = container(count.width(Length::Shrink)).padding([0, 4]).style(fill_box(fade(p.accent), 20.0));
            let label = semi(trn("New Message", "New Messages", n, &[]), 13.0);
            let keys = prefs::hotkey_label(st.settings.hotkey_keys("Scroll to the bottom of a chat"));
            let keys =
                keys.split(' ').map(|k| -> El<'a> { semi(k.to_string(), 13.0).color(fade(p.accent)).center().into() });
            let keys = Row::with_children(keys);
            row![count, label.color(fade(p.accent)), keys].spacing(7).align_y(Center).into()
        } else {
            let chevron = tinted(&CHEVRON_DOWN, 20.0, fade(p.strong).scale_alpha(0.8));
            container(chevron).center(20).into()
        };
        let shadow =
            Shadow { color: fade(Color::from_rgba(0.0, 0.0, 0.0, 0.15)), offset: Vector::ZERO, blur_radius: 20.0 };
        let style = button::Style {
            background: Some(fade(p.card).into()),
            border: border::rounded(30),
            shadow,
            ..Default::default()
        };
        let button = button(body)
            .padding(4)
            .style(move |_, _| style)
            .on_press_maybe(shown.then_some(UiMessage::Timeline(timeline::Msg::ScrollToBottom)));
        let at = container(button).center_x(Fill).align_bottom(Fill);
        Some(at.padding(Padding::ZERO.bottom(21.0 + 6.5)).into())
    }

    /// Desktop's `MessageList`: the messages in timeline order. In a reply chain, the quotes, the reply lines,
    /// the replies buttons and the unread divider do not show.
    /// The rows of the messages of the conversation of the pane. The pane keeps them until the store changes, so that a
    /// frame of a long chat does not read all its messages.
    fn conversation_rows(&self, st: &BeeperStore, chat: &Chat) -> Rc<Rows> {
        let (memo, generation) = (&self.pane().rows, self.store.generation().expect("a view holds a read lock"));
        if let Some((_, _, rows)) = memo.borrow().as_ref().filter(|(g, c, _)| *g == generation && *c == chat.id) {
            return rows.clone();
        }
        let (hide, members) = (st.settings.on("hideDeleted"), st.settings.on("membershipEvents"));
        let msgs: Vec<&Message> = self.pane_msgs(st);
        let msgs: Vec<&Message> =
            msgs.into_iter().filter(|m| !(hide && m.is_deleted == Some(true)) && (members || !membership(m))).collect();
        let rows = Rc::new(Rows::of(st, &msgs));
        *memo.borrow_mut() = Some((generation, chat.id.clone(), rows.clone()));
        rows
    }

    fn view_messages<'a>(
        &'a self,
        st: &BeeperStore,
        chat: &Chat,
        msgs: &[&Message],
        rows: &Rows,
        thread: bool,
    ) -> Column<'a, UiMessage> {
        let p = &self.pal;
        let Rows { run_start, last_live, any_pending, last_seen, unread, chained, counts, gallery, keys, days, .. } =
            rows;
        let last_received = msgs.iter().rposition(|m| m.is_sender != Some(true));
        // Desktop's `ThreadedRepliesView` lists the root and the replies of a thread under its header, and loads older
        // messages of the chat only until the root loads.
        let root = self.pane().thread_root.as_deref();
        let top = if root.is_some() { 38.0 } else { 96.0 };
        let mut col = Column::new().padding(Padding::new(8.0).right(8.0 + GUTTER).top(top).bottom(0.0));
        let loaded = root.is_some_and(|r| msgs.first().is_some_and(|m| m.id == r));
        col = col.push((!thread && !loaded).then(|| self.top_of_thread(st, chat)));
        // Desktop's `getReplyThreadListData`: the thread pane divides at the first reply of others after the read mark.
        let first_unread = match root {
            Some(r) => self.pane().thread_divider.as_ref().and_then(|read| {
                let unread =
                    |m: &&Message| m.sort_key > *read && m.is_sender != Some(true) && m.is_deleted != Some(true);
                msgs.iter().position(|m| m.thread_root.as_deref() == Some(r) && unread(m))
            }),
            None => *unread,
        };
        let first_unread = first_unread.filter(|_| !thread);
        let (h24, group) = (st.settings.pref("timeFormat") == 1, chat.chat_type == "group");
        let item = |i: usize| -> El<'a> {
            // Desktop's `GalleryGrouper` shows a gallery at its first message.
            if gallery[i] != i {
                return Column::new().into();
            }
            let end = i + gallery[i..].iter().take_while(|&&s| s == i).count();
            let (m, prev, tail) = (msgs[i], i.checked_sub(1).map(|j| msgs[j]), msgs[end - 1]);
            let day_changed = prev.is_none_or(|q| day_num(&q.timestamp) != day_num(&m.timestamp));
            let mut head = Column::new().push(prev.map(|q| Space::new().height(gap_below(q, m))));
            if day_changed {
                head = head.push(self.day_pill(&m.timestamp).padding(Padding::new(4.0).left(GUTTER).right(0.0)));
            }
            if first_unread.is_some_and(|u| (i..end).contains(&u)) {
                head = head.push(self.unread_divider(st));
            }
            if let Some(timer) = m.timer {
                let next =
                    msgs.get(i + 1).filter(|n| n.timer.is_some() && day_num(&n.timestamp) == day_num(&m.timestamp));
                return match next.is_none_or(|n| n.sender_id != m.sender_id) {
                    true => head.push(self.timer_notice(st, chat, m, timer, next.is_none())).into(),
                    false => Column::new().into(),
                };
            }
            if membership(m) {
                let mut rows = Column::new();
                let s = run_start[i];
                let size = run_start[s..].iter().take_while(|&&r| r == s).count();
                let open = self.pane().open_members.contains(&msgs[s].id);
                let dim = self.wallpaper_fg(st, chat, thread).unwrap_or(p.muted).scale_alpha(0.75);
                if size >= 3 && i == s {
                    let label = row![
                        txt(format!("{size} Membership Events"), 11.0).color(dim),
                        txt(if open { tr("Collapse") } else { tr("Expand") }, 11.0).color(p.accent.scale_alpha(0.75))
                    ];
                    let toggle = button(label.spacing(4))
                        .padding(0)
                        .style(btn_style([Color::TRANSPARENT; 3], [dim; 3], 0.0))
                        .on_press(UiMessage::Members(members::Msg::Toggle(msgs[s].id.clone())));
                    rows = rows.push(head.push(container(toggle).center_x(Fill)));
                    head = Column::new();
                }
                if size < 3 || open {
                    let gap = if size >= 3 { Space::new().height(6) } else { Space::new() };
                    let line = txt(m.text.clone().expect("membership text"), 11.0);
                    let line = container(line.color(dim).align_x(Center)).center_x(Fill).padding([2, 0]);
                    rows = rows.push(head.push(gap).push(line));
                }
                return rows.into();
            }
            let first = day_changed || prev.is_none_or(|q| q.sender_id != m.sender_id || membership(q));
            let last = msgs.get(end).is_none_or(|n| {
                n.sender_id != m.sender_id || membership(n) || day_num(&n.timestamp) != day_num(&tail.timestamp)
            });
            // Desktop's `showSeenIcon`: never on an action; in a group on an own message or the last one, and in a
            // DM on an own message after the last received one.
            let own = tail.is_sender == Some(true);
            let seen_icon = !membership(tail)
                && match group {
                    true => own || end == msgs.len(),
                    false => own && last_received.is_none_or(|r| end - 1 > r),
                };
            let mut viewers: Vec<(&str, &str)> = (last_seen.iter())
                .filter(|(_, (at, _))| seen_icon && *at == end - 1)
                .map(|(u, (_, ts))| (&u[..], &ts[..]))
                .collect();
            viewers.sort_by_key(|&(_, ts)| ts);
            let status = if let Some(&(_, ts)) = viewers.last() {
                if group {
                    let names: Vec<&str> = viewers
                        .iter()
                        .map(|&(uid, _)| {
                            chat.participants
                                .items
                                .iter()
                                .find(|u| u.id == uid)
                                .and_then(|u| u.custom_nickname.as_deref().or(u.full_name.as_deref()))
                                .unwrap_or(uid)
                        })
                        .collect();
                    Some(if names.len() <= 4 {
                        trf("Seen by %s", &[&names.join(", ")])
                    } else {
                        let (first, more) = (names[..3].join(", "), names.len() - 3);
                        trn("Seen by %1$s & %2$d other", "Seen by %1$s & %2$d others", more, &[&first, &more])
                    })
                } else {
                    let played = matches!(m.message_type.as_deref(), Some("VOICE" | "AUDIO"));
                    Some(match day_num(ts) == today() {
                        true => trf(if played { "Played at %s" } else { "Seen at %s" }, &[&format_short_ts(ts, h24)]),
                        false => trf(if played { "Played %s" } else { "Seen %s" }, &[&day_label(ts)]),
                    })
                }
            } else if m.send_error.is_some() || m.is_deleted == Some(true) || m.is_sender != Some(true) {
                None
            } else if pending(st, m) {
                let queued = self.queued.iter().any(|q| q.sends.iter().any(|s| s.0 == m.id));
                let late = still_sending_at(m) <= now_unix_millis();
                Some(
                    tr(if queued {
                        "Queued"
                    } else if late {
                        "Still sending..."
                    } else {
                        "Sending..."
                    })
                    .into(),
                )
            } else {
                (Some(end - 1) == *last_live && !any_pending)
                    .then(|| tr(if m.is_delivered { "Delivered" } else { "Sent" }).into())
            };
            let split = chained.get(end).is_some_and(|c| !c);
            let replies = if !thread && split && !st.threaded(chat) { counts[end - 1] } else { 0 };
            let ms = &msgs[i..end];
            let hide_time = time_hidden(m, msgs.get(end).copied(), st.settings.pref("timeFormat") == 1);
            let flags = (first, last, chained[i], thread, hide_time);
            let view = self.view_message(st, chat, ms, flags, (status, replies), head);
            let view = match send_rise(st, m) {
                Some(t) => lift::lift(view, 14.0 * (1.0 - t), 0.97 + 0.03 * t).into(),
                None => view,
            };
            let flash = self.flash.as_ref().filter(|(id, _)| *id == m.id).map(|(_, at)| {
                let t = (at.elapsed().as_secs_f32() / 1.5).min(1.0);
                let a = if t < 0.2 { ease_out(t / 0.2) } else { 1.0 - ease_out((t - 0.2) / 0.8) };
                container(Space::new()).width(Fill).height(Fill).style(fill_box(p.accent.scale_alpha(0.4 * a), 0.0))
            });
            let cell = container(stack![view].push(flash));
            let cell = if thread || root.is_some() { cell } else { cell.id(format!("msg-{}", m.id)) };
            match root.filter(|r| *r == m.id).map(|r| st.thread_replies(r).count()).filter(|&n| n > 0) {
                Some(n) => column![cell, self.thread_divider(n)].into(),
                None => cell.into(),
            }
        };
        let list = if thread { &self.pane().thread_list } else { &self.pane().timeline };
        list.sections(days.clone());
        col.push(list.view(keys, true, 48.0, UiMessage::Relist, item))
    }

    /// Desktop's `kmt`, which shows while the pointer is on the header card: the other members who have not left, the
    /// named ones first by name, each with a 20 px avatar, the name and the handles. A click starts a chat with one.
    pub(crate) fn header_members<'a>(&'a self, chat: &Chat) -> Option<El<'a>> {
        let p = &self.pal;
        let others: Vec<&User> =
            chat.participants.items.iter().filter(|u| u.is_self != Some(true) && u.has_exited != Some(true)).collect();
        let mut shown = if others.is_empty() { chat.participants.items.iter().collect() } else { others };
        shown.retain(|u| u.has_exited != Some(true));
        if shown.is_empty() {
            return None;
        }
        // Desktop's `Cr(t, [f => !mD(f), "fullName"])`: the members with a real name first, then by name.
        shown.sort_by_key(|u| (!crate::real_name(u), u.full_name.clone().unwrap_or_default()));
        let rows = shown.into_iter().map(|u| -> El<'a> {
            let full = u.full_name.clone().unwrap_or_default();
            let phone = u.phone_number.as_deref();
            let mut ids: Vec<String> = Vec::new();
            ids.extend(u.username.as_ref().filter(|n| Some(n.as_str()) != phone).map(|n| match n.starts_with('@') {
                true => n.clone(),
                false => format!("@{n}"),
            }));
            ids.extend(phone.filter(|ph| *ph != full).map(crate::fmt_phone));
            ids.extend(u.email.clone().filter(|_| phone != Some(full.as_str())));
            let line = row![self.face(u.img_url.as_deref(), &crate::member_name(u), 20.0, 0.5)]
                .push(container(med(full, 13.0).color(p.text)).padding(Padding::ZERO.right(13.0)))
                .push(row(ids.into_iter().map(|i| -> El<'a> { txt(i, 13.0).color(p.weak).into() })).spacing(8))
                .spacing(4)
                .align_y(Center);
            let item = button(line)
                .padding(4)
                .style(btn_style([CLEAR, p.elevated_hover, p.elevated_hover], [p.text; 3], 4.0))
                .on_press(UiMessage::Members(members::Msg::Message(chat.id.clone(), u.id.clone())));
            container(item).padding([2, 0]).into()
        });
        let list = scrollable(Column::with_children(rows)).direction(stable_bar()).style(scroll_style(p.scroll));
        let list = container(list).max_height(0.9 * self.pane().size.height).padding([2, 4]).style(move |_: &Theme| {
            container::Style {
                background: Some(p.elevated.into()),
                border: iced::Border { color: p.border_neutrals, width: 1.0, radius: 6.0.into() },
                shadow: iced::Shadow {
                    color: Color::from_rgba(0.0, 0.0, 0.0, 0.11),
                    offset: iced::Vector::new(0.0, 13.0),
                    blur_radius: 20.0,
                },
                ..Default::default()
            }
        });
        Some(mouse_area(list).on_enter(UiMessage::HeaderHover(true)).on_exit(UiMessage::HeaderHover(false)).into())
    }

    /// Desktop's `MessageDate` pill of the day of `timestamp`, in the middle of its row.
    fn day_pill<'a>(&self, timestamp: &str) -> iced::widget::Container<'a, UiMessage> {
        let p = &self.pal;
        let label = semi(day_label(timestamp), 11.0).color(p.on_secondary);
        container(container(label).padding([5, 9]).style(bordered_box(p.card, p.chip, 1.0, 12.0))).center_x(Fill)
    }

    /// A message, or the gallery of the messages `ms`.
    fn view_message<'a>(
        &'a self,
        st: &BeeperStore,
        chat: &Chat,
        ms: &[&Message],
        (first, last, chained, thread, hide_time): (bool, bool, bool, bool, bool),
        (status, replies): (Option<String>, usize),
        head: Column<'a, UiMessage>,
    ) -> El<'a> {
        let (p, m) = (&self.pal, ms[0]);
        let (sent, sent_fg, sent_meta) = self.sent_colors(st, Some(&chat.id));
        let on_wall = self.wallpaper_fg(st, chat, thread);
        let muted = on_wall.unwrap_or(p.muted);
        let mine = m.is_sender == Some(true);
        let shows_time = st.settings.on("showTimestamps") && !hide_time;
        let italic = iced::Font { style: iced::font::Style::Italic, ..fw(Weight::Normal) };
        // Desktop keeps the row of a deleted message, with its sender, its menu and its selection.
        let deleted = (m.is_deleted == Some(true)).then(|| {
            let fg = on_wall.unwrap_or(mix(p.card, p.text, 0.8));
            let icon = svg(TRASH.clone())
                .width(13.0 * 448.0 / 512.0)
                .height(13)
                .style(move |_, _| svg::Style { color: Some(fg) });
            let text = tr(if m.removed { "Removed from Beeper" } else { "This message has been deleted" });
            let body = row![icon, txt(text, 13.0).font(italic).color(fg)];
            container(body.spacing(5).align_y(iced::Bottom))
                .padding(Padding::new(13.0).top(9.0).bottom(7.0))
                .style(bordered_box(CLEAR, mix(p.card, p.received, 0.8), 1.0, 17.0))
        });
        let group = chat.chat_type == "group";
        let selected = ms.iter().any(|g| self.pane().selected_msgs.contains(&g.id));
        let rise = send_rise(st, m).unwrap_or(1.0);
        let dim = move |c: Color| (if selected { brightness(c, 0.7) } else { c }).scale_alpha(rise);
        // Desktop's `isErrored` bubble takes the secondary container colors.
        let failed = m.send_error.as_ref().filter(|_| mine);
        let (bg, fg) = match (mine, failed) {
            (true, Some(_)) => (dim(p.received), dim(p.on_secondary)),
            (true, None) => (dim(sent), dim(sent_fg)),
            (false, _) => (dim(p.received), dim(p.strong)),
        };
        let poll = m.poll.as_ref().filter(|_| st.feature("enable-polls").on && chat.polls.is_some());
        // Desktop's `isE2EE` message that the keys did not decrypt yet.
        let utd = m.text.as_deref() == Some(beeper_core::matrix::UTD_TEXT);
        let text = match (&m.poll, poll) {
            (Some(p), None) => p.fallback.clone(),
            _ if utd => tr("This message is waiting for decryption.").into(),
            _ => m.text.clone().unwrap_or_default(),
        };
        let mut parts = Column::new().spacing(2).align_x(if mine { Right } else { iced::Left });
        let named = (group || network_bot(chat, &m.sender_id)) && !mine && first;
        let sender = || {
            let name = sender_label(chat, m);
            self.user_trigger(chat, &m.sender_id, semi(name.clone(), 12.0).color(hex(*pick(&name, &p.names))))
        };
        // Desktop's `hasAttachedMedia`: a message with media names its sender above the bubble, a text bubble inside.
        let media = !m.attachments.is_empty()
            || m.links.iter().any(|l| !l.title.is_empty() || l.img.is_some() || l.summary.is_some());
        // Desktop's `CallActionMessage` takes the place of the text of a call.
        let call = m.call.as_ref().filter(|_| m.is_deleted != Some(true));
        let text_bubble = m.is_deleted != Some(true)
            && call.is_none()
            && m.message_type.as_deref() != Some("HIDDEN")
            && !jumbo(&text)
            && (utd || (!text.is_empty() && poll.is_none() && m.message_type.as_deref() != Some("VOICE")));
        let inside = (named && !media && text_bubble).then(|| container(sender()).padding(Padding::ZERO.bottom(2.0)));
        if named && inside.is_none() {
            parts = parts.push(container(sender()).padding(Padding::ZERO.left(11.0)));
        }
        // Desktop's `Wle`: in the chat of a room with threads, a reply that is also sent to the chat shows its root in
        // place of a quote. The thread pane does not quote the fallback reply of a thread reply.
        let (threaded, in_pane) = (st.threaded(chat), self.pane().thread_root.is_some());
        let root_line = m.thread_root.as_deref().filter(|_| threaded && !in_pane && !thread && m.broadcast);
        parts = parts.push(root_line.map(|r| self.thread_root_line(st, r, mine)));
        let linked =
            m.linked_message_id.as_ref().filter(|_| !thread && root_line.is_none() && !(in_pane && m.fallback));
        let cached = |r: &String| self.quotes.get(r).and_then(|q| q.as_ref()?.as_ref().ok());
        let quote = linked.map(|r| {
            let Some(q) = st.loaded(r).or_else(|| cached(r)) else {
                // Desktop's `L1t`: "Loading…" while `loadMessage` runs, then "Message unavailable".
                let loading = match self.quotes.get(r) {
                    None => {
                        self.wants.ask(&self.wants.quotes, &(chat.id.clone(), r.clone()));
                        true
                    }
                    Some(done) => done.is_none(),
                };
                let text = if loading { tr("Loading…") } else { tr("Message unavailable") };
                let pill =
                    container(txt(text, 12.0).color(p.sub)).padding(8).style(bordered_box(CLEAR, p.chip, 2.0, 17.0));
                return ((!chained).then(|| column![].push(pill)), false);
            };
            let q_mine = q.is_sender == Some(true);
            let col = self.linked_message(chat, q, group, on_wall, sent).filter(|_| !chained);
            (col.map(|c| column![mouse_area(c).on_press(UiMessage::Message(Msg::ShowReplies(q.id.clone())))]), q_mine)
        });
        // Desktop's `pM`: a link with a title, an image or a summary has a preview.
        for link in m.links.iter().filter(|l| !l.title.is_empty() || l.img.is_some() || l.summary.is_some()) {
            parts = parts.push(self.view_link(link, 0.45 * self.pane().size.width, dim));
        }
        if ms.len() > 1 {
            parts = parts.push(self.view_gallery(ms));
        }
        for (ai, a) in m.attachments.iter().enumerate().filter(|_| ms.len() == 1) {
            let name = a.file_name.clone().expect("attachment file name");
            let content: El<'a> = match (a.is_voice_note == Some(true), a.attachment_type.as_str()) {
                (_, "audio") if self.playable(a) => {
                    let ts = shows_time.then(|| {
                        let time = format_short_ts(&m.timestamp, st.settings.pref("timeFormat") == 1);
                        (time, if mine { sent_meta } else { p.sub })
                    });
                    self.view_audio(m, a, mine, ts, fg, bg)
                }
                (false, "img" | "video") => {
                    let is_video = a.attachment_type == "video";
                    let size = a.size.as_ref().and_then(|s| Some((s.width? as f32, s.height? as f32)));
                    let share = if self.sidebar_hidden { 0.85 } else { 0.45 };
                    let (w, h, cover) = media_box(self.pane().size.width, size, share);
                    // Desktop plays a video with `isGif` inline like a GIF, without sound, again and again.
                    let looped = is_video && a.is_gif == Some(true) && m.unresolved_media.is_none();
                    let thumb_url = match looped {
                        true => a.src_url.as_ref().map(|u| format!("{u}{}", crate::media_view::LOOP)),
                        false => a.poster_img.clone().or_else(|| a.src_url.clone()),
                    };
                    let pill = |label: &'a str, alpha: f32| -> El<'a> {
                        container(
                            container(txt(label, 14.0).line_height(1.0).color(Color::WHITE))
                                .center_y(36)
                                .padding([0, 17])
                                .style(fill_box(p.toggle_on, 18.0)),
                        )
                        .center_x(w)
                        .center_y(h)
                        .style(fill_box(Color::BLACK.scale_alpha(alpha), 17.0))
                        .into()
                    };
                    // A cropped still shows from a baked cut (the renderer draws `Cover` and crops past their box).
                    let cut = |u: &String| self.cover_tile(u, (w.round() as u32, h.round() as u32));
                    let cut = thumb_url.as_ref().filter(|_| cover && !looped).and_then(cut);
                    let base: El<'_> = match thumb_url.as_ref().and_then(|u| self.media_at(u)) {
                        Some(Ok(_)) if let Some(t) = cut => image(t.clone()).width(w).height(h).into(),
                        Some(Ok(frames)) => {
                            let frames = anim::playing(frames, st.settings.on("autoplayGifs"));
                            anim::gif(frames, self.started, move |f| image(f).width(w).height(h).border_radius(17.0))
                        }
                        Some(Err(_)) if m.unresolved_media.is_none() => {
                            pill(tr("Could not load. Click to try again"), 0.45)
                        }
                        _ => container(ic(icon::IMAGE, 28.0).color(p.sub))
                            .center_x(w)
                            .center_y(h)
                            .style(fill_box(p.link_card, 17.0))
                            .into(),
                    };
                    let (slot, press): (El<'_>, _) = if let Some(raw) =
                        m.unresolved_media.as_ref().filter(|_| !is_video)
                    {
                        let failed = self.resolving_media.get(&m.id) == Some(&false);
                        let (label, alpha) = match failed {
                            true => (tr("Could not load. Click to try again"), 0.45),
                            false => (tr("Click to load"), 0.25),
                        };
                        (
                            stack![base, pill(label, alpha)].into(),
                            UiMessage::Loaded(loads::Msg::ResolveMedia(
                                m.account_id.clone(),
                                m.id.clone(),
                                raw.clone(),
                            )),
                        )
                    } else if thumb_url.as_ref().and_then(|u| self.media_at(u)).is_some_and(Result::is_err) {
                        (base, UiMessage::Loaded(loads::Msg::RetryMedia(thumb_url.expect("a failed media has a URL"))))
                    } else if let Some(src) = a.src_url.as_ref().filter(|_| is_video && a.is_gif != Some(true)) {
                        parts = parts.push(self.view_video(&m.id, src, base, (w, h), video::Place::Inline(ai)));
                        continue;
                    } else if is_video && a.is_gif != Some(true) {
                        (base, UiMessage::OpenQuickLook(name, "Video attachment".into()))
                    } else {
                        (base, UiMessage::Media(media_view::Msg::OpenCarousel(m.id.clone(), ai, false)))
                    };
                    button(slot).padding(0).style(btn_style([CLEAR; 3], [p.text; 3], 17.0)).on_press(press).into()
                }
                _ => {
                    let meta = file_label(a).into_iter().chain(a.file_size.map(human_size)).collect::<Vec<_>>();
                    // Desktop's `AttachmentFileRow`: at most 240 px wide, the name in `body-sm` on two lines at most,
                    // and the details in `label-sm`.
                    let info = column![clamp(&name, 186.0, 2, 13.0, Weight::Semibold).line_height(Pixels(16.0))]
                        .push((!meta.is_empty()).then(|| txt(meta.join(" · "), 11.0).color(p.sub)))
                        .spacing(4);
                    let line = row![tinted(file_icon(a), 24.0, p.strong), info].spacing(8).align_y(Center);
                    button(container(line).max_width(240.0 - 2.0 * BUBBLE_PAD_X))
                        .padding([8, 11])
                        .style(btn_style([p.received; 3], [p.strong; 3], 17.0))
                        .on_press(UiMessage::Message(Msg::Download(m.id.clone(), Some(ai))))
                        .into()
                }
            };
            parts = parts.push(content);
        }
        if let Some(poll) = poll {
            parts = parts.push(self.view_poll(st, chat, m, poll, (bg, fg)));
        }
        if let Some(pill) = deleted {
            parts = parts.push(pill);
        } else if m.message_type.as_deref() == Some("HIDDEN") {
            let bold = |s: String| span::<(), _>(s).font(fw(Weight::Bold));
            let head = container(txt(tr("Hidden event"), 13.0).font(fw(Weight::Bold)))
                .padding([2.6, 5.2])
                .style(fill_box(p.chip, 0.0));
            let why = tr("Shown because you enabled Settings > General > Show hidden events in timeline");
            let body = rich_text![bold(text.clone()), span::<(), _>(" from "), bold(m.sender_id.clone())]
                .size(13.0)
                .color(p.text);
            let row =
                row![self.tip(head.into(), why, vec![], tooltip::Position::Top), body].spacing(5.2).align_y(Center);
            let section = container(row.padding(Padding::ZERO.right(6.5))).clip(true);
            parts = parts.push(section.style(bordered_box(CLEAR, p.chip, 2.0, 4.0)));
        } else if let Some(call) = call {
            // A 2 px `secondary-container` outline with a 17 px radius, `8px 11px` inside it, and an 8 px gap.
            let tag = if call.is_video() { crate::chat_list::VIDEO_TAG } else { crate::chat_list::CALL_TAG };
            let line = row![
                tinted(crate::chat_list::tag_glyph(tag), 16.0, p.strong),
                txt(crate::chat_list::call_label(call), 13.0).color(p.strong)
            ];
            let pill = container(line.spacing(8).align_y(Center)).padding([9, 13]);
            parts = parts.push(pill.style(bordered_box(CLEAR, p.chip, 2.0, 17.0)));
        } else if jumbo(&text) {
            parts = parts.push(container(txt(text.clone(), 48.0).font(EMOJI_FONT)).padding([0, 12]));
        } else if utd {
            let line = txt(text.clone(), 13.0).font(italic).color(fg.scale_alpha(0.8));
            let time = shows_time.then(|| {
                let time = format_short_ts(&m.timestamp, st.settings.pref("timeFormat") == 1);
                txt(time, 11.0).color(if mine { sent_meta } else { p.sub })
            });
            let body = column![].push(inside).push(row![line].push(time).spacing(4).align_y(Bottom));
            parts = parts.push(container(body).padding([9.0, BUBBLE_PAD_X]).style(fill_box(bg, 17.0)));
        } else if !text.is_empty() && poll.is_none() && m.message_type.as_deref() != Some("VOICE") {
            let ts = shows_time.then(|| format_short_ts(&m.timestamp, st.settings.pref("timeFormat") == 1));
            let room = ts.as_deref().map_or(0.0, |t| crate::style::text_width(t, 11.0, Weight::Normal) + 4.0);
            let time: Option<El<'a>> = ts.map(|t| txt(t, 11.0).color(if mine { sent_meta } else { p.sub }).into());
            // Desktop's `MessageErrorTooltip` takes the place of the time.
            let (room, time) = match failed {
                Some(e) => {
                    let label = if m.edited() { tr("Failed to edit") } else { tr("Not sent") };
                    let icon = svg(ERROR_OUTLINE.clone()).width(20).height(20);
                    let icon = icon.style(|_, _| svg::Style { color: Some(ON_ERROR_CONTAINER) });
                    let tip = self.tip(
                        icon.into(),
                        e.message.clone().unwrap_or_else(|| tr("Unknown error occurred").into()),
                        vec![],
                        tooltip::Position::Left,
                    );
                    let meta = row![txt(label, 11.0).color(ON_ERROR_CONTAINER), tip].spacing(4).align_y(Center);
                    (crate::style::text_width(label, 11.0, Weight::Normal) + 28.0, Some(meta.into()))
                }
                // Desktop's `BookmarkIndicator` goes before the time.
                None if m.is_bookmarked == Some(true) => {
                    let color = if mine { sent_meta } else { p.sub };
                    let icon = svg(BOOKMARK_ICON.clone()).width(10).height(10);
                    let icon = icon.style(move |_, _| svg::Style { color: Some(color) });
                    let label = tr("Bookmarked");
                    let icon = self.tip(icon.into(), label, vec![], tooltip::Position::Left);
                    (room + 14.0, Some(row![icon].push(time).spacing(4).align_y(Center).into()))
                }
                None => (room, time),
            };
            let me = |id: &str| id == st.beeper_state.user_id;
            let arrow =
                || span(icon::ARROW_UP_RIGHT.to_string()).font(iced::Font::with_name("icons")).size(11.0).color(fg);
            let rich = |r: beeper_core::codec::Run| {
                // Desktop's `--font-weight-emphasized` bold, and its inline `code`: `--font-family-mono` at 0.9em and
                // weight 500, on `--color-on-surface` at 0.1 with a 4 px radius and `.1em .33em` of padding.
                let style = if r.italic { iced::font::Style::Italic } else { iced::font::Style::Normal };
                let font = match r.code {
                    true => iced::Font { weight: Weight::Medium, ..iced::Font::MONOSPACE },
                    false => {
                        iced::Font { style, ..crate::style::fw(if r.bold { Weight::Semibold } else { Weight::Normal }) }
                    }
                };
                let mention = r.link.as_deref().and_then(|l| l.strip_prefix("https://matrix.to/#/"));
                let mention = mention.and_then(beeper_core::codec::url_decode).filter(|id| id.starts_with('@'));
                let s = span(r.text).size(13.0).color(fg).font(font).strikethrough(r.strike);
                let s = match r.code {
                    true => s
                        .size(13.0 * 0.9)
                        .background(Color { a: 0.1, ..p.strong })
                        .border(border::rounded(4))
                        .padding(Padding::from([0.1 * 13.0 * 0.9, 0.33 * 13.0 * 0.9])),
                    false => s,
                };
                // Desktop's `.spoiler`: black with the text hidden, and after a click white on 30% black.
                let s = match (r.spoiler, self.pane().spoilers.contains(&m.id)) {
                    (true, false) => s
                        .color(Color::TRANSPARENT)
                        .background(Color::BLACK)
                        .border(border::rounded(2))
                        .padding(Padding::from([1, 2]))
                        .link(format!("{}{}", crate::SPOILER, m.id)),
                    (true, true) => {
                        s.color(Color::WHITE).background(Color::BLACK.scale_alpha(0.3)).padding(Padding::from([1, 2]))
                    }
                    _ => s,
                };
                if r.spoiler {
                    return [Some(s), None].into_iter().flatten();
                }
                let bold = crate::style::fw(Weight::Bold);
                let s = match mention {
                    Some(id) if me(&id) => {
                        let pill = if mine { Color { a: 0.3, ..fg } } else { p.accent };
                        s.font(bold).color(Color::WHITE).background(pill).border(border::rounded(4)).padding(2)
                    }
                    Some(_) => s.font(bold).color(if mine { fg } else { p.accent }),
                    None => s.underline(r.link.is_some()).link_maybe(r.link.clone()),
                };
                let arrow = s.link.is_some().then(arrow);
                [Some(s), arrow].into_iter().flatten()
            };
            let plain = |w: &str| {
                let t = w.trim_end();
                let (linked, url) = match email(t) {
                    Some(e) => (e, Some(format!("mailto:{e}"))),
                    None => (t, t.starts_with("http").then(|| t.to_string())),
                };
                let word = span(linked.to_string()).size(13.0).color(fg).underline(url.is_some());
                let arrow = url.is_some().then(arrow);
                let rest = w[linked.len()..].to_string();
                [Some(word.link_maybe(url)), arrow, (!rest.is_empty()).then(|| span(rest).size(13.0).color(fg))]
                    .into_iter()
                    .flatten()
            };
            let blocks: Vec<(Option<String>, Vec<_>, String)> = match m.html.as_deref() {
                Some(html) => beeper_core::codec::html_blocks(html)
                    .into_iter()
                    .map(|b| {
                        let plain = b.runs.iter().map(|r| r.text.as_str()).collect();
                        (b.marker, with_emoji_font(b.runs.into_iter().flat_map(rich).collect()), plain)
                    })
                    .collect(),
                None => {
                    vec![(None, with_emoji_font(text.split_inclusive(' ').flat_map(plain).collect()), text.clone())]
                }
            };
            let max = 0.45 * self.pane().size.width - 2.0 * BUBBLE_PAD_X;
            // Desktop's `.message-text` clamps a message at 40 lines, and "Show more" opens it. The whole blocks that
            // fit show as they are, and the block that crosses the limit shows its first lines as plain text.
            let clamp = clamp_blocks(blocks.iter().map(|b| b.2.as_str()), max);
            let open = self.pane().unclamped.contains(&m.id);
            let (room, time, footer_time) = match clamp {
                Some(_) => (0.0, None, time),
                None => (room, time, None),
            };
            let (blocks, cut) = match clamp {
                Some((whole, left)) if !open => {
                    let cut = blocks.get(whole).map(|b| b.2.clone()).filter(|_| left > 0);
                    (blocks.into_iter().take(whole).collect(), cut.map(|t| (t, left)))
                }
                _ => (blocks, None),
            };
            let (mut prev_item, last) = (None, blocks.len().saturating_sub(1));
            let nbsp = crate::style::text_width("\u{a0}", 13.0, Weight::Normal);
            let pad = "\u{a0}".repeat((room / nbsp).ceil() as usize);
            let full = !st.settings.on("balanceText") && crate::style::wraps(&format!("{text}{pad}"), max, 13.0);
            let body = Column::with_children(blocks.into_iter().enumerate().map(|(i, (marker, mut spans, _))| {
                // iced does not draw an empty last line.
                if spans.last().is_some_and(|s| s.text.ends_with('\n')) {
                    spans.push(span("\u{200b}").size(13.0));
                }
                if i == last && room > 0.0 {
                    spans.push(span(pad.clone()).size(13.0));
                }
                let line = rich_text(spans)
                    .line_height(iced::widget::text::LineHeight::Absolute(16.0.into()))
                    .wrapping(iced::widget::text::Wrapping::WordOrGlyph)
                    .on_link_click(UiMessage::OpenUrl);
                let gap = match (prev_item.replace(marker.is_some()), marker.is_some()) {
                    (None, _) => 0.0,
                    (Some(true), true) => 6.0,
                    _ => 9.0,
                };
                let indent = |m: &String| if m == "•" { 15.6 } else { 23.4 };
                let marker = marker.map(|m| {
                    let w = indent(&m);
                    container(txt(m, 13.0).color(fg)).width(w).align_right(w).padding(Padding::ZERO.right(5.0))
                });
                container(row![].push(marker).push(line)).padding(Padding::ZERO.top(gap)).into()
            }))
            .width(if full { Length::Fixed(max) } else { Length::Shrink });
            let cut = cut.map(|(t, left)| {
                txt(cut_lines(&t, max, left), 13.0).color(fg).wrapping(iced::widget::text::Wrapping::WordOrGlyph)
            });
            let body =
                stack![column![body].push(cut)].push(time.map(|t| container(t).align_right(Fill).align_bottom(Fill)));
            // Desktop's `read-more-button`: the primary color, white on a sent bubble, with the time at its end.
            let more = clamp.map(|_| {
                let label = semi(if open { tr("Show less") } else { tr("Show more") }, 14.0);
                let color = if mine { fg } else { p.accent };
                let toggle = button(label.line_height(Pixels(18.0)).color(color)).bare(color);
                let toggle = toggle.on_press(UiMessage::Message(Msg::ToggleClamp(m.id.clone())));
                // A Fill gap would widen the bubble past its text, so the time follows the label.
                row![toggle].push(footer_time).spacing(8).align_y(Bottom)
            });
            parts = parts.push(
                container(column![].push(inside).push(body).push(more))
                    .padding([9.0, BUBBLE_PAD_X])
                    .max_width(0.45 * self.pane().size.width)
                    .style(fill_box(bg, 17.0)),
            );
        }
        let open = self.pane().expanded_history_msg_id.as_ref() == Some(&m.id);
        let label = |t: String| txt(t, 12.0).color(muted);
        // Desktop's `EditedLabel` shows on an edited message that is not deleted, failed or retrying.
        let edited = (m.edited() && m.is_deleted != Some(true) && m.send_error.is_none()).then(|| -> El<'a> {
            let small = |t: &str| txt(t.to_string(), 11.0);
            if m.edit_history.is_empty() {
                return small(tr("Edited")).color(muted).into();
            }
            let toggle = small(if open { tr("Hide Edits") } else { tr("Edited") }).font(fw(Weight::Semibold));
            button(toggle.color(p.accent))
                .padding(0)
                .style(|_, _| button::Style::default())
                .on_press(UiMessage::Message(Msg::ToggleEditHistory(m.id.clone())))
                .into()
        });
        let replies = (replies > 0).then(|| -> El<'a> {
            let n = trnx("%d Reply", "%d Replies", replies, "button label for replies to a message", &[&replies]);
            button(semi(n, 10.0).color(muted))
                .padding(0)
                .style(|_, _| button::Style::default())
                .on_press(UiMessage::Message(Msg::ShowReplies(m.id.clone())))
                .into()
        });
        let broadcast =
            (in_pane && m.broadcast).then(|| -> El<'a> { txt(tr("Also sent to the chat"), 11.0).color(muted).into() });
        let mut footer: Vec<El<'a>> =
            [status.map(|t| label(t).into()), replies, broadcast, edited].into_iter().flatten().collect();
        if !mine {
            footer.reverse();
        }
        let footer = (!footer.is_empty()).then(|| {
            let line = footer.into_iter().enumerate().fold(Row::new().align_y(Center), |r, (i, t)| {
                r.push((i > 0).then(|| container(txt("・", 12.0).color(muted.scale_alpha(0.8))).padding([0, 4])))
                    .push(t)
            });
            container(line).padding([0.0, BUBBLE_PAD_X])
        });
        // Desktop's `_fe`: a gallery shows the reactions of all its messages.
        let reactions: Vec<&Reaction> = ms.iter().flat_map(|g| &g.reactions).collect();
        let transcribe = self.view_transcribe(st, m).filter(|_| m.is_deleted != Some(true));
        let tail = if footer.is_some() { 15.0 } else { 0.0 }
            + if reactions.is_empty() { 0.0 } else { 21.0 }
            + if transcribe.is_some() { 16.0 } else { 0.0 };
        let edits = open.then(|| {
            m.edit_history.iter().fold(Column::new().spacing(2), |c, e| {
                c.push(txt(format!("{} · {}", e.text, format_short_ts(&e.timestamp, false)), 11.0).color(p.muted))
            })
        });
        let parts: El<'a> = if reactions.is_empty() {
            parts.into()
        } else {
            let min_w = reaction_keys(ms).len() as f32 * 36.0 + 2.0 * BUBBLE_PAD_X;
            let list = self.reaction_list(chat, ms);
            let look = PillLook {
                group,
                own: if p.card.r < 0.5 { hex(0x1b43aa) } else { hex(0xcbd9ff) },
                other: p.received,
                hover: Some(p.selected),
                fg: (p.text, p.on_secondary),
                ring: (if on_wall.is_some() { CLEAR } else { p.card }, 2.0),
                gap: 2.0,
            };
            let pills = self.reaction_pills(st, chat, ms, &look);
            stack![
                container(column![parts, Space::new().width(min_w)].align_x(if mine { Right } else { iced::Left }))
                    .padding(Padding::ZERO.bottom(21.0)),
                container(crate::popover::popover(
                    pills,
                    list,
                    self.single_msg() == Some(&m.id) && st.settings.on("reactionsOnSelect"),
                ))
                .align_bottom(Fill)
                .width(Fill)
                .align_x(if mine { Right } else { iced::Left })
                .padding([0.0, BUBBLE_PAD_X]),
            ]
            .into()
        };
        // Desktop's error actions: Retry and Remove for a local echo, and Delete for a message that a bridge did not
        // send. A failed edit has none.
        let pending = pending(st, m);
        let actions = failed.filter(|_| pending || !m.edited()).map(|e| {
            let action = |label: String, msg: Option<UiMessage>| {
                button(txt(label, 12.0).line_height(Pixels(16.0)))
                    .padding([4, 8])
                    .style(btn_style([p.received; 3], [p.text; 3], 9999.0))
                    .on_press_maybe(msg)
            };
            let retry = pending.then(|| match e.retrying {
                true => action(format!("{} ...", tr("Retrying")), None),
                false => action(tr("Retry").into(), Some(UiMessage::Message(Msg::RetrySend(m.id.clone())))),
            });
            let remove = (!m.edited()).then(|| match pending {
                true => action(
                    trx("Remove", "message error action").into(),
                    Some(UiMessage::Message(Msg::RemoveUnsent(m.id.clone()))),
                ),
                false => action(
                    trx("Delete", "message error action").into(),
                    Some(UiMessage::Message(Msg::Delete(chat.id.clone(), m.id.clone()))),
                ),
            });
            container(row![].push(retry).push(remove).spacing(4)).padding(Padding::ZERO.top(4.0))
        });
        // Desktop's `ThreadRepliesButton` shows under the root in the chat of a room with threads.
        let thread_button =
            (threaded && !in_pane && !thread).then(|| self.thread_replies_button(st, chat, &m.id)).flatten();
        let thread_button = thread_button.map(|b| container(b).padding(Padding::ZERO.top(2.0)));
        let parts = column![parts]
            .push(edits)
            .push(actions)
            .push(thread_button)
            .push(footer)
            .push(transcribe)
            .align_x(if mine { Right } else { iced::Left });
        let tools = (self.pane().hovered_msg.as_ref() == Some(&m.id)).then(|| {
            let b = |c: char, msg: UiMessage| -> El<'a> {
                let style = move |_: &Theme, s| button::Style {
                    text_color: match s {
                        button::Status::Hovered => brightness(muted, 0.8),
                        _ => muted,
                    },
                    ..Default::default()
                };
                button(container(ic(c, 20.0)).center(32)).padding(0).style(style).on_press(msg).into()
            };
            let live = m.is_deleted != Some(true);
            let reply = (chat.messages.reply && live)
                .then(|| b(icon::REPLY, UiMessage::Composer(composer::Msg::StartReply(m.id.clone()))));
            let in_thread = (!in_pane && can_thread(st, chat, m)).then(|| -> El<'a> {
                let style = move |_: &Theme, s| button::Style {
                    text_color: match s {
                        button::Status::Hovered => brightness(muted, 0.8),
                        _ => muted,
                    },
                    ..Default::default()
                };
                let glyph = container(tinted(&VIEW_SIDEBAR, 20.0, muted)).center(32);
                let b =
                    button(glyph).padding(0).style(style).on_press(UiMessage::Thread(threads::Msg::Show(m.id.clone())));
                self.tip(b.into(), tr("Reply in thread"), vec![], tooltip::Position::Top).into()
            });
            let react = (chat.messages.react && m.reactable()).then(|| {
                mouse_area(b(icon::SMILE, UiMessage::Message(Msg::React(m.id.clone()))))
                    .on_enter(UiMessage::Message(Msg::ShowQuickReactions))
                    .into()
            });
            let quick =
                (self.quick_open && st.settings.on("quickReactions")).then(|| crate::emoji::quick_reactions(st));
            let quick = quick.into_iter().flatten().map(|e| -> El<'a> {
                let msg = UiMessage::Message(Msg::ToggleReaction(chat.id.clone(), m.id.clone(), e.clone()));
                let glyph = txt(e, 18.0).font(EMOJI_FONT);
                button(container(glyph).center(32))
                    .padding(0)
                    .style(btn_style([CLEAR; 3], [p.text; 3], 8.0))
                    .on_press(msg)
                    .into()
            });
            // Desktop's download button beside an attachment, and the time of a message without text in its flyover.
            // The items of a gallery open in the carousel, so a gallery has no download button here.
            let file = m
                .attachments
                .first()
                .filter(|a| live && ms.len() == 1 && a.attachment_type != "audio" && a.is_voice_note != Some(true));
            let download = file.map(|a| -> El<'a> {
                let tip = match a.file_size {
                    Some(n) => trf("Download - %s", &[&human_size(n)]),
                    None => trx("Download", "message attachment action").into(),
                };
                let button = button(ic(icon::DOWNLOAD, 20.0))
                    .bare(p.accent)
                    .on_press(UiMessage::Message(Msg::Download(m.id.clone(), Some(0))));
                let gap = if mine { Padding::ZERO.right(12.0) } else { Padding::ZERO.left(12.0) };
                container(self.tip(button.into(), tip, vec![], tooltip::Position::Top)).padding(gap).into()
            });
            let time = (file.is_some() && text.is_empty() && shows_time).then(|| -> El<'a> {
                txt(format_short_ts(&m.timestamp, st.settings.pref("timeFormat") == 1), 11.0).color(muted).into()
            });
            let all: Vec<El<'a>> =
                [download, time, reply, in_thread, react].into_iter().flatten().chain(quick).collect();
            match mine {
                true => Row::with_children(all.into_iter().rev()),
                false => Row::with_children(all),
            }
        });
        let side = (group && !mine).then(|| -> El<'a> {
            if last {
                let url =
                    chat.participants.items.iter().find(|u| u.id == m.sender_id).and_then(|u| u.img_url.as_deref());
                // Desktop's `sender-img`: a click starts a chat with the sender, when the account can.
                let dm = UiMessage::Members(members::Msg::Message(chat.id.clone(), m.sender_id.clone()));
                let face = mouse_area(self.face(url, &sender_label(chat, m), FACE, 0.0));
                let face = if self.can(&chat.account_id, "create_dm") { face.on_press(dm) } else { face };
                self.user_trigger(chat, &m.sender_id, face)
            } else {
                Space::new().width(FACE).into()
            }
        });
        let line: El<'a> = if mine {
            row![Space::new().width(Fill)].push(tools).push(parts).spacing(6).align_y(Center).into()
        } else {
            let body = row![parts].push(tools).spacing(6).align_y(Center);
            row![].push(side).push(body).push(Space::new().width(Fill)).spacing(FACE_GAP).align_y(Bottom).into()
        };
        let (line, head): (El<'a>, El<'a>) = match quote {
            None => (line, head.into()),
            Some((q, q_mine)) => {
                let color = on_wall.unwrap_or(p.reply_line);
                let bar = || container(Space::new()).width(3).height(Fill).style(fill_box(color, 1.5));
                let curve = svg(REPLY_CURVE[usize::from(q_mine)].clone())
                    .width(25)
                    .height(19)
                    .style(move |_, _| svg::Style { color: Some(color) });
                let side = |c: Column<'a, UiMessage>| -> El<'a> {
                    let c = c.align_x(if q_mine { Right } else { iced::Left });
                    match q_mine {
                        true => row![Space::new().width(Fill), c.padding(Padding::ZERO.right(14.0))].into(),
                        false => row![]
                            .push(group.then(|| Space::new().width(FACE + FACE_GAP)))
                            .push(c.padding(Padding::ZERO.left(14.0)))
                            .into(),
                    }
                };
                let cross = q_mine != mine;
                // The curve ends level with the middle of the bubble, above the footer and the reactions.
                let link = column![bar(), curve, Space::new().height(Fill), Space::new().height(tail + 19.0)];
                let line = if cross { stack![line, side(link)].into() } else { line };
                let head = if cross && chained { stack![head, side(column![bar()])].into() } else { head.into() };
                let quote = q.map(|q| -> El<'a> {
                    if q_mine {
                        row![Space::new().width(Fill), q].into()
                    } else {
                        row![].push(group.then(|| Space::new().width(FACE + FACE_GAP))).push(q).into()
                    }
                });
                (column![].push(quote).push(line).into(), head)
            }
        };
        let (menu, click) = match ms.len() {
            1 => (UiMessage::Message(Msg::OpenMenu(m.id.clone())), UiMessage::Message(Msg::Click(m.id.clone()))),
            _ => {
                let ids: Vec<String> = ms.iter().map(|g| g.id.clone()).collect();
                (UiMessage::Gallery(gallery::Msg::Menu(ids.clone())), UiMessage::Gallery(gallery::Msg::Click(ids)))
            }
        };
        let line = mouse_area(line)
            .on_enter(UiMessage::Message(Msg::HoverEnter(m.id.clone())))
            .on_exit(UiMessage::Message(Msg::HoverExit(m.id.clone())))
            .on_right_press(menu)
            .on_press(click)
            .on_double_click(UiMessage::Message(Msg::DoubleClick(m.id.clone())));
        column![head, line].into()
    }

    /// Desktop's `bfe` (`LinkedMessage`) of the quoted message `q`: a box of its media, the tags of its other
    /// attachments and its deletion, over a box of its text with the spaces collapsed and cut at 128 characters. A
    /// group names the sender above media, else at the top of the text. A deleted hidden message shows nothing.
    fn linked_message<'a>(
        &'a self,
        chat: &Chat,
        q: &Message,
        group: bool,
        on_wall: Option<Color>,
        sent: Color,
    ) -> Option<Column<'a, UiMessage>> {
        let p = &self.pal;
        let deleted = q.is_deleted == Some(true);
        if deleted && q.message_type.as_deref() == Some("HIDDEN") {
            return None;
        }
        let q_mine = q.is_sender == Some(true);
        let (edge, fill) = if q_mine { (sent, mix(p.card, sent, 0.3)) } else { (p.received, p.card) };
        let max = 0.45 * self.pane().size.width;
        let boxed = move |c: El<'a>| container(c).padding(8).max_width(max).style(bordered_box(fill, edge, 2.0, 17.0));
        let visual = q.attachments.iter().any(|a| quote_tile(a) || a.attachment_type == "audio");
        let name = || semi(sender_label(chat, q), 12.0).line_height(Pixels(16.0));
        let items: Vec<El<'a>> = q.attachments.iter().map(|a| self.linked_attachment(a)).collect();
        let media = (!items.is_empty() || deleted).then(|| {
            let gone = deleted.then(|| med(tr("Message Deleted"), 13.0).color(p.text));
            boxed(Row::with_children(items).push(gone).spacing(4).align_y(Center).into())
        });
        let text = q.text.as_deref().filter(|_| !deleted).map(|t| t.split_whitespace().collect::<Vec<_>>().join(" "));
        let text = text.filter(|t| !t.is_empty()).map(|t| match t.chars().count() > 128 {
            true => format!("{}...", t.chars().take(125).collect::<String>()),
            false => t,
        });
        let within = (group && !visual).then(|| name().color(p.strong));
        let text = text.map(|t| boxed(column![].push(within).push(med(t, 13.0).color(p.text)).spacing(2).into()));
        let above = (group && visual).then(|| {
            container(name().color(on_wall.unwrap_or(p.strong))).padding(Padding::new(0.0).left(11.0).right(11.0))
        });
        let shown = media.is_some() || text.is_some();
        let col = column![].push(above).push(media).push(text).spacing(8);
        shown.then(|| col.align_x(if q_mine { Right } else { iced::Left }))
    }

    /// An attachment in Desktop's `bfe`: a 100 px tile of an image or a video at half opacity, a tag of a sticker, a
    /// GIF, a voice note, an audio file or a contact card, or the file row of Desktop's `FileAttachment`.
    fn linked_attachment<'a>(&'a self, a: &Attachment) -> El<'a> {
        let p = &self.pal;
        if quote_tile(a) {
            let tile = a.poster_img.as_ref().or(a.src_url.as_ref()).and_then(|u| self.cover_tile(u, (100, 100)));
            return match tile {
                Some(t) => image(t.clone()).width(100).height(100).opacity(0.5_f32).into(),
                None => container(ic(icon::IMAGE, 24.0).color(p.sub)).center_x(100).center_y(100).into(),
            };
        }
        let name = a.file_name.as_deref().unwrap_or_default();
        let tag = match () {
            _ if a.is_sticker == Some(true) => Some(tr("🖼 [Sticker]").to_string()),
            _ if a.is_gif == Some(true) => Some(tr("🎬 [GIF]").to_string()),
            _ if a.attachment_type == "audio" && a.is_voice_note == Some(true) => Some(tr("🎤 [Voice Note]").into()),
            _ if a.attachment_type == "audio" => Some(tr("🎵 [Audio]").to_string()),
            _ if name.ends_with(".vcf") || a.mime_type.as_deref() == Some("text/x-vcard") => {
                Some(trf("👤 [Contact: %s]", &[&contact_name(name)]))
            }
            _ => None,
        };
        if let Some(tag) = tag {
            return med(tag, 13.0).color(p.text).into();
        }
        let meta = file_label(a).into_iter().chain(a.file_size.map(human_size)).collect::<Vec<_>>();
        let info = column![med(name.to_string(), 13.0).color(p.text)]
            .push((!meta.is_empty()).then(|| txt(meta.join(" ・ "), 11.0).color(p.sub)));
        row![tinted(file_icon(a), 20.0, p.text), info].spacing(8).align_y(Center).into()
    }

    /// Desktop's `u2t`: a disappearing timer change between two rules, with "Change Setting" on the last one.
    fn timer_notice<'a>(&'a self, st: &BeeperStore, chat: &Chat, m: &Message, timer: u64, last: bool) -> El<'a> {
        let p = &self.pal;
        let who = match st.is_me(&m.sender_id) {
            true => tr("You").to_string(),
            false => chat
                .participants
                .items
                .iter()
                .find(|u| u.id == m.sender_id)
                .map_or_else(|| tr("Someone").to_string(), member_name),
        };
        let text = match timer {
            0 => trf("%s turned off disappearing messages.", &[&who]),
            t => trf("%1$s set new messages to disappear after %2$s.", &[&who, &timer_distance(t)]),
        };
        let small = |s: String, c: Color| semi(s, 11.0).color(c);
        let change = last.then(|| {
            button(small(tr("Change Setting").into(), p.accent))
                .padding(Padding::ZERO.left(4.0))
                .style(btn_style([CLEAR; 3], [p.accent; 3], 0.0))
                .on_press(UiMessage::ToggleRightPanel)
        });
        let rule = || container(Space::new()).width(Fill).height(1).style(fill_box(p.chip, 0.0));
        let line = row![small(text, p.on_secondary).align_x(Center)].push(change).align_y(Center);
        let line = row![rule(), container(line).max_width(400), rule()].spacing(10).align_y(Center);
        container(line).padding(Padding::ZERO.bottom(6.0).left(GUTTER)).into()
    }

    /// Desktop's reactions list of messages `ms`: each reaction with the face and the name of its sender, in key
    /// order, and "and N more" after 19 when there are more than 20.
    pub(crate) fn reaction_list<'a>(&self, chat: &Chat, ms: &[&Message]) -> El<'a> {
        let p = &self.pal;
        let reactions: Vec<&Reaction> = ms.iter().flat_map(|m| &m.reactions).collect();
        let keys = reaction_keys(ms);
        let order = keys.iter().flat_map(|&k| reactions.iter().filter(move |x| x.reaction_key == k));
        let all: Vec<_> = order.copied().collect();
        let (shown, more) = if all.len() <= 20 { (&all[..], 0) } else { (&all[..19], all.len() - 19) };
        let list = shown.iter().fold(Column::new(), |col, x| {
            let u = chat.participants.items.iter().find(|u| u.id == x.participant_id);
            let name = u.and_then(|u| u.full_name.as_deref()).unwrap_or(&x.participant_id);
            let line = row![
                container(txt(x.reaction_key.clone(), 17.0).font(EMOJI_FONT)).padding(Padding::ZERO.right(10.0)),
                container(self.face(u.and_then(|u| u.img_url.as_deref()), name, 20.0, 0.5))
                    .padding(Padding::ZERO.right(8.0)),
                med(name.to_string(), 13.0).color(p.text),
            ]
            .align_y(Center);
            col.push(container(line).padding([4, 8]))
        });
        let list = list.push((more > 0).then(|| container(txt(format!("and {more} more"), 13.0)).padding([4, 8])));
        container(list).padding([2, 4]).style(bordered_box(p.menu, p.menu_border, 1.0, 6.0)).into()
    }

    /// Desktop's `MessageReactions` pills of messages `ms`: each key with up to three faces when `look.group` or when
    /// several people reacted, and "+N" for more. A click adds or removes your reaction on all the messages.
    pub(crate) fn reaction_pills<'a>(
        &self,
        st: &BeeperStore,
        chat: &Chat,
        ms: &[&Message],
        look: &PillLook,
    ) -> Row<'a, UiMessage> {
        reaction_keys(ms).into_iter().fold(Row::new().spacing(look.gap), |r, k| {
            let reactors: Vec<_> = ms.iter().flat_map(|m| &m.reactions).filter(|x| x.reaction_key == k).collect();
            let n = reactors.len();
            let own = reactors.iter().any(|x| st.is_me(&x.participant_id));
            let mut pill =
                row![txt(k.to_string(), 16.0).line_height(Pixels(15.0)).font(EMOJI_FONT)].spacing(4).align_y(Center);
            if look.group || n > 1 {
                for x in reactors.iter().take(3) {
                    let u = chat.participants.items.iter().find(|u| u.id == x.participant_id);
                    let name = u.and_then(|u| u.full_name.as_deref()).unwrap_or(&x.participant_id);
                    pill = pill.push(self.face(u.and_then(|u| u.img_url.as_deref()), name, 16.0, 0.5));
                }
                if n > 3 {
                    pill = pill.push(semi(format!("+{}", n - 3), 11.0).color(look.fg.1));
                }
            }
            let fill = if own { look.own } else { look.other };
            let hover = look.hover.unwrap_or(fill);
            let style = bordered_btn([fill, hover, hover], [look.fg.0; 3], look.ring.0, look.ring.1, 14.0);
            let pill = button(pill).padding([4.0 + look.ring.1, 5.0 + look.ring.1]).style(style);
            r.push(pill.on_press(toggle_reaction(st, &chat.id, ms, k)))
        })
    }
}

#[derive(Clone)]
pub enum Msg {
    Delete(String, String),
    /// Desktop's `removeFromBeeper` of the messages.
    RemoveFromBeeper(Vec<String>),
    ToggleReaction(String, String, String),
    ToggleBookmark(String),
    MarkUnreadFromHere(String, String),
    /// Desktop's Mark as Read of the message menu.
    MarkReadThrough(String, String),
    /// The poll message and all the own answers.
    VotePoll(String, Vec<String>),
    /// Desktop's `showReplyThread`: shows the reply chain of the message over the timeline.
    ShowReplies(String),
    HideReplies,
    /// Desktop's `PollVotesBrowserWindow` for a chat's poll message.
    OpenPollVotes(String, String),
    EndPoll(String, String),
    OpenMenu(String),
    /// Select the message, or unselect it when it is selected.
    Click(String),
    Select(Vec<String>),
    /// Copy the text of the selected messages, with their timestamps or not.
    CopySelected(bool),
    /// Do the "On double-clicking a message" action.
    DoubleClick(String),
    /// Select the message above (`false`) or below (`true`) the selected one, as with the arrow keys.
    /// Select the next or the previous message, or with `extend`, add it to the selection.
    Step(bool, bool),
    /// Open the emoji picker at the cursor to react to a message.
    React(String),
    /// Open the emoji picker at a point to react to a message.
    ReactAt(String, Point),
    /// An action picked in the message menu, which closes it.
    Menu(Box<UiMessage>),
    CopyText(String),
    /// Copy a link and say so, like Desktop's Media window.
    CopyLink(String),
    CopyImage(String, usize),
    /// The width, the height and the RGBA pixels of an image to copy.
    ImageFetched(Result<(usize, usize, Vec<u8>), String>),
    /// Open a message in the main window.
    GoTo(String, String),
    /// Save the attachments of a message, or only one of them, to the downloads folder.
    Download(String, Option<usize>),
    Downloaded(Result<String, String>),
    /// Desktop's "Delete message?" dialog of the messages.
    AskDelete(Vec<String>),
    /// Desktop's `retryErroredMessage` and `removeErroredMessage` of a failed send.
    RetrySend(String),
    RemoveUnsent(String),
    ToggleEditHistory(String),
    /// "Show more" or "Show less" of a message over 40 lines.
    ToggleClamp(String),
    HoverEnter(String),
    ShowQuickReactions,
    HoverExit(String),
}

impl BeeperGuiApp {
    pub(crate) fn message(&mut self, msg: Msg) -> Task<UiMessage> {
        match msg {
            Msg::Delete(cid, mid) => {
                self.confirm = None;
                let mut st = self.store.write();
                st.redact_message(&cid, &mid).expect("delete message");
            }
            Msg::RemoveFromBeeper(ids) => self.remove_from_beeper(ids),
            Msg::ToggleReaction(cid, mid, key) => {
                let mut st = self.store.write();
                let mine =
                    stored(&st, &mid).reactions.iter().any(|r| r.reaction_key == key && st.is_me(&r.participant_id));
                if mine {
                    st.remove_reaction(&cid, &mid, &key).expect("remove reaction");
                } else {
                    st.add_reaction(&cid, &mid, &key, None).expect("add reaction");
                    st.add_recent_emoji(&key);
                    if let Some(c) = st.matrix.clone() {
                        let content = serde_json::json!({ "recent_emoji": st.recent_emoji });
                        let write = beeper_core::matrix::set_global(c, "io.element.recent_emoji", content);
                        return Task::perform(write, UiMessage::Pushed);
                    }
                }
            }
            Msg::ToggleBookmark(mid) => {
                self.chat_menu = None;
                self.store.write().toggle_bookmark(&mid);
            }
            Msg::MarkUnreadFromHere(cid, mid) => {
                self.store.write().mark_unread_from_message(&cid, &mid);
                self.refresh_theme_and_tray();
            }
            Msg::MarkReadThrough(cid, mid) => {
                self.store.write().mark_read_through(&cid, &mid);
                self.refresh_theme_and_tray();
            }
            Msg::VotePoll(mid, answers) => {
                let mut st = self.store.write();
                let chat = stored(&st, &mid).chat_id;
                st.vote_poll(&chat, &mid, answers);
            }
            Msg::ShowReplies(id) => (self.pane.selected_msgs, self.pane.reply_thread) = (Vec::new(), Some(id)),
            Msg::HideReplies => (self.pane.selected_msgs, self.pane.reply_thread) = (Vec::new(), None),
            Msg::EndPoll(chat, mid) => self.store.write().end_poll(&chat, &mid),
            Msg::OpenPollVotes(chat, mid) => {
                if let Some(w) = self.vote_windows.iter().find(|w| w.2 == mid) {
                    return window::gain_focus(w.0);
                }
                let (id, open) = self.open_child(Size::new(342.0, 563.0));
                self.vote_windows.push((id, chat, mid));
                return open;
            }
            Msg::OpenMenu(mid) => {
                self.video.iter_mut().for_each(|v| v.sub = false);
                (self.audio.sub, self.audio.lang) = (false, false);
                self.pane.message_menu = Some((mid, self.cursor.get()));
                self.pane.gallery_menu.clear();
            }
            Msg::Select(mids) => self.pane.selected_msgs = mids,
            Msg::CopySelected(stamp) => {
                let st = self.store.read();
                let (msgs, _) = self.timeline(&st);
                let picked: Vec<&Message> =
                    msgs.into_iter().filter(|m| self.pane.selected_msgs.contains(&m.id)).collect();
                let many = picked.len() > 1;
                let text: Vec<String> = picked.iter().map(|m| copy_text(m, stamp, many)).collect();
                drop(st);
                return self.update(UiMessage::Message(Msg::CopyText(text.join("\n"))));
            }
            Msg::Click(mid)
                if !self.mods.shift()
                    && self.store.read().settings.on("undoOnPendingClick")
                    && self.queued.iter().any(|q| q.sends.iter().any(|s| s.0 == mid)) =>
            {
                return self.update(UiMessage::Composer(composer::Msg::UndoSend));
            }
            Msg::Click(mid) => {
                let picked = self.pane.selected_msgs.contains(&mid);
                if self.mods.shift() {
                    let st = self.store.read();
                    let (msgs, live) = self.timeline(&st);
                    let at = |id: &str| msgs.iter().position(|m| m.id == id).expect("the message is in the open chat");
                    let (a, b) = (at(self.pane.msg_anchor.as_deref().unwrap_or(&mid)), at(&mid));
                    let range = (a.min(b)..=a.max(b)).filter(|&i| live[i]).map(|i| msgs[i].id.clone());
                    let range: Vec<String> = range.filter(|id| !self.pane.selected_msgs.contains(id)).collect();
                    drop(st);
                    self.pane.selected_msgs.extend(range);
                } else {
                    match (self.mods.command(), picked) {
                        (true, false) => self.pane.selected_msgs.push(mid.clone()),
                        (false, false) => self.pane.selected_msgs = vec![mid.clone()],
                        (_, true) => self.pane.selected_msgs.retain(|m| *m != mid),
                    }
                }
                self.pane.msg_anchor = Some(mid);
            }
            Msg::DoubleClick(mid) => {
                let st = self.store.read();
                let action = st.settings.pref("messageDoubleClick");
                let reactable = st.loaded(&mid).is_some_and(Message::reactable);
                drop(st);
                return match action {
                    1 | 5 if !reactable => Task::none(),
                    0 => Task::none(),
                    1 => self.update(UiMessage::Message(Msg::React(mid))),
                    3 => self.update(UiMessage::Forward(forward::Msg::Open(vec![mid]))),
                    2 => {
                        self.pane.selected_msgs = vec![mid.clone()];
                        self.update(UiMessage::Composer(composer::Msg::StartReply(mid)))
                    }
                    4 => self.update(UiMessage::Message(Msg::OpenMenu(mid))),
                    5 => self.update(self.quick_react(mid)),
                    n => unreachable!("messageDoubleClick option {n}"),
                };
            }
            Msg::Step(down, extend) => {
                let st = self.store.read();
                let (msgs, live) = self.timeline(&st);
                let live = |i: &usize| live[*i];
                let picked = |m: &&Message| self.pane.selected_msgs.contains(&m.id);
                let (first, last) = (msgs.iter().position(picked), msgs.iter().rposition(picked));
                let extend = extend && last.is_some();
                let next = match (first.zip(last), down) {
                    (None, false) => (0..msgs.len()).rev().find(live),
                    (None, true) => None,
                    (Some((_, l)), true) => (l + 1..msgs.len()).find(live),
                    (Some((f, _)), false) if extend => (0..f).rev().find(live),
                    (Some((_, l)), false) => (0..l).rev().find(live).or(Some(l).filter(live)),
                };
                let next = next.map(|i| msgs[i].id.clone());
                drop(st);
                match (extend, next) {
                    (true, Some(mid)) => self.pane.selected_msgs.push(mid),
                    (true, None) => return Task::none(),
                    (false, next) => self.pane.selected_msgs = next.into_iter().collect(),
                }
                return match self.pane.selected_msgs.last() {
                    Some(mid) => {
                        self.pane.timeline.reveal(vlist::key(mid));
                        locate(format!("msg-{mid}"), self.pane.scroll_id(), true)
                    }
                    None => operation::focus("composer-input"),
                };
            }
            Msg::React(mid) => return self.open_emoji(Some((mid, self.cursor.get()))),
            Msg::ReactAt(mid, at) => return self.open_emoji(Some((mid, at))),
            Msg::Menu(msg) => {
                self.pane.message_menu = None;
                return self.update(*msg);
            }
            Msg::CopyText(s) => return iced::clipboard::write(s),
            Msg::CopyImage(mid, ai) => {
                let st = self.store.read();
                let a = &stored(&st, &mid).attachments[ai];
                let url = a.src_url.clone().expect("an image has a source");
                let fetch = beeper_core::matrix::media(st.matrix.clone(), url);
                let decode = async move {
                    let bytes = fetch.await?;
                    let decode = move || {
                        let px = ::image::load_from_memory(&bytes).map_err(|e| e.to_string())?.into_rgba8();
                        Ok((px.width() as usize, px.height() as usize, px.into_raw()))
                    };
                    tokio::task::spawn_blocking(decode).await.expect("image decode task")
                };
                return Task::perform(decode, |a0| UiMessage::Message(Msg::ImageFetched(a0)));
            }
            Msg::ImageFetched(r) => {
                let copy = |(width, height, px): (usize, usize, Vec<u8>), c: &mut Option<arboard::Clipboard>| {
                    let img = arboard::ImageData { width, height, bytes: px.into() };
                    let c = match c {
                        Some(c) => c,
                        None => c.insert(arboard::Clipboard::new().map_err(|e| e.to_string())?),
                    };
                    c.set_image(img).map_err(|e| e.to_string())
                };
                if let Err(e) = r.and_then(|b| copy(b, &mut self.image_clipboard)) {
                    eprintln!("copy image failed: {e}");
                    self.status_banner = tr("Couldn't copy the image").into();
                }
            }
            Msg::CopyLink(url) => {
                self.status_banner = tr("Copied to clipboard!").into();
                return iced::clipboard::write(url);
            }
            Msg::GoTo(chat, mid) => {
                let main = window::gain_focus(self.main_window.expect("the main window is open"));
                return Task::batch([self.update(UiMessage::CommandBar(commands::Msg::JumpTo(chat, mid))), main]);
            }
            Msg::Download(mid, only) => {
                let st = self.store.read();
                let m = &stored(&st, &mid);
                let files: Vec<(String, String)> = m
                    .attachments
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| only.is_none_or(|o| o == *i))
                    .map(|(_, a)| {
                        let name = a.file_name.clone().expect("an attachment has a file name");
                        (a.src_url.clone().expect("an attachment has a source"), name)
                    })
                    .collect();
                let client = st.matrix.clone();
                self.status_banner = trf("Downloading %s", &[&files[0].1]);
                self.sticky_banner = Some(self.status_banner.clone());
                let (dir, ask) = (downloads_dir(&st.settings), st.settings.on("alwaysAskDownload"));
                return Task::perform(
                    async move {
                        let mut last = String::new();
                        for (url, name) in files {
                            let name = safe_file_name(&name);
                            if name.trim_matches('.').is_empty() {
                                return Err(format!("bad file name {name:?}"));
                            }
                            // Desktop's `always_ask_download_location`: the save dialog picks each path.
                            let path = match ask {
                                true => {
                                    let save = portal::Pick::Save(name.clone());
                                    let picked = portal::open_files(portal::PORTAL.into(), tr("Save").into(), save);
                                    match picked.await?.into_iter().next() {
                                        Some(path) => path,
                                        None => continue,
                                    }
                                }
                                false => {
                                    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                                    unused_path(&dir, &name)
                                }
                            };
                            let bytes = beeper_core::matrix::media(client.clone(), url).await?;
                            std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
                            last = path.file_name().expect("a file path").to_string_lossy().into_owned();
                        }
                        Ok(last)
                    },
                    |a0| UiMessage::Message(Msg::Downloaded(a0)),
                );
            }
            Msg::Downloaded(r) => {
                self.status_banner = match r {
                    Ok(name) => trf("Downloaded %s", &[&name]),
                    Err(e) => trf("Download failed: %s", &[&e]),
                }
            }
            Msg::AskDelete(mids) => {
                (self.confirm, self.delete_target) = (Some(ASK_DELETE_MESSAGE), Some(mids.join(" ")))
            }
            Msg::RetrySend(id) => self.store.write().retry_send(&id),
            Msg::RemoveUnsent(id) => self.store.write().remove_unsent(&id),
            Msg::ToggleClamp(mid) => {
                if !self.pane.unclamped.remove(&mid) {
                    self.pane.unclamped.insert(mid);
                }
            }
            Msg::ToggleEditHistory(mid) => {
                self.pane.expanded_history_msg_id =
                    (self.pane.expanded_history_msg_id.as_ref() != Some(&mid)).then_some(mid);
            }
            Msg::HoverEnter(id) => {
                self.quick_open &= self.pane.hovered_msg.as_ref() == Some(&id);
                self.pane.hovered_msg = Some(id);
            }
            Msg::ShowQuickReactions => self.quick_open = true,
            Msg::HoverExit(id) => {
                if self.pane.hovered_msg.as_ref() == Some(&id) {
                    self.pane.hovered_msg = None;
                }
            }
        }
        self.settle()
    }
}
