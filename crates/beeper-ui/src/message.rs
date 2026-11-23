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
use crate::merged;
use crate::reminders::reminder_day;
use crate::reply_line::{reply_line, Segment, Shape};
use crate::style::rem;
use crate::style::{
    bold, bordered_box, bordered_btn, btn_style, clamp, fill_box, fw, hex, ic, icon, med, mix, pane_card, pick,
    scroll_style, semi, stable_bar, text_width, txt, Bare, CLEAR, GUTTER,
};
use crate::threads;
use crate::time::{clock, day_label, day_num, day_of, format_short_ts, locale_string, today};
use crate::timeline;
use crate::toasts::Toast;
use crate::units::human_size;
use crate::view_once;
use crate::windows;
use crate::{
    anim, badged, brightness, can_thread, chat_theme, commands, copy_text, delete_dialog, distance, downloads_dir,
    ease_out, edit_open, editable, enter_key, feedback_btn, file_icon, fmt_phone, gallery, handle, has_older,
    hotkey_caps, hotkey_text, lift, locate, member_name, membership, menu_item, menu_pitch, menu_rule, network_bot,
    pending, pill_btn, portal, profile_link, reply_root, reply_root_with, safe_file_name, send_rise, sender_label,
    short_title, still_sending_at, stored, tinted, unavailable, unused_path, video, vlist, BeeperGuiApp, El, PillLook,
    UiMessage, ASK_DELETE_MESSAGE, BEEPER_ICON, BOOKMARK_ICON, BUBBLE_PAD_X, BUBBLE_PAD_Y, CHEVRON_DOWN,
    CHEVRON_FORWARD, CHEVRON_RIGHT, COMPOSER_ICONS, ERROR_OUTLINE, ISSUES_URL, MESSAGE_MENU_W, ORANGE, OUTLINE_CLOCK,
    POLL_CHECK, POLL_ENDED, TIMER, TRASH,
};
use beeper_core::config::{millis_from_rfc3339, now_unix_millis};
use beeper_core::matrix::{Pressed, ROOM_MENTION};
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
    border, mouse, Bottom, Center, Color, Fill, Length, Padding, Pixels, Point, Right, Shadow, Size, Task, Theme,
    Vector,
};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

/// The name of an attachment, or Desktop's "Attachment" for one that a bridge sent without a file name or body.
pub(crate) fn attachment_name(a: &Attachment) -> String {
    a.file_name.clone().unwrap_or_else(|| tr("Attachment").into())
}

/// The `host` of a URL (`Xp`), with its port when it is not the default one. A URL without a host has none.
fn url_host(url: &str) -> Option<String> {
    let url = url::Url::parse(url).ok()?;
    let host = url.host_str().filter(|h| !h.is_empty())?;
    Some(url.port().map_or_else(|| host.to_string(), |port| format!("{host}:{port}")))
}

/// The links of a block's spans that show their host in a tooltip, like Desktop's `ExtLink` with `showHostOnHover`:
/// all that linkify finds in plain text, and in HTML an anchor whose text is not a URL of its host. The first span
/// of a link decides, as Desktop looks at the first text of the anchor. A `matrix.to` link has none.
fn tipped_links(spans: &[iced::widget::text::Span<'_, String>], html: bool) -> HashSet<String> {
    let mut seen = HashSet::new();
    (spans.iter())
        .filter_map(|s| Some((s.link.as_ref()?, &s.text)))
        .filter(|(link, _)| seen.insert(link.as_str()))
        .filter(|(link, text)| {
            let host = url_host(link);
            host.is_some() && !link.starts_with("https://matrix.to/#/") && (!html || url_host(text) != host)
        })
        .map(|(link, _)| link.clone())
        .collect()
}

/// Desktop's `filter: brightness(0.8)` of the attachments of a selected message (`.message-attachments-container`),
/// as 20% black over their 17 px corners.
/// The user ID of a `matrix.to` mention link.
fn mentioned_user(link: &str) -> Option<String> {
    link.strip_prefix("https://matrix.to/#/").and_then(beeper_core::codec::url_decode).filter(|id| id.starts_with('@'))
}

/// Desktop's `MessageMention`: a user mention shows "@" and the text of its link without the "@" of the text.
fn at_mentions(runs: &mut [beeper_core::codec::Run]) {
    let mut prev = None;
    for r in runs {
        if r.link != prev && r.link.as_deref().and_then(mentioned_user).is_some() {
            r.text = format!("@{}", r.text.strip_prefix('@').unwrap_or(&r.text));
        }
        prev.clone_from(&r.link);
    }
}

fn dimmed<'a>(content: El<'a>, selected: bool) -> El<'a> {
    let shade = selected
        .then(|| container(Space::new()).width(Fill).height(Fill).style(fill_box(Color::BLACK.scale_alpha(0.2), 17.0)));
    stack![content].push(shade).into()
}

/// Desktop's `EnhancedImg` button on an animation that does not autoplay: play, or pause while it plays, which shows
/// only while the pointer is on the image (`.video-idle-play-button`, a 36 px `#efeff0` circle with a `#202124`
/// icon in the middle).
pub(crate) fn gif_toggle<'a>(gif: El<'a>, playing: bool, toggle: Msg) -> El<'a> {
    let (bg, fg) = (hex(0xefeff0), hex(0x202124));
    let glyph = tinted(&crate::voice::ICONS[usize::from(playing)], 20.0, fg);
    let button = button(container(glyph).center(36))
        .padding(0)
        .style(btn_style([bg; 3], [fg; 3], 18.0))
        .on_press(UiMessage::Message(toggle));
    let layer = container(button).center(Fill);
    match playing {
        true => iced::widget::hover(gif, layer),
        false => stack![gif, layer].into(),
    }
}

/// Desktop's `AJ`, which joins the ID of a message and the index of an earlier version in an edit history row.
const EDIT_HISTORY: &str = "::edit-history::";

/// Desktop's `yg`: date-fns `formatDistanceStrict` with its units cut short, as "45s", "5m" or "2mo".
fn short_distance(ms: u64) -> String {
    let long = timer_distance(ms);
    let (n, unit) = long.split_once(' ').expect("a count and a unit");
    let unit = match unit.trim_end_matches('s') {
        "second" => "s",
        "minute" => "m",
        "hour" => "h",
        "day" => "d",
        "month" => "mo",
        _ => "y",
    };
    format!("{n}{unit}")
}

/// The HTML elements that keep Desktop from balancing the lines of a message (`MessageText`).
const BLOCK_TAGS: [&str; 15] =
    ["pre", "blockquote", "table", "ol", "ul", "details", "hr", "p>", "p ", "h1", "h2", "h3", "h4", "h5", "h6"];

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
pub(crate) fn raised(
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

/// Desktop's `.message-contents blockquote` around the runs of blocks deeper than `level`: a 4 px bar and a tint of
/// the text color `ink`, a box in a box for each level.
fn quotes<'a>(items: Vec<(u8, El<'a>)>, level: u8, ink: Color) -> Vec<El<'a>> {
    let mut out: Vec<El<'a>> = Vec::new();
    let mut items = items.into_iter().peekable();
    while let Some((depth, item)) = items.next() {
        if depth <= level {
            out.push(item);
            continue;
        }
        let mut run = vec![(depth, item)];
        while let Some(next) = items.next_if(|(d, _)| *d > level) {
            run.push(next);
        }
        let bar = container(Space::new().width(4.0).height(Fill)).style(fill_box(Color { a: 0.2, ..ink }, 0.0));
        let inner = container(Column::with_children(quotes(run, level + 1, ink))).padding(Padding::from([4.5, 13.0]));
        out.push(
            container(row![bar, inner].height(Length::Shrink)).style(fill_box(Color { a: 0.1, ..ink }, 0.0)).into(),
        );
    }
    out
}

/// Desktop's `.message-contents h1` to `h6`: the title and body type sizes, at 590 for `h1` and 510 for the rest.
fn heading<'a>(
    spans: Vec<iced::advanced::text::Span<'a, String>>,
    level: u8,
) -> Vec<iced::advanced::text::Span<'a, String>> {
    let (size, line, weight) = match level {
        0 => return spans,
        1 => (18.0, 26.0, Weight::Semibold),
        2 => (16.0, 20.0, Weight::Medium),
        3 => (15.0, 20.0, Weight::Medium),
        _ => (14.0, 18.0, Weight::Medium),
    };
    let line = iced::widget::text::LineHeight::Absolute(rem(line).into());
    spans
        .into_iter()
        .map(|s| {
            let font = s.font.unwrap_or_default();
            s.size(rem(size)).line_height(line).font(iced::Font { weight, ..font })
        })
        .collect()
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

/// Desktop's click on a reaction pill of a message or a gallery: it removes your reactions of the group from all the
/// messages that have them, or adds the group's reaction to all the messages that have the group. An emoji it adds
/// has no skin tone, or your own when the network takes any emoji (`canReactWithAnyEmoji`).
fn toggle_reaction(st: &BeeperStore, chat: &Chat, ms: &[&Message], group: &str) -> UiMessage {
    let in_group = |r: &&Reaction| reaction_group(&r.reaction_key) == group;
    let own = |r: &&Reaction| in_group(r) && st.is_me(&r.participant_id);
    let mine = ms.iter().any(|m| m.reactions.iter().any(|r| own(&r)));
    let toggle =
        |m: &Message, key: &str| UiMessage::Message(Msg::ToggleReaction(chat.id.clone(), m.id.clone(), key.into()));
    // Desktop's `Dte`, then `D3e` in the own tone.
    let add = |key: &str| match crate::emoji::is_emoji(key) {
        true => {
            let plain: String = key.chars().filter(|c| !('\u{1f3fb}'..='\u{1f3ff}').contains(c)).collect();
            let any = chat.messages.react && chat.allowed_reactions.is_empty();
            any.then(|| crate::emoji::with_skin_tone(&plain, st.skin_tone)).flatten().unwrap_or(plain)
        }
        false => key.to_string(),
    };
    let key = ms.iter().flat_map(|m| &m.reactions).find(in_group).map(|r| add(&r.reaction_key));
    let toggles: Vec<UiMessage> = ms
        .iter()
        .flat_map(|m| match mine {
            true => m.reactions.iter().filter(own).map(|r| toggle(m, &r.reaction_key)).collect::<Vec<_>>(),
            false => {
                m.reactions.iter().find(in_group).zip(key.as_deref()).map(|(_, k)| toggle(m, k)).into_iter().collect()
            }
        })
        .collect();
    match <[UiMessage; 1]>::try_from(toggles) {
        Ok([one]) => one,
        Err(many) => UiMessage::Commands(many),
    }
}

/// Desktop's `ja`: a reaction key without skin tones or variation selectors, which makes the group of a pill.
fn reaction_group(key: &str) -> String {
    key.chars().filter(|c| !('\u{1f3fb}'..='\u{1f3ff}').contains(c) && !matches!(c, '\u{fe0e}' | '\u{fe0f}')).collect()
}

/// Desktop's `bL`: the reactions of messages `ms` by group, in the order each group first appears.
fn reaction_groups<'a>(ms: &[&'a Message]) -> Vec<(String, Vec<&'a Reaction>)> {
    let mut groups: Vec<(String, Vec<&Reaction>)> = Vec::new();
    for r in ms.iter().flat_map(|m| &m.reactions).filter(|r| !r.reaction_key.is_empty()) {
        let group = reaction_group(&r.reaction_key);
        match groups.iter_mut().find(|(g, _)| *g == group) {
            Some((_, rs)) => rs.push(r),
            None => groups.push((group, vec![r])),
        }
    }
    groups
}

/// Desktop's `gapBelow` (`S4t`) of a message: 12 px before a message of another member chat of a merged chat
/// (`isNextDifferentThread`), else by the time to the next message.
pub(crate) fn gap_below(m: &Message, next: &Message) -> f32 {
    let ms = |m: &Message| millis_from_rfc3339(&m.timestamp).expect("message timestamp");
    match (ms(next) - ms(m)).abs() / 60_000 {
        _ if m.chat_id != next.chat_id => 12.0,
        _ if day_num(&m.timestamp) != day_num(&next.timestamp) => 0.0,
        241.. => 26.0,
        6.. => 13.0,
        2.. => 4.0,
        _ if m.sender_id == next.sender_id => 2.0,
        _ => 4.0,
    }
}

/// What Desktop's `C4t` knows of the neighbours of a message in its reply chain, for its `replyLineSegments`.
#[derive(Clone, Copy, Default)]
struct Chain {
    /// `N` and `j`: the message before it and the one after it are in its reply chain (`uR`).
    prev: bool,
    next: bool,
    /// The next message is your own, and it has the same sender.
    next_mine: bool,
    next_same: bool,
    /// The next message is on another day, so no line goes on through a gap below.
    next_day: bool,
}

/// Desktop's `replyLineSegments`, by where they go.
#[derive(Debug, Default, PartialEq)]
struct Segments {
    /// `along-linked-message`, beside the quote.
    quote: Option<Segment>,
    /// `along-message-body` of your own message, or `along-placeholder-sender-avatar`.
    body: Option<Segment>,
    /// `above-sender-avatar`.
    above_face: Option<Segment>,
    /// `along-message-footer`, through the gap below too.
    footer: Option<Segment>,
}

impl Chain {
    /// Desktop's `replyLineSegments` of a message that has a quote (`linked`, `T`), shows it (`quote`, `U`) of your
    /// own message or not, and Desktop's `showSenderImg` (`Q`) from whether the sender changes (`last`).
    fn segments(self, linked: bool, quote: Option<bool>, mine: bool, group: bool, last: bool) -> (Segments, bool) {
        use crate::reply_line::Shape::*;
        let Chain { prev: n, next: j, next_mine, next_same, .. } = self;
        let (u, z) = (quote.is_some(), quote == Some(true));
        let (start, end) = (!n && j, !j && (n || u));
        let can_show = group && !mine;
        let face = can_show && (last || end);
        // Desktop's `VJ`.
        let shape = |mine: bool| match () {
            _ if end => End,
            _ if start && !u => Start,
            _ if mine && !next_mine => Loop,
            _ => Line,
        };
        let body = (mine && (n || j || u)).then(|| Segment::new(shape(true), if u { !z } else { !n }, !j));
        let placeholder = (can_show && !face && (linked || j)).then(|| Segment::new(shape(false), u && !z, false));
        let footer =
            j.then(|| Segment::new(Line, body.is_none() && placeholder.is_none(), !group && (!mine || !next_same)));
        let segments = Segments {
            quote: (u && z).then(|| Segment::new(Start, false, !group && !mine)),
            body: body.or(placeholder),
            above_face: (face && (u || n)).then(|| Segment::new(Line, u && !z, true)),
            footer,
        };
        (segments, face)
    }
}

/// Desktop's `xgt`: a count over 999 in thousands or millions. JavaScript's `toFixed` rounds an exact tie up, where
/// Rust rounds it to even.
fn short_count(n: usize) -> String {
    match n {
        ..1000 => n.to_string(),
        1000..1_000_000 => format!("{:.1}K", (n + usize::from(n % 500 == 250)) as f64 / 1e3),
        _ => format!("{:.2}M", (n + usize::from(n % 250_000 == 125_000)) as f64 / 1e6),
    }
}

/// The tooltip of Desktop's `BrandTypingIndicator`, which Desktop does not translate.
fn typing_tip(names: &[String]) -> String {
    match names {
        [one] => format!("{one} is typing…"),
        [rest @ .., last] if names.len() <= 6 => format!("{} and {last} are typing…", rest.join(", ")),
        _ => {
            let more = names.len() - 6;
            let verb = if more > 1 { "s are" } else { " is" };
            format!("{} and {more} other{verb} typing…", names[..6].join(", "))
        }
    }
}

/// Desktop's `ActionButtonIcon`: a Material icon drawn 16 px at its 1.2 scale in the 14 px icon box of a `Button`,
/// which shows the middle 14/19.2 of it.
static BUTTON_ICONS: std::sync::LazyLock<HashMap<&str, svg::Handle>> =
    std::sync::LazyLock::new(|| crate::material_icons(14.0 / 19.2));

/// The rays of Desktop's `Button-module__spinner` mask.
const SPINNER_RAYS: &str = "M8.33333 17.5V14.1667C8.33333 13.7064 8.70643 13.3333 9.16667 13.3333C9.6269 13.3333 10 13.7064 10 14.1667V17.5C10 17.9602 9.6269 18.3333 9.16667 18.3333C8.70643 18.3333 8.33333 17.9602 8.33333 17.5ZM5.03581 12.1191C5.36124 11.7937 5.88876 11.7937 6.21419 12.1191C6.53963 12.4446 6.53963 12.9721 6.21419 13.2975L3.85742 15.6543C3.53198 15.9797 3.00447 15.9797 2.67904 15.6543C2.3536 15.3289 2.3536 14.8013 2.67904 14.4759L5.03581 12.1191ZM12.1191 12.1191C12.4242 11.814 12.9067 11.7952 13.234 12.0622L13.2975 12.1191L15.6543 14.4759L15.7121 14.5394C15.9789 14.8667 15.9594 15.3492 15.6543 15.6543C15.3492 15.9594 14.8667 15.9789 14.5394 15.7121L14.4759 15.6543L12.1191 13.2975L12.0622 13.234C11.7952 12.9067 11.814 12.4242 12.1191 12.1191ZM4.16667 8.33333C4.6269 8.33333 5 8.70643 5 9.16667C5 9.6269 4.6269 10 4.16667 10H0.833333C0.373096 10 0 9.6269 0 9.16667C0 8.70643 0.373096 8.33333 0.833333 8.33333H4.16667ZM17.5 8.33333C17.9602 8.33333 18.3333 8.70643 18.3333 9.16667C18.3333 9.6269 17.9602 10 17.5 10H14.1667C13.7064 10 13.3333 9.6269 13.3333 9.16667C13.3333 8.70643 13.7064 8.33333 14.1667 8.33333H17.5ZM2.67904 2.74414C2.98419 2.43909 3.46667 2.4201 3.79394 2.68717L3.85742 2.74414L6.21419 5.10091L6.27116 5.16439C6.53819 5.49166 6.51923 5.97416 6.21419 6.2793C5.90919 6.5843 5.42659 6.60378 5.09928 6.33708L5.03581 6.2793L2.67904 3.92253L2.62126 3.85905C2.35451 3.53174 2.37402 3.04916 2.67904 2.74414ZM14.4759 2.74414C14.8013 2.41874 15.3289 2.4188 15.6543 2.74414C15.9797 3.06958 15.9797 3.59709 15.6543 3.92253L13.2975 6.2793C12.9721 6.60473 12.4446 6.60473 12.1191 6.2793C11.7938 5.95385 11.7937 5.42631 12.1191 5.10091L14.4759 2.74414ZM8.33333 4.16667V0.833333C8.33333 0.373096 8.70643 0 9.16667 0C9.6269 0 10 0.373096 10 0.833333V4.16667C10 4.6269 9.6269 5 9.16667 5C8.70643 5 8.33333 4.6269 8.33333 4.16667Z";

/// Desktop's `Button-module__spinner`: the rays under a conic gradient from 15% of the color at the top, clockwise to
/// all of it. resvg has no conic gradient, so each 10° wedge of the rays takes the opacity of its middle.
static BUTTON_SPINNER: std::sync::LazyLock<svg::Handle> = std::sync::LazyLock::new(|| {
    let (c, tau) = (18.3333 / 2.0, std::f32::consts::TAU);
    let at = |k: u8| {
        let a = f32::from(k) * tau / 36.0;
        format!("{} {}", c + 20.0 * a.sin(), c - 20.0 * a.cos())
    };
    let wedges: String = (0..36)
        .map(|k| {
            let alpha = 0.15 + 0.85 * (f32::from(k) + 0.5) / 36.0;
            format!(
                r##"<clipPath id="w{k}"><path d="M{c} {c}L{}L{}Z"/></clipPath><use xlink:href="#r" clip-path="url(#w{k})" fill-opacity="{alpha}"/>"##,
                at(k),
                at(k + 1)
            )
        })
        .collect();
    let doc = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" viewBox="0 0 18.3333 18.3333"><defs><path id="r" d="{SPINNER_RAYS}"/></defs>{wedges}</svg>"##
    );
    svg::Handle::from_memory(doc.into_bytes())
});

/// Desktop's `Button-module__spinner`, `size` px, which turns a twelfth at a time, once in 0.8 s.
fn button_spinner<'a>(size: f32, color: Color) -> El<'a> {
    let step = (crate::login::EPOCH.elapsed().as_secs_f32() / 0.8 * 12.0).floor();
    svg(BUTTON_SPINNER.clone())
        .width(size)
        .height(size)
        .opacity(color.a)
        .rotation(iced::Rotation::Solid(iced::Radians(step / 12.0 * std::f32::consts::TAU)))
        .style(move |_, _| svg::Style { color: Some(color) })
        .into()
}

/// Desktop's `ActionButtons` button: a rounded `secondary` `Button` of the `md` size, 32 px high, on
/// `--semantic-colors-bg-secondary`, with its icon, its label in 13 px semibold and a chevron on a link, 6 px apart.
/// Without `press` its callback runs: disabled at half opacity on the `bg-disabled` fill, with the spinner in place
/// of its content.
fn action_button<'a>(p: &crate::style::Palette, b: &ActionButton, press: Option<UiMessage>) -> El<'a> {
    let loading = press.is_none();
    let shown = |c: Color| if loading { CLEAR } else { c };
    let icon = b.icon.as_deref().and_then(|n| BUTTON_ICONS.get(n)).map(|h| tinted(h, 14.0, shown(p.main_text)));
    let label = semi(b.label.clone(), 13.0).line_height(1.0).color(shown(p.main_text));
    // Desktop's `chevron-compact-right`, the Material `chevron_forward` of 14 px.
    let chevron = (b.action == ButtonAction::Link).then(|| tinted(&CHEVRON_FORWARD, 14.0, shown(p.tertiary)));
    let content = row![].push(icon).push(label).push(chevron).spacing(6).align_y(Center);
    let (alpha, bg) = if loading { (0.5, p.main_muted) } else { (1.0, p.bg_secondary) };
    let spinner = loading.then(|| container(button_spinner(14.0, p.main_text.scale_alpha(alpha))).center(Fill));
    // The 1 px transparent border of `Button-module__button` adds to its 12 px padding.
    button(stack![container(content).center_y(Fill)].push(spinner))
        .height(32)
        .padding([0, 13])
        .style(move |_, _| button::Style {
            background: Some(bg.scale_alpha(alpha).into()),
            border: border::rounded(9999),
            ..Default::default()
        })
        .on_press_maybe(press)
        .into()
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

/// Whether an image has more pixels than [`crate::gallery::BAKE`] times its box of `px`, so that its baked cut is
/// smaller than the image.
pub(crate) fn larger(h: &image::Handle, (w, ht): (u32, u32)) -> bool {
    let image::Handle::Rgba { width, height, .. } = h else { return false };
    let scale = crate::gallery::BAKE;
    *width as f32 > scale * w as f32 && *height as f32 > scale * ht as f32
}

/// Desktop's box of a sticker of known size: at most 400 px high and `share` of the window wide, never upscaled, and
/// never cropped (`object-fit: contain`), however tall or wide it is.
pub(crate) fn sticker_box(window_w: f32, (w, h): (f32, f32), share: f32) -> (f32, f32, bool) {
    if w <= 0.0 || h <= 0.0 {
        return media_box(window_w, None, share);
    }
    let k = 1f32.min(400.0 / h).min((share * window_w).max(1.0) / w);
    (w * k, h * k, false)
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
            && !jumbo(&emoji_text(n))
            && format_short_ts(&n.timestamp, h24) == format_short_ts(&m.timestamp, h24)
    })
}

/// The states of Desktop's `AttachmentFileRow`: its media downloads, needs the user to ask for it, or is ready.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum FileState {
    Fetching,
    Unfetched,
    Ready,
}

/// Desktop's `FileRowSpinner`: 12 spokes that turn by a step each twelfth of a second, 24 px.
pub(crate) fn file_row_spinner<'a>(color: Color) -> El<'a> {
    let step = (crate::login::EPOCH.elapsed().as_secs_f32() * 12.0).floor() % 12.0;
    svg(FILE_ROW_SPINNER.clone())
        .width(24)
        .height(24)
        .rotation(iced::Rotation::Floating(iced::Radians(step * std::f32::consts::TAU / 12.0)))
        .style(move |_, _| svg::Style { color: Some(color) })
        .into()
}

static FILE_ROW_SPINNER: std::sync::LazyLock<svg::Handle> = std::sync::LazyLock::new(|| {
    let spokes: String = (0..12)
        .map(|i| {
            let (turn, opacity) = (i * 30, 0.25 + 0.75 * i as f32 / 11.0);
            format!(r#"<rect x="11" y="3" width="2" height="5.5" rx="1" opacity="{opacity}" transform="rotate({turn} 12 12)"/>"#)
        })
        .collect();
    let svg = format!(r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="#000">{spokes}</svg>"##);
    svg::Handle::from_memory(svg.into_bytes())
});

/// Desktop's `DO`: when the message disappears, or `None` without a timer.
pub(crate) fn disappears(chat: &Chat, m: &Message) -> Option<String> {
    let t = m.disappearing.as_ref()?;
    Some(match m.expiry(now_unix_millis())? {
        Countdown::Left(left) => trf("Disappears in %s", &[&distance(i64::try_from(left).unwrap_or(i64::MAX))]),
        Countdown::Unread => {
            let recipient = t.kind.as_deref() == Some("after_read_by_recipient") && m.is_sender == Some(true);
            let label = match (recipient, chat.chat_type == "group") {
                (true, true) => "Disappears %s after someone in the group reads it",
                (true, false) => "Disappears %s after the recipient reads it",
                (false, _) => "Disappears %s after you mark it read",
            };
            trf(label, &[&timer_distance(t.timer)])
        }
    })
}

/// Desktop's `gx`: an attachment that is a contact card.
fn is_vcard(a: &Attachment) -> bool {
    let name = a.file_name.as_deref().unwrap_or_default().to_lowercase();
    name.ends_with(".vcf") || a.mime_type.as_deref().is_some_and(|m| m.eq_ignore_ascii_case("text/x-vcard"))
}

/// Desktop's `BrandTypingIndicator` `transition: { duration: 0.32 }`.
pub(crate) const TYPING_FADE: std::time::Duration = std::time::Duration::from_millis(320);

/// Desktop's `.reply-thread-messages` `transition-duration: 0.6s`, the longest part of the reply chain fade-in.
pub(crate) const REPLIES_RISE: std::time::Duration = std::time::Duration::from_millis(600);

/// The reply chain overlay `t` seconds after it opened, as its opacity and how far its messages are below their place:
/// Desktop's `.reply-thread` fades in over 0.2 s and its messages rise 10 px over 0.6 s, both on
/// `cubic-bezier(.165, .84, .44, 1)`.
pub(crate) fn replies_reveal(t: f32) -> (f32, f32) {
    let curve = |d: f32| crate::style::cubic_bezier((t / d).clamp(0.0, 1.0), [0.165, 0.84, 0.44, 1.0]);
    (curve(0.2), 10.0 * (1.0 - curve(REPLIES_RISE.as_secs_f32())))
}

/// The time from one flyover item to the next: the item's ease-out is a quarter in after 32 ms of its 0.2 s.
const FLYOVER_STAGGER: f32 = 0.032;

/// Desktop's `pm` text: the formatted body with each custom emoji (`data-mx-emoticon`) as 📟 when it has one, else
/// the text, so a message of custom emoji is a jumbo emoji too.
fn emoji_text(m: &Message) -> String {
    match m.html.as_deref().filter(|h| h.contains("data-mx-emoticon")) {
        Some(html) => {
            let runs = beeper_core::codec::html_blocks(html).into_iter().flat_map(|b| b.runs);
            runs.map(|r| if r.emoticon.is_some() { "📟".into() } else { r.text }).collect()
        }
        None => m.text.clone().unwrap_or_default(),
    }
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
/// The parts of `text` around each "@room", and whether each is one.
fn room_parts(text: &str) -> Vec<(&str, bool)> {
    let mut parts = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find(ROOM_MENTION) {
        parts.extend((i > 0).then(|| (&rest[..i], false)));
        parts.push((&rest[i..i + ROOM_MENTION.len()], true));
        rest = &rest[i + ROOM_MENTION.len()..];
    }
    parts.extend((!rest.is_empty()).then_some((rest, false)));
    parts
}

/// HTML's `dir="auto"`: whether the first strong character of `text` is of a right-to-left script.
fn right_to_left(text: &str) -> bool {
    let rtl = |c: char| matches!(c as u32, 0x0590..=0x08FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFF | 0x10800..=0x10FFF | 0x1E800..=0x1EFFF);
    text.chars().find(|c| c.is_alphabetic()).is_some_and(rtl)
}

/// How far under the top of the timeline the date of the day at its top floats, under the header card.
const STICKY_DATE: f32 = 80.0;

/// The container of the unread divider, which a chat scrolls to when it opens.
const UNREAD_DIVIDER: &str = "unread-divider";

/// Desktop's `updateUnreadDividerMessageSortKey` and `firstUnreadMessageIDIn` when a chat opens: the message after its
/// read position, below one at or before it, when a message is after it. The unread marks of the messages place it
/// when the loaded messages do not reach the read position.
pub(crate) fn divider_at(st: &BeeperStore, chat: &Chat, msgs: &[&Message]) -> Option<String> {
    // Desktop's `firstUnreadMessageIDIn` of a merged chat: the first message of others after the read position of its
    // unread member chat.
    if chat.merge.is_some() {
        let unread = |m: &&&Message| {
            let c = st.chat(&m.chat_id);
            let read = c.last_read_message_sort_key.as_ref();
            c.is_unread() && m.is_sender != Some(true) && read.is_some_and(|r| m.sort_key > *r)
        };
        return msgs.iter().find(unread).map(|m| m.id.clone());
    }
    let last = msgs.last().map(|m| &m.sort_key);
    let read = chat.last_read_message_sort_key.as_ref().filter(|k| Some(*k) < last);
    let after = read.and_then(|r| msgs.windows(2).position(|w| w[0].sort_key <= *r && w[1].sort_key > *r));
    let at = after.map(|i| i + 1);
    // The unread marks of the messages count only while the chat is unread: a read through the bottom of the
    // timeline leaves them, and leaving the chat reads nothing.
    let marked = |m: &&&Message| m.is_deleted != Some(true) && m.is_unread == Some(true);
    let at = at.or_else(|| msgs.iter().position(|m| chat.is_unread() && marked(&m)));
    at.map(|i| msgs[i].id.clone())
}

pub(crate) struct Rows {
    /// The messages, as indices of `messages`.
    pub(crate) msgs: Vec<usize>,
    /// The first message of the membership event run of each message.
    run_start: Vec<usize>,
    last_live: Option<usize>,
    pub(crate) any_pending: bool,
    /// For each person other than you and the sender, the newest message that the person saw before a reply of the
    /// person, and the time.
    last_seen: HashMap<String, (usize, String)>,
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
    /// The message IDs that the reply chains look up outside the chats of the messages.
    links: Vec<String>,
}

/// What the rows of a pane read from the store, so that the pane keeps them while it stays the same: the timelines of
/// the chats of the rows, the lookups of their reply chains, and the values that pick and mark their messages.
#[derive(PartialEq)]
pub(crate) struct RowsKey {
    /// The store, and its count of moves of message indices.
    store: (u64, u64),
    /// The chat and the thread root of the pane, and the member chats of a merged chat.
    pane: (String, Option<String>, Vec<String>),
    /// The chats of the rows with their timeline revisions, whether they show threads, and where their older messages
    /// load from.
    chats: Vec<(String, u64, bool, Older)>,
    /// Whether the thread root of a thread pane is loaded, and its loaded replies.
    thread: Option<(bool, usize)>,
    /// The chat and its timeline revision of each message of `Rows::links`, while it is loaded.
    links: Vec<(String, Option<(String, u64)>)>,
    /// `hideDeleted` and `membershipEvents`.
    settings: (bool, bool),
    /// The Matrix client and the demo, which pick the messages and their pending state, and the sent local messages.
    matrix: (bool, bool, usize),
    /// Your user ID, and the UTC offset of the local time zone, which places the days.
    me: (String, i64),
}

impl RowsKey {
    fn of(st: &BeeperStore, pane: &crate::Pane, chat: &Chat, chats: &[&str], links: &[String]) -> Self {
        let rev = |c: &str| st.timeline_rev(c);
        let members = st.members(st.chat_idx(&chat.id)).into_iter().map(|i| st.chats[i].id.clone()).collect();
        let chats = chats.iter().map(|&c| {
            let (threads, older) =
                st.find_chat(c).map_or((false, Older::Unknown), |c| (st.threaded(c), c.older.clone()));
            (c.to_string(), rev(c), threads, older)
        });
        let thread = pane.thread_root.as_deref().map(|r| (st.loaded(r).is_some(), st.thread_replies(r).count()));
        let link = |r: &String| (r.clone(), st.loaded(r).map(|m| (m.chat_id.clone(), rev(&m.chat_id))));
        let now = now_unix_millis();
        Self {
            store: (st.instance, st.message_moves()),
            pane: (chat.id.clone(), pane.thread_root.clone(), members),
            chats: chats.collect(),
            thread,
            links: links.iter().map(link).collect(),
            settings: (st.settings.on("hideDeleted"), st.settings.on("membershipEvents")),
            matrix: (st.matrix.is_some(), st.config.demo, st.sent.len()),
            me: (st.me().id.clone(), beeper_core::config::local_millis(now) - now),
        }
    }
}

/// The first message of the membership event run of each message: Desktop's `MembershipGrouper` groups the member
/// events of a day in a row.
pub(crate) fn member_runs(msgs: &[&Message]) -> Vec<usize> {
    let mut run_start: Vec<usize> = (0..msgs.len()).collect();
    for i in 1..msgs.len() {
        let (a, b) = (msgs[i - 1], msgs[i]);
        if membership(a) && membership(b) && day_num(&a.timestamp) == day_num(&b.timestamp) {
            run_start[i] = run_start[i - 1];
        }
    }
    run_start
}

/// The size of the membership run that starts at `start`.
fn run_size(runs: &[usize], start: usize) -> usize {
    runs[start..].iter().take_while(|&&r| r == start).count()
}

impl Rows {
    fn of(st: &BeeperStore, msgs: &[&Message]) -> Self {
        let n = msgs.len();
        let run_start = member_runs(msgs);
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
        let chats: HashSet<&str> = msgs.iter().map(|m| m.chat_id.as_str()).collect();
        let mut links = Vec::new();
        let mut look = |r: &str, m: Option<&Message>| {
            if m.is_none_or(|m| !chats.contains(m.chat_id.as_str())) {
                links.push(r.to_string());
            }
        };
        let roots: Vec<&str> = msgs.iter().map(|m| reply_root_with(st, m, &mut look)).collect();
        links.sort_unstable();
        links.dedup();
        let mut later: HashMap<&str, usize> = HashMap::new();
        let mut counts = vec![0; n];
        let day: Vec<i64> = msgs.iter().map(|m| day_num(&m.timestamp)).collect();
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
            chained: (0..n).map(|i| i > 0 && roots[i - 1] == roots[i]).collect(),
            gallery: gallery::starts(st, msgs, &counts),
            counts,
            keys: msgs.iter().map(|m| vlist::key(&m.id)).collect(),
            days: (0..n).filter(|&i| i == 0 || day[i - 1] != day[i]).collect(),
            links,
        }
    }
}

/// Desktop's `mhe`: Bookmark for a sent message that is not deleted, once the bookmarks room is read. The demo has no
/// bookmarks room. Every chat here is on a bridge of Beeper's, which has bookmarks (`supportsBookmarks`).
fn bookmarkable(st: &BeeperStore, m: &Message) -> bool {
    m.is_deleted != Some(true) && !pending(st, m) && (st.bookmarks_room.is_some() || st.config.demo)
}

/// Desktop's Mark as Unread from `m`: in a bridged chat (`canMarkUnreadFromMessage`), and in incognito mode, which
/// keeps the unread mark where it is, not from a message after the mark (`ue`).
fn unread_from(st: &BeeperStore, m: &Message) -> bool {
    let read = st.chat(&m.chat_id).last_read_message_sort_key.as_ref();
    m.account_id != "hungryserv" && (!st.incognito() || read.is_none_or(|r| m.sort_key <= *r))
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

    /// Desktop's `BrandTypingIndicator` shows and hides in 0.32 s, and shows at once while the timeline is not at its
    /// bottom (`skipEntryAnimation`).
    pub(crate) fn typing_changed(&mut self, chat: &str) {
        let ids = self.store.read().typing.get(chat).cloned().unwrap_or_default();
        let now = std::time::Instant::now();
        let up =
            self.panes().any(|p| p.selected_chat_id.as_deref() == Some(chat) && p.scrolled_up.is_some_and(|s| s.0));
        match self.typing_shown.get_mut(chat) {
            Some(t) if t.1 && ids.is_empty() => (t.1, t.2) = (false, now),
            Some(t) if t.1 => t.0 = ids,
            _ if ids.is_empty() => {}
            _ => {
                let at = if up { now.checked_sub(TYPING_FADE).unwrap_or(now) } else { now };
                self.typing_shown.insert(chat.to_string(), (ids, true, at));
            }
        }
    }

    /// Who types in `chat`, also while the indicator hides, and how far it shows, with framer-motion's `easeInOut`.
    fn typing_now<'s>(&'s self, st: &'s BeeperStore, chat: &str) -> Option<(&'s [String], f32)> {
        let typing = st.typing.get(chat).filter(|ids| !ids.is_empty());
        let Some((ids, on, at)) = self.typing_shown.get(chat) else { return typing.map(|ids| (&ids[..], 1.0)) };
        let t = at.elapsed().as_secs_f32() / TYPING_FADE.as_secs_f32();
        let v = if t < 1.0 { crate::style::cubic_bezier(t, [0.42, 0.0, 0.58, 1.0]) } else { 1.0 };
        let v = if *on { v } else { 1.0 - v };
        (v > 0.0).then(|| (&typing.unwrap_or(ids)[..], v))
    }

    /// Desktop's `BrandTypingIndicator` at `v` of its height and opacity. A group shows the first 4 faces in 34 px,
    /// each `min(20, (34 + 10(n - 1)) / n)` px and 10 px under the next one, which a 2 px ring cuts out (`ode`), each
    /// with its name as a tooltip. The bubble's tooltip says who types.
    fn typing_bubble<'a>(&self, st: &BeeperStore, chat: &Chat, ids: &[String], v: f32) -> El<'a> {
        let p = &self.pal;
        let ink = p.text.scale_alpha(v);
        let dot = |delay| anim::dot(6.0, ink, self.started, delay, |t| 0.2 + 0.2 * (1.0 - (2.0 * t - 1.0).abs()));
        let bubble = container(row![dot(0), dot(340), dot(670)].spacing(4))
            .padding([12, 11])
            .style(fill_box(p.received.scale_alpha(v), 16.0));
        let users: Vec<&User> =
            ids.iter().filter_map(|id| chat.participants.items.iter().find(|u| u.id == *id)).collect();
        let names: Vec<String> = users.iter().map(|u| member_name(u)).collect();
        let bubble: El<'a> = match names.is_empty() {
            true => bubble.into(),
            false => self.tip(bubble.into(), typing_tip(&names), vec![], tooltip::Position::Top).into(),
        };
        let faces = (chat.chat_type == "group").then(|| {
            let shown = &users[..users.len().min(4)];
            let n = shown.len().max(1) as f32;
            let size = 20f32.min((34.0 + 10.0 * (n - 1.0)) / n);
            let left = (34.0 - (n * size - 10.0 * (n - 1.0))) / 2.0;
            // The ring takes the color of the timeline, which a wallpaper does not have.
            let ring = self.wallpaper_fg(st, chat, false).is_none().then(|| p.card.scale_alpha(v));
            let layers = shown.iter().enumerate().flat_map(|(k, u)| {
                let x = left + k as f32 * (size - 10.0);
                let cut = ring.filter(|_| k > 0).map(|c| -> El<'a> {
                    let disc = container(Space::new()).width(size + 4.0).height(size + 4.0);
                    pin(disc.style(fill_box(c, size / 2.0 + 2.0))).x(x - 2.0).y(-2.0).into()
                });
                let face =
                    self.face_alpha(u.img_url.as_deref(), u.full_name.as_deref().unwrap_or_default(), size, 0.0, v);
                let face = self.tip(face, member_name(u), vec![], tooltip::Position::Top);
                cut.into_iter().chain([pin(face).x(x).into()])
            });
            Stack::with_children(layers).width(34).height(size)
        });
        // framer-motion's `height` and `marginTop: 8` from 0, with the 30 px of the bubble.
        let body = container(row![].push(faces).push(bubble).spacing(8).align_y(Center)).height(30.0 * v).clip(true);
        container(column![Space::new().height(8.0 * v), body]).padding(Padding::ZERO.left(8.0)).into()
    }

    /// Electron's context menu over selected text (electron-context-menu as Desktop sets it up): Search with Google
    /// and Copy.
    fn selection_menu(&self, selection: &str) -> (El<'_>, f32, f32) {
        let p = &self.pal;
        let item = |label: String, msg: UiMessage| {
            menu_item(p, None, label, "", Some(UiMessage::Message(Msg::Menu(Box::new(msg)))))
        };
        let mut search = url::Url::parse("https://www.google.com/search").expect("a valid URL");
        search.query_pairs_mut().append_pair("q", selection);
        let items = column![
            item(tr("Search with Google").into(), UiMessage::OpenUrl(search.into())),
            menu_rule(p),
            item(tr("Copy").into(), UiMessage::Message(Msg::CopyText(selection.to_string()))),
        ];
        let menu = crate::menu_card(p, items, MESSAGE_MENU_W);
        (menu.into(), 8.0 + 2.0 * menu_pitch() + 11.0, 0.0)
    }

    /// Desktop's `Yit` for several selected messages: the items that act on the whole selection.
    fn selected_messages_menu(&self) -> (El<'_>, f32, f32) {
        let p = &self.pal;
        let st = self.store.read();
        let (msgs, _) = self.timeline(&st);
        let picked: Vec<&Message> = msgs.into_iter().filter(|m| self.pane().selected_msgs.contains(&m.id)).collect();
        let n = picked.len();
        let key = |name| hotkey_text(st.settings.hotkey_keys(name));
        let item = |label: String, hotkey: &str, msg: Option<UiMessage>| {
            (
                menu_item(p, None, label, hotkey.to_string(), msg.map(|m| UiMessage::Message(Msg::Menu(Box::new(m))))),
                menu_pitch(),
            )
        };
        let stamp = st.settings.on("copyTimestamps");
        let earliest = picked.first().filter(|m| unread_from(&st, m));
        let mut rows = vec![item(
            trn("Quote reply %d message…", "Quote reply %d messages…", n, &[&n]),
            &key("Message: Quote & Reply"),
            Some(UiMessage::Message(Msg::QuoteSelected)),
        )];
        rows.extend(earliest.map(|m| {
            let unread = UiMessage::Message(Msg::MarkUnreadFromHere(m.chat_id.clone(), m.id.clone()));
            item(tr("Mark as Unread").into(), "", Some(unread))
        }));
        rows.push((menu_rule(p), 11.0));
        rows.push(item(
            tr("Copy").into(),
            if stamp { "" } else { "Ctrl C" },
            Some(UiMessage::Message(Msg::CopySelected(false))),
        ));
        rows.push(item(
            tr("Copy with Timestamp").into(),
            if stamp { "Ctrl C" } else { "" },
            Some(UiMessage::Message(Msg::CopySelected(true))),
        ));
        // Desktop's `canOpen` of the forward menu: no view-once media.
        if !picked.iter().any(|m| m.view_once()) {
            rows.push(item(
                tr("Forward…").into(),
                &key("Message: Forward"),
                Some(UiMessage::RunHotkey("Message: Forward")),
            ));
        }
        let ids: Vec<String> = picked.iter().map(|m| m.id.clone()).collect();
        if delete_dialog(&st, &picked).is_some() {
            rows.push(item(tr("Delete…").into(), "", Some(UiMessage::Message(Msg::AskDelete(ids)))));
        }
        rows.push((menu_rule(p), 11.0));
        rows.push(item(
            tr("Report a Problem").into(),
            &key("Report a Problem"),
            Some(UiMessage::OpenUrl(ISSUES_URL.into())),
        ));
        let (menu, h) = crate::menu_rows(p, rows, MESSAGE_MENU_W);
        (menu.into(), h, 0.0)
    }

    /// The message menu, its height, and the top of the row of its open Remind Me submenu, or else of its Playback
    /// Speed row.
    pub(crate) fn view_message_menu(&self, mid: &str) -> (El<'_>, f32, f32) {
        if let Some(menu) = self.view_gallery_menu(mid) {
            return menu;
        }
        let p = &self.pal;
        if let Some(selection) = &self.pane().menu_selection {
            return self.selection_menu(selection);
        }
        if self.pane().selected_msgs.len() > 1 && self.pane().selected_msgs.iter().any(|s| s == mid) {
            return self.selected_messages_menu();
        }
        let st = self.store.read();
        let m = st.message(mid);
        let (cid, mid, mine) = (m.chat_id.clone(), m.id.clone(), m.is_sender == Some(true));
        let text = m.text.clone().unwrap_or_default();
        // Desktop's items of the clicked link: its address for a `mailto:` one, else its URL.
        let clicked = self.pane().menu_link.as_deref();
        let address = clicked.and_then(|h| h.strip_prefix("mailto:")).map(str::to_string);
        let link = clicked.filter(|h| h.starts_with("https://")).map(str::to_string);
        let entire = link.is_some();
        let rule = || (menu_rule(p), 11.0);
        let key = |name| hotkey_text(st.settings.hotkey_keys(name));
        let item = |label: String, hotkey: &str, msg: Option<UiMessage>| {
            (
                menu_item(p, None, label, hotkey.to_string(), msg.map(|m| UiMessage::Message(Msg::Menu(Box::new(m))))),
                menu_pitch(),
            )
        };
        let bookmark = tr(if m.is_bookmarked == Some(true) { "Remove Bookmark" } else { "Bookmark" });
        let at = trf(if mine { "Sent at %s" } else { "Received at %s" }, &[&locale_string(&m.timestamp)]);
        let timer = disappears(st.chat(&m.chat_id), m);
        let stamp = st.settings.on("copyTimestamps");
        // Desktop's `Hit`: view-once media has nothing to download or copy.
        let files = if m.view_once() { 0 } else { m.attachments.len() };
        // Desktop's `selectedImage`: the image items come with a right click on the image.
        let image = self.pane().menu_image;
        // Desktop's `selectedImageAttachment`: Copy copies the one image of a message without text.
        let only_image = text.is_empty() && files == 1 && m.attachments[0].attachment_type == "img";
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
        // Desktop's video menu takes the place of the message menu for a right click on the video, with Bookmark.
        if let Some(rows) = self.video_menu_rows(&mid, false).filter(|_| self.pane().video_menu) {
            let mark = bookmarkable(&st, m).then(|| {
                [rule(), item(bookmark.into(), "", Some(UiMessage::Message(Msg::ToggleBookmark(mid.clone()))))]
            });
            let rows: Vec<(El, f32)> = rows.into_iter().chain(mark.into_iter().flatten()).collect();
            let (menu, h) = crate::menu_rows(p, rows, MESSAGE_MENU_W);
            return (menu.into(), h, 0.0);
        }
        let chat = st.chat(&cid);
        let (rules, live) = (chat.messages, m.is_deleted != Some(true));
        let others = chat.participants.items.iter().filter(|u| u.is_self != Some(true)).count();
        let sender = chat.participants.items.iter().any(|u| u.id == m.sender_id);
        let privately =
            !mine && sender && others > 1 && m.is_deleted != Some(true) && self.can(&m.account_id, "create_dm");
        // Desktop's `D`: the items that act on a message hide while you edit it.
        let editing = self.pane().editing_message_id.as_ref() == Some(&mid);
        // Desktop's `isPending` (its `pendingMessages`): a send that the homeserver has not taken yet, or that failed,
        // has no Edit, Reply, Reply in thread, Bookmark, Mark as Unread, End Poll, Delete or voice items, and its
        // React is off. Copy, Forward and "Sent at" stay.
        let queued = pending(&st, m);
        // Desktop's detected-data items (phone, address, date, flight) above these come from the macOS-only
        // TextsSwift `detectData`; on Linux it returns nothing, so they never show (App:48518, 96520).
        let before: Vec<(El, f32)> = [
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
            (editable(m, chat) && !editing && !queued).then(|| {
                let edit =
                    edit_open(m, rules).then(|| UiMessage::Composer(composer::Msg::StartEdit(mid.clone(), text)));
                vec![item(tr("Edit…").into(), &key("Message: Edit"), edit)]
            }),
            (rules.reply && live && !editing && !queued).then(|| {
                vec![item(
                    tr("Reply…").into(),
                    &key("Message: Quote & Reply"),
                    Some(UiMessage::Composer(composer::Msg::StartReply(mid.clone()))),
                )]
            }),
            (self.pane().thread_root.is_none() && !editing && !queued && can_thread(&st, chat, m)).then(|| {
                let label = tr("Reply in thread…").into();
                vec![item(label, &key("Show Replies"), Some(UiMessage::Thread(threads::Msg::Show(mid.clone()))))]
            }),
            (self.pane().thread_root.is_none()
                && live
                && !editing
                && st.threaded(chat)
                && st.thread_replies(&mid).next().is_some())
            .then(|| vec![(self.follow_item(&st, chat, &mid), menu_pitch())]),
            privately.then(|| {
                vec![item(
                    tr("Reply Privately…").into(),
                    "",
                    Some(UiMessage::ChatList(chat_list::Msg::ReplyPrivately(mid.clone()))),
                )]
            }),
            (rules.react && !editing).then(|| {
                let react = (m.reactable() && !queued).then(|| UiMessage::Message(Msg::ReactOn(mid.clone())));
                vec![item(tr("React…").into(), "", react)]
            }),
            bookmarkable(&st, m)
                .then(|| vec![item(bookmark.into(), "", Some(UiMessage::Message(Msg::ToggleBookmark(mid.clone()))))]),
        ]
        .into_iter()
        .flatten()
        .flatten()
        .collect();
        // Desktop's `zit` row, after Bookmark, opens the reminder submenu level with it.
        let remind_top = before.iter().map(|r| r.1).sum::<f32>();
        let remind = crate::reminders::can_remind(&st, m).then(|| {
            let label = match st.message_reminders.contains_key(&mid) {
                true => trx("Edit Reminder", "message menu action"),
                false => trx("Remind Me", "message menu action"),
            };
            let open = UiMessage::Reminders(crate::reminders::Msg::MessageSub);
            (mouse_area(menu_item(p, None, label.into(), "›", Some(open.clone()))).on_enter(open).into(), menu_pitch())
        });
        let after: Vec<(El, f32)> = [
            (m.is_unread == Some(true)).then(|| {
                let read = UiMessage::Message(Msg::MarkReadThrough(cid.clone(), mid.clone()));
                vec![item(tr("Mark as Read").into(), "", Some(read))]
            }),
            (unread_from(&st, m) && !queued).then(|| {
                vec![item(
                    tr("Mark as Unread").into(),
                    "",
                    Some(UiMessage::Message(Msg::MarkUnreadFromHere(cid.clone(), mid.clone()))),
                )]
            }),
            (mine
                && m.poll.as_ref().is_some_and(|p| !p.ended)
                && m.is_deleted != Some(true)
                && !editing
                && !queued
                && st.feature("enable-polls").on
                && st.chat(&cid).polls.is_some_and(|r| r.end))
            .then(|| {
                vec![item("End Poll".into(), "", Some(UiMessage::Message(Msg::EndPoll(cid.clone(), mid.clone()))))]
            }),
            // Desktop's `Z`: from the reply chain, the thread pane, or a right click on a quote, which jumps to the
            // quoted message. The jump closes the reply chain and the thread pane.
            (self.pane().reply_thread.is_some() || self.pane().thread.is_some() || self.pane().menu_linked.is_some())
                .then(|| {
                    let target = self.pane().menu_linked.clone().unwrap_or_else(|| mid.clone());
                    let jump = UiMessage::Commands(vec![
                        UiMessage::Message(Msg::HideReplies),
                        UiMessage::Thread(threads::Msg::Hide),
                        UiMessage::CommandBar(commands::Msg::JumpTo(cid, target.clone())),
                        UiMessage::Message(Msg::Select(vec![target])),
                    ]);
                    vec![item(tr("Jump to Message").into(), "", Some(jump))]
                }),
        ]
        .into_iter()
        .flatten()
        .flatten()
        .collect();
        let head: Vec<(El, f32)> = before.into_iter().chain(remind).chain(after).collect();
        let top = match self.pane().remind_sub {
            true => remind_top,
            false => head.iter().map(|r| r.1).sum::<f32>() + 11.0,
        };
        // Desktop's `Zit`: no Delete… for the message that you edit.
        let delete = (!editing && delete_dialog(&st, &[m]).is_some())
            .then(|| item(tr("Delete…").into(), "", Some(UiMessage::Message(Msg::AskDelete(vec![mid.clone()])))));
        let voice = if queued { Vec::new() } else { self.voice_menu_rows(&st, m) };
        let tail: Vec<(El, f32)> = voice.into_iter().chain(delete.filter(|_| !queued)).collect();
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
            // Desktop's item of a right-clicked custom emoji that is not the own one's name yet: it adds the image to
            // the own image pack, and is off when the pack has the name.
            self.pane()
                .menu_emoticon
                .as_ref()
                .filter(|(n, u)| !n.is_empty() && !u.is_empty() && !crate::emoji::is_mxc(n))
                .map(|(name, url)| {
                    let taken = st.image_pack["images"]
                        .get(name)
                        .is_some_and(|i| !matches!(i, serde_json::Value::Null | serde_json::Value::Bool(false)));
                    let add = UiMessage::Message(Msg::AddCustomEmoji(name.clone(), url.clone()));
                    vec![item(trf("Add :%s: to Custom Emoji", &[name]), "", (!taken).then_some(add))]
                }),
            (image && files > 0 && !only_image).then(|| {
                vec![item(tr("Copy Image").into(), "", Some(UiMessage::Message(Msg::CopyImage(mid.clone(), 0))))]
            }),
            // Desktop's `canOpen` of the forward menu: no view-once media.
            (!m.view_once()).then(|| {
                vec![item(
                    tr("Forward…").into(),
                    &key("Message: Forward"),
                    Some(UiMessage::Forward(forward::Msg::Open(vec![mid.clone()]))),
                )]
            }),
            (files > 0).then(|| {
                vec![item(
                    download,
                    &key("Download Attachments"),
                    Some(UiMessage::Message(Msg::Download(mid.clone(), None))),
                )]
            }),
            // Desktop opens the first PDF of the message in the carousel.
            m.attachments.iter().position(crate::pdf::is_pdf).map(|ai| {
                let open = UiMessage::Media(media_view::Msg::OpenCarousel(mid.clone(), ai, false));
                vec![item(tr("Preview in Carousel").into(), "", Some(open))]
            }),
            Some(vec![item(at, "", None)]),
            timer.map(|t| vec![item(t, "", None)]),
            Some(vec![
                rule(),
                item(
                    tr("Report a Problem").into(),
                    &key("Report a Problem"),
                    Some(UiMessage::OpenUrl(ISSUES_URL.into())),
                ),
            ]),
        ]
        .into_iter()
        .flatten()
        .flatten()
        .collect();
        let (menu, h) = crate::menu_rows(p, rows, MESSAGE_MENU_W);
        (menu.into(), h, top)
    }

    /// Desktop's `Nmt`: the Chat Info toggle, a 20 px info icon in 4 px of padding, shaded while Chat Info is open.
    fn view_info_toggle<'a>(&self, st: &BeeperStore) -> El<'a> {
        let p = &self.pal;
        let open = self.show_right_panel;
        let bg = if open { p.strong.scale_alpha(0.08) } else { CLEAR };
        let muted = p.muted;
        let icon =
            svg(crate::INFO_CIRCLE.clone()).width(20).height(20).style(move |_, _| svg::Style { color: Some(muted) });
        let toggle =
            button(icon).padding(4).style(btn_style([bg; 3], [p.muted; 3], 8.0)).on_press(UiMessage::ToggleRightPanel);
        let caps = hotkey_caps(st.settings.hotkey_keys("Toggle Chat Info"));
        let toggle = self.tip_caps(toggle.into(), tr("Chat Info"), caps, tooltip::Position::Bottom);
        container(toggle).align_right(Fill).padding(Padding::ZERO.top(8.0).right(8.0)).into()
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
        let avatar = badged(self.chat_avatar(chat, 46.0, 0.0), merged::chat_brand(chat), 46.0, 16.0, mono, p.card);
        let title = semi(chat.title.clone(), 15.0).line_height(Pixels(rem(20.0))).color(p.strong);
        let username = username.map(|u| txt(u, 12.0).color(p.sub));
        let about = chat.description.clone().filter(|d| !d.is_empty());
        let about = about.map(|d| txt(d, 13.0).color(p.sub).align_x(Center));
        let profile = profile_link(chat).map(|url| {
            button(semi(tr("View Profile"), 13.0).color(p.accent)).bare(p.accent).on_press(UiMessage::OpenUrl(url))
        });
        // Desktop's `inviterInfo` of a group: "__INVITER__ invited you (__USERNAME__) on __PLATFORM__", with the inviter's
        // 16 px avatar and name, which open their card.
        let invite = chat.inviter().filter(|_| chat.chat_type == "group").map(|u| -> El<'a> {
            let me = chat.participants.items.iter().find(|u| u.is_self == Some(true)).and_then(handle);
            let name = u.full_name.clone().unwrap_or_else(|| u.id.clone());
            let who = row![self.face(u.img_url.as_deref(), &name, 16.0, 0.0), semi(name, 13.0).color(p.text)]
                .spacing(4)
                .align_y(Center);
            let who = self.user_trigger(chat, &u.id, who);
            let text = tr(match me {
                Some(_) => "__INVITER__ invited you (__USERNAME__) on __PLATFORM__",
                None => "__INVITER__ invited you on __PLATFORM__",
            });
            let platform = beeper_core::config::account_network(&chat.account_id).display_name;
            let (before, rest) = text.split_once("__INVITER__").unwrap_or(("", text));
            let rest = rest.replace("__PLATFORM__", platform);
            let mut line = row![txt(before.to_string(), 13.0).color(p.text), who].align_y(Center);
            line = match (me, rest.split_once("__USERNAME__")) {
                (Some(me), Some((a, b))) => line
                    .push(txt(a.to_string(), 13.0).color(p.text))
                    .push(semi(me, 13.0).color(p.text))
                    .push(txt(b.to_string(), 13.0).color(p.text)),
                _ => line.push(txt(rest, 13.0).color(p.text)),
            };
            line.wrap().into()
        });
        let col = column![avatar, column![title].push(username).align_x(Center)].push(about).push(profile).push(invite);
        let col = col.spacing(10).align_x(Center).max_width(480).padding(Padding::new(24.0).bottom(32.0));
        Some(container(col).center_x(Fill).into())
    }

    /// Desktop's `InviteDetailsPane` (`P$t`), which shows in place of the chat for an invite to a Matrix room: the
    /// room, who invited the user, the topic, and Decline and Accept.
    fn view_invite_details<'a>(&self, chat: &Chat) -> El<'a> {
        let p = &self.pal;
        let inviter = chat.inviter().map(|u| {
            let name =
                semi(u.full_name.clone().unwrap_or_default(), 14.0).line_height(Pixels(rem(18.0))).color(p.strong);
            self.user_trigger(chat, &u.id, column![name, txt(format!("({})", u.id), 13.0).color(p.sub)].align_x(Center))
        });
        let group = chat.chat_type == "group";
        // Desktop does not translate these.
        let text = if group { "invited you to the group:" } else { "invited you to chat" };
        let text = txt(text, 13.0).color(p.text);
        let title = group.then(|| semi(chat.title.clone(), 15.0).line_height(Pixels(rem(20.0))).color(p.strong));
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
            let keys = hotkey_caps(st.settings.hotkey_keys("Chat: Toggle Mark As Read"));
            let tip = self.tip_caps(link.into(), tr("Mark as Read"), keys, tooltip::Position::Top);
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
        let typing = self.typing_now(&st, cid).filter(|_| in_chat);
        let col = col.push(typing.map(|(ids, v)| self.typing_bubble(&st, chat, ids, v)));
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
        // Desktop's `beeper-nudge-active` shakes the scrolling container of the chat that a nudge came to.
        let (dx, dy) = self.nudges.offset(st.resolve_canonical_chat_id(&chat.id).as_deref());
        let up = self.pane().scrolled_up.is_some_and(|s| s.0);
        let held = timeline::hold(scroller(col).id(self.pane().scroll_id()), up).glide(self.pane().glide);
        // Chromium scrolls the timeline while a drag that selects its text holds the pointer past its top or bottom.
        let list: El<'_> =
            lift::tight(crate::selectable::autoscroll(held, self.pane().scroll_id(), true)).shift(dx, dy).into();
        let list = match (&self.pane().editing_message_id, self.pane().stash.as_ref().map(|s| s.since)) {
            (Some(mid), Some(since)) => {
                let cancel = UiMessage::Composer(composer::Msg::CancelMode);
                timeline::veil(list, iced::widget::Id::from(format!("msg-{mid}")), since, p.card, cancel).into()
            }
            _ => list,
        };
        let opened = self.pane().reply_opened.map_or(1.0, |at| at.elapsed().as_secs_f32());
        let (shown, below) = replies_reveal(opened);
        let replies = self.pane().reply_thread.as_ref().map(|id| {
            let root = st.loaded_idx(id).map_or(id.as_str(), |i| reply_root(&st, &st.messages[i]));
            let chain: Vec<&Message> = msgs.iter().copied().filter(|m| reply_root(&st, m) == root).collect();
            let list = lift::tight(scroller(self.view_messages(&st, chat, &chain, &Rows::of(&st, &chain), true)));
            // iced draws no widget at an opacity, so only the background and the close button fade in, and the
            // messages rise inside the overlay, which hides what is under it.
            let list = container(lift::lift(list, below, 1.0)).clip(true);
            // Desktop blurs the chat 8px under a 0.6 background; iced has no backdrop blur.
            let bg = p.card.scale_alpha(0.85 * shown);
            mouse_area(container(list).width(Fill).height(Fill).style(fill_box(bg, 0.0)))
                .on_press(UiMessage::Message(Msg::HideReplies))
        });
        let close = replies.is_some().then(|| -> El<'_> {
            let close = button(ic(icon::CLOSE, 20.0).color(p.weak.scale_alpha(shown)))
                .padding(10)
                .style(|_, _| button::Style::default())
                .on_press(UiMessage::Message(Msg::HideReplies));
            container(close).align_right(Fill).padding(16).into()
        });
        // Desktop's `thread-header-card`: the avatar on its disc over the name pill, 4 px into it. The card shows no
        // hover; the pointer on it shows the members (`kmt`) instead.
        // Desktop's `thread-header-disappearing-timer`: the 12 px timer of a chat whose messages disappear, 4 px after
        // the name, with "Messages disappear after %s".
        let timer = chat.disappearing.filter(|&t| t > 0).map(|t| {
            let tertiary = p.tertiary;
            let icon = svg(TIMER.clone()).width(12).height(12).style(move |_, _| svg::Style { color: Some(tertiary) });
            let tip = trf("Messages disappear after %s", &[&beeper_core::models::timer_distance(t)]);
            container(self.tip(icon.into(), tip, vec![], tooltip::Position::Bottom)).padding(Padding::ZERO.left(2))
        });
        let name_row = row![semi(short_title(chat).to_string(), 13.0).line_height(Pixels(rem(16.0))).color(p.fg)]
            .push(timer)
            .push(ic(icon::CHEVRON_RIGHT, 12.0).color(p.tertiary))
            .spacing(2)
            .align_y(Center);
        // Desktop's `thread-header-status`: the activity of the other person of a 1:1 chat under the name.
        let typing = st.typing.get(cid).filter(|ids| chat.chat_type == "single" && ids.len() == 1);
        let status = typing.map(|_| txt("typing", 10.0).color(p.sub));
        let status_line = status.is_some();
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
        // Desktop's pill shows the earliest message reminder until it is due, else the pending chat reminder.
        let chat_reminder = chat.reminder.as_ref().filter(|_| !self.expired(chat));
        let chat_reminder = chat_reminder.map(|r| (r.remind_at_ms, r.dismiss_on_incoming_message));
        let earliest = st.message_reminders_of(&chat.id).first().map(|(_, r)| r.remind_at_ms);
        let reminder = match earliest.filter(|_| crate::reminders::message_reminders_on(&st)) {
            Some(at) => (at > now_unix_millis()).then_some((at, false)),
            None => chat_reminder,
        };
        let reminder = reminder.map(|(remind_at_ms, if_no_reply)| {
            let (day, time) = (reminder_day(remind_at_ms), clock(remind_at_ms, st.settings.pref("timeFormat") == 1));
            let at = if day_of(remind_at_ms) == today() { time.clone() } else { format!("{time} {day}") };
            let tip = match if_no_reply {
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
            container(pill).padding(Padding::ZERO.top(9.0))
        });
        // Desktop's `thread-header-lhs`: "Show Sidebar" while the main window hides the sidebar, then the reminder.
        let show_sidebar = (self.sidebar_hidden && self.drawing.get().is_none()).then(|| {
            let muted = p.muted;
            let icon = svg(crate::THUMBNAIL_BAR.clone())
                .width(20)
                .height(20)
                .style(move |_, _| svg::Style { color: Some(muted) });
            // `button.btn-icon`: 4 px around the icon inside a 1 px transparent border, radius 6.
            let show =
                button(icon).padding(5).style(crate::ghost(p, 6.0)).on_press(UiMessage::RunHotkey("Toggle Sidebar"));
            let caps = hotkey_caps(st.settings.hotkey_keys("Toggle Sidebar"));
            container(self.tip_caps(show.into(), tr("Show Sidebar"), caps, tooltip::Position::Bottom))
                .padding(Padding::ZERO.top(3.0))
        });
        let lhs = (show_sidebar.is_some() || reminder.is_some())
            .then(|| row![].push(show_sidebar).push(reminder).spacing(1).padding(Padding::ZERO.left(8.0)));
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
        // Desktop's request has no thread header, with its feedback button, only the Chat Info toggle (`Nmt`) 8 px
        // from the top right corner.
        // Desktop's thread header (`ij`) is gone while chats are selected (`threadSelectionMode`).
        let selecting = self.drawing.get().is_none() && (self.select_mode || !self.selection.is_empty());
        let headed = in_chat && !selecting && chat.is_request != Some(true);
        // Desktop's `MergeSuggestionBanner` follows the header card 16 px under it, and the members follow it.
        let banner = self.view_merge_banner(&st, chat).filter(|_| headed);
        let card_h = 8.0 + 36.0 + 28.0 + if status_line { 2.0 + 13.0 } else { 0.0 };
        let below = if banner.is_some() { 16.0 + crate::merge_suggest::BANNER_H } else { 0.0 };
        let banner = banner.map(|b| container(b).center_x(Fill).padding(Padding::ZERO.top(card_h + 16.0 - 11.0)));
        let members = self.header_hover.0.then(|| self.header_members(chat)).flatten();
        let members =
            members.filter(|_| in_chat).map(|m| container(m).center_x(Fill).padding(Padding::ZERO.top(84.0 + below)));
        // Desktop's `sticky-date-container`: while the timeline scrolls, the date of the day at its top floats under the
        // header.
        let sticky = self
            .pane()
            .timeline
            .sticky()
            .filter(|_| in_chat)
            .and_then(|i| msgs.get(i))
            .map(|m| self.day_pill(&m.timestamp, true).padding(Padding::ZERO.top(STICKY_DATE).left(GUTTER)));
        let overlay = match in_chat {
            true if selecting => stack![fade],
            true if chat.is_request == Some(true) => stack![fade, self.view_info_toggle(&st)],
            true => stack![fade, header].push(feedback).push(lhs),
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
        let disabled = |leading: Option<El<'static>>, lead: String, link: &str, end: &str, msg| -> El<'static> {
            let part = |t: String, c: Color| txt(t, 13.0).color(c);
            let link = row![].push(leading).push(part(lead, p.sub)).push(part(link.into(), p.text));
            let link = link.push(part(end.into(), p.sub)).align_y(Center);
            let link = button(container(link).center_x(Fill).center_y(36))
                .padding(8)
                .style(btn_style([CLEAR; 3], [p.sub; 3], 0.0))
                .on_press(msg);
            container(link).style(fill_box(p.card, 0.0)).into()
        };
        let upgraded = chat.replacement_room.as_deref().and_then(|r| st.resolve_chat_idx(r)).map(|i| {
            let lead = format!("{} ", tr("Chat was upgraded."));
            let msg = UiMessage::SelectChat(st.chats[i].id.clone());
            disabled(None, lead, tr("Click here to follow the conversation"), ".", msg)
        });
        // Desktop's `blockedComposerThread`: a merged chat whose target member chat is blocked leads with its switcher.
        let merged = !st.members(st.chat_idx(&chat.id)).is_empty();
        let target = if merged { &st.chats[self.compose_target_of(&st, st.chat_idx(&chat.id))] } else { chat };
        let blocked = (st.user_blocked(target) && target.block & NO_UNBLOCK == 0).then(|| {
            let lead = tr("You blocked __NAME__. __UNBLOCK__").replace("__NAME__", &target.title);
            let lead = lead.trim_end_matches("__UNBLOCK__").to_string();
            let unblock = trx("Unblock", "action to unblock the other person in a DM");
            let switcher = merged.then(|| -> El<'static> {
                let face = container(self.chat_avatar(target, 20.0, 1.25)).padding(Padding::ZERO.right(6.0));
                mouse_area(face).on_enter(UiMessage::SwitchCard).into()
            });
            disabled(switcher, lead, unblock, "", UiMessage::Blocking(blocking::Msg::Ask(target.id.clone(), false)))
        });
        // Desktop's `TargetSwitcherNudge` comes with the text composer and its switcher.
        let texting = upgraded.is_none() && blocked.is_none() && !chat.exited() && unavailable(&st, chat).is_none();
        let texting = texting && !selecting;
        let nudge = self
            .view_switcher_nudge(&st)
            .filter(|_| texting && in_chat && self.pane().thread_root.is_none() && self.drawing.get().is_none())
            .filter(|_| self.recorder.is_none() && self.talk.is_none());
        // Desktop's `ThreadComposer` draws nothing once the user left the chat or was removed from it, or in the
        // selection mode (`threadSelectionMode`).
        let composer = match upgraded.or(blocked) {
            _ if chat.exited() || selecting => None,
            Some(link) => Some(link),
            None => unavailable(&st, chat).is_none().then(|| self.view_composer(&st, chat)),
        };
        let jump = self.view_scroll_to_bottom(&st, chat);
        let pane = column![stack![list, overlay]
            .push(sticky)
            .push(banner)
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
            // Desktop's overlay is over the chat (`z-index: 10`): it takes the pointer, so the buttons under it lose
            // their hover and the tooltip of a hovered button closes.
            let zone = mouse_area(zone).interaction(mouse::Interaction::Idle);
            container(zone).padding(Padding { top: 39.0, right: 13.0, bottom: 6.5, left: 6.5 })
        });
        let wallpaper =
            wallpaper.map(|paint| iced::widget::responsive(move |size| chat_theme::fill(paint.clone(), size, 10.0)));
        let pane = stack![].push(wallpaper).push(pane).push(nudge).push(picker).push(self.view_suggestions(&st));
        let pane = pane.push(dropping);
        container(pane).width(Fill).height(Fill).clip(true).style(pane_card(p.card)).into()
    }

    /// Desktop's `MessageErrorTooltip`: the label, and the error icon whose tooltip is the error of the send.
    fn error_tooltip<'a>(&self, e: &beeper_core::models::SendError, label: &'a str) -> El<'a> {
        let icon = svg(ERROR_OUTLINE.clone()).width(20).height(20);
        let icon = icon.style(|_, _| svg::Style { color: Some(ON_ERROR_CONTAINER) });
        let error = e.message.clone().unwrap_or_else(|| tr("Unknown error occurred").into());
        let tip = self.tip(icon.into(), error, vec![], tooltip::Position::Left);
        row![crate::style::fixed(label, 11.0, Weight::Normal).color(ON_ERROR_CONTAINER), tip]
            .spacing(4)
            .align_y(Center)
            .into()
    }

    /// Desktop's `jumpToMessage` and `revealMessage`: shows message `mid` of chat `cid` 80 px under the top of the
    /// timeline of the pane (`scrollToSortKey`), and a reply only in a thread in the thread pane. The thread pane
    /// jumps in its own thread. It selects the message unless one is being edited (`selectOnlyMessage`), and flashes
    /// it when `highlight`: Desktop's `selectMessage` jumps from search, notifications, links, bookmarks, reminders
    /// and the "Jump to Message" item flash it, and its `jumpToMessage` from the media viewer does not.
    pub(crate) fn jump_to(&mut self, cid: String, mid: String) -> Task<UiMessage> {
        self.reveal(cid, mid, true)
    }

    /// [`Self::jump_to`], with the flash only when `highlight`.
    pub(crate) fn reveal(&mut self, cid: String, mid: String, highlight: bool) -> Task<UiMessage> {
        if self.show_command_palette {
            self.search_closed();
        }
        (self.show_command_palette, self.command_palette_query, self.list.chat_menu) = (false, String::new(), None);
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
        if self.pane.editing_message_id.is_none() {
            self.pane.selected_msgs = vec![mid.clone()];
        }
        if highlight {
            self.flash = Some((mid, std::time::Instant::now()));
        }
        // Desktop's `scrollToPending` unpins the list from its bottom (`unpinAndSetScroll`), so the view stays on the
        // message while the rows around it load and measure.
        self.pane.scrolled_up = Some((true, std::time::Instant::now()));
        locate(msg, self.pane.scroll_id(), crate::Align::Start(80.0))
    }

    /// Desktop's `scrollToUnreadDivider` when a chat opens: the unread divider 80 px under the top of the timeline,
    /// which reads the chat only as far as it shows.
    pub(crate) fn scroll_to_unread(&mut self) -> Task<UiMessage> {
        let mid = {
            let st = self.store.read();
            let msgs = self.pane_msgs(&st);
            self.first_unread(&msgs).map(|i| msgs[i].id.clone())
        };
        let Some(mid) = mid else { return Task::none() };
        self.pane.scrolled_up = Some((true, std::time::Instant::now()));
        self.pane.timeline.reveal(vlist::key(&mid));
        locate(UNREAD_DIVIDER, self.pane.scroll_id(), crate::Align::Start(80.0))
    }

    /// Desktop's `UnreadDivider`: "New", or the incognito line with the days left of a free trial.
    fn unread_divider<'a>(&self, st: &BeeperStore) -> El<'a> {
        let p = &self.pal;
        let hush = st.incognito_indicator();
        let color = if hush { p.rule } else { p.accent };
        let line = || container(Space::new()).width(Fill).height(1).style(fill_box(color, 0.0));
        if !hush {
            // `.unread-divider span`: label-sm at 0.8 opacity, 12 px above and below, 10 px from the primary lines.
            let label = txt(trx("New", "unread message divider label"), 11.0).line_height(Pixels(rem(14.0)));
            return row![line(), label.color(p.accent.scale_alpha(0.8)), line()]
                .spacing(10)
                .align_y(Center)
                .padding([12, 0])
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
        // A chat that opens at its bottom has no scroll-to-bottom button to fade out, like Desktop's.
        let fade = match self.pane.scrolled_up {
            None => up,
            Some((was, _)) => was != up,
        };
        if fade {
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
    /// The rows of the messages of the conversation of the pane. The pane keeps them until what they are made from
    /// changes (`RowsKey`), so that a frame of a long chat does not read all its messages, also after a write to
    /// another chat.
    fn conversation_rows(&self, st: &BeeperStore, chat: &Chat) -> Rc<Rows> {
        let (pane, memo) = (self.pane(), &self.pane().rows);
        let kept = memo.borrow().as_ref().and_then(|(key, rows)| {
            let chats: Vec<&str> = key.chats.iter().map(|c| c.0.as_str()).collect();
            (*key == RowsKey::of(st, pane, chat, &chats, &rows.links)).then(|| rows.clone())
        });
        if let Some(rows) = kept {
            return rows;
        }
        let (hide, members) = (st.settings.on("hideDeleted"), st.settings.on("membershipEvents"));
        let msgs: Vec<&Message> = self.pane_msgs(st);
        let mut chats = vec![chat.id.as_str()];
        chats.extend(st.members(st.chat_idx(&chat.id)).into_iter().map(|i| st.chats[i].id.as_str()));
        chats.extend(msgs.iter().map(|m| m.chat_id.as_str()));
        chats.sort_unstable();
        chats.dedup();
        // Desktop shows expired view-once media even when it hides deleted messages.
        let hidden = |m: &Message| hide && m.is_deleted == Some(true) && !m.view_once_expired();
        let msgs: Vec<&Message> = msgs.into_iter().filter(|m| !hidden(m) && (members || !membership(m))).collect();
        let rows = Rc::new(Rows::of(st, &msgs));
        *memo.borrow_mut() = Some((RowsKey::of(st, pane, chat, &chats, &rows.links), rows.clone()));
        rows
    }

    /// Desktop's `MembershipGrouper`: a collapsed group of three or more member events opens while the selection is in
    /// it, and the group that opened so closes again once the selection leaves it.
    fn follow_member_selection(&mut self) {
        let group = {
            let st = self.store.read();
            let (msgs, _) = self.timeline(&st);
            let runs = member_runs(&msgs);
            let picked = msgs.iter().position(|m| membership(m) && self.pane.selected_msgs.contains(&m.id));
            let start = picked.map(|i| runs[i]).filter(|&s| run_size(&runs, s) >= 3);
            start.map(|s| msgs[s].id.clone())
        };
        if let Some(auto) = self.pane.auto_members.take_if(|a| group.as_ref() != Some(a)) {
            self.pane.open_members.remove(&auto);
        }
        if let Some(group) = group.filter(|g| self.pane.open_members.insert(g.clone())) {
            self.pane.auto_members = Some(group);
        }
    }

    /// The message of the open chat that the unread divider goes before.
    pub(crate) fn first_unread(&self, msgs: &[&Message]) -> Option<usize> {
        let at = self.pane().unread_divider.as_ref()?;
        msgs.iter().position(|m| m.id == *at)
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
        let Rows { run_start, last_live, any_pending, last_seen, chained, counts, gallery, keys, days, .. } = rows;
        let last_received = msgs.iter().rposition(|m| m.is_sender != Some(true));
        // Desktop's `ThreadedRepliesView` lists the root and the replies of a thread under its header, and loads older
        // messages of the chat only until the root loads.
        let root = self.pane().thread_root.as_deref();
        let list = if thread { &self.pane().thread_list } else { &self.pane().timeline };
        // The date of the day that starts under the header shows only where it floats.
        let covered = list.covered().filter(|_| !thread);
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
            None => self.first_unread(msgs),
        };
        let first_unread = first_unread.filter(|_| !thread);
        let (h24, group) = (st.settings.pref("timeFormat") == 1, chat.chat_type == "group");
        let now = now_unix_millis();
        let gone = |m: &Message| m.disappeared(now) && !pending(st, m);
        // Desktop's `uR` with `groupReplies`, which the thread pane turns off, and without a reply that is also sent to
        // the chat (`Vb`). The reply chain overlay draws no lines.
        let broadcast = |m: &Message| st.threaded(chat) && root.is_none() && m.broadcast;
        let linked = |a: &Message, b: &Message, k: usize| {
            !thread && root.is_none() && chained[k] && !broadcast(a) && !broadcast(b)
        };
        let ink = self.wallpaper_fg(st, chat, thread).unwrap_or(p.reply_line);
        let item = |i: usize| -> El<'a> {
            // Desktop's `GalleryGrouper` shows a gallery at its first message.
            if gallery[i] != i {
                return Column::new().into();
            }
            let end = i + gallery[i..].iter().take_while(|&&s| s == i).count();
            let (m, prev, tail) = (msgs[i], i.checked_sub(1).map(|j| msgs[j]), msgs[end - 1]);
            let next = msgs.get(end).copied();
            let mine = m.is_sender == Some(true);
            let day_changed = prev.is_none_or(|q| day_num(&q.timestamp) != day_num(&m.timestamp));
            let mut head = Column::new().width(Fill).push(prev.map(|q| Space::new().height(gap_below(q, m))));
            if day_changed {
                let shown = covered.is_none_or(|c| i > c);
                head = head.push(self.day_pill(&m.timestamp, shown).padding(Padding::new(4.0).left(GUTTER).right(0.0)));
            }
            // The `along-message-footer` line of the message before goes on through the gap below it, past its
            // sender's avatar, and the date between two messages of a chain (`MessageDate`) has a line of its own.
            if let Some(q) = prev.filter(|q| linked(q, m, i)) {
                let (dm, face) = (!group && !mine, group && q.is_sender != Some(true) && q.sender_id != m.sender_id);
                let (segment, top, bottom) = match day_changed {
                    true => (Segment::new(Shape::Line, false, dm), 0.0, if dm { 4.0 } else { 0.0 }),
                    false => {
                        let end = !group && (q.is_sender != Some(true) || q.sender_id != m.sender_id);
                        (Segment::new(Shape::Line, face, end), if face { 4.0 } else { 0.0 }, if dm { 2.0 } else { 0.0 })
                    }
                };
                head = Column::new().push(stack![head].push_under(reply_line(segment, ink, top, bottom)));
            }
            // Desktop's `TimelineNote` of a merged chat: the chat of the message, where the chat of the messages changes.
            if prev.is_none_or(|q| q.chat_id != m.chat_id) && chat.merge.is_some() {
                head = head.push(merged::source_label(st, chat, &m.chat_id).map(|l| self.timeline_note(l)));
            }
            if first_unread.is_some_and(|u| (i..end).contains(&u)) {
                head = head.push(container(self.unread_divider(st)).id(UNREAD_DIVIDER));
            }
            if let Some(timer) = m.timer {
                let next =
                    msgs.get(i + 1).filter(|n| n.timer.is_some() && day_num(&n.timestamp) == day_num(&m.timestamp));
                return match next.is_none_or(|n| n.sender_id != m.sender_id) {
                    true => head.push(self.timer_notice(st, chat, m, timer, next.is_none())).into(),
                    false => Column::new().into(),
                };
            }
            // Desktop's `DisappearedMessagesGrouper`: one note for the messages of a day whose timers ran out.
            if gone(m) {
                let day = |n: &Message| day_num(&n.timestamp) == day_num(&m.timestamp);
                if prev.is_some_and(|q| gone(q) && day(q)) {
                    return Column::new().into();
                }
                let n = msgs[i..].iter().take_while(|n| gone(n) && day(n)).count();
                return head
                    .push(self.timeline_note(trn("%d message disappeared", "%d messages disappeared", n, &[&n])))
                    .into();
            }
            if membership(m) {
                let mut rows = Column::new();
                let s = run_start[i];
                let size = run_size(run_start, s);
                let open = self.pane().open_members.contains(&msgs[s].id);
                let dim = self.wallpaper_fg(st, chat, thread).unwrap_or(p.muted).scale_alpha(0.75);
                if size >= 3 && i == s {
                    let label = row![
                        txt(trn("%d Membership Event", "%d Membership Events", size, &[&size]), 11.0).color(dim),
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
            let first = day_changed || prev.is_none_or(|q| q.sender_id != m.sender_id || membership(q) || gone(q));
            let next_day = next.is_none_or(|n| day_num(&n.timestamp) != day_num(&tail.timestamp));
            // Desktop's `showSenderImg` also shows the avatar before a reply of another chain.
            let last = next.is_none_or(|n| {
                n.sender_id != m.sender_id
                    || membership(n)
                    || gone(n)
                    || next_day
                    || (!chained[end] && reply_root(st, n) != n.id)
            });
            let chain = Chain {
                prev: prev.is_some_and(|q| linked(q, m, i)),
                next: next.is_some_and(|n| linked(tail, n, end)),
                next_mine: next.is_some_and(|n| n.is_sender == Some(true)),
                next_same: next.is_some_and(|n| n.sender_id == m.sender_id && !membership(n)),
                next_day,
            };
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
                    // Desktop's `Seen by`: the newest reader first, by first name when no other member has it, with
                    // all the full names in its tooltip.
                    let members = || chat.participants.items.iter().filter(|u| u.has_exited != Some(true));
                    let first = |u: &User| u.short_name().to_lowercase();
                    let (names, full): (Vec<String>, Vec<String>) = viewers
                        .iter()
                        .rev()
                        .map(|&(uid, _)| {
                            let Some(u) = chat.participants.items.iter().find(|u| u.id == uid) else {
                                return (uid.to_string(), uid.to_string());
                            };
                            let full = u.custom_nickname.as_deref().or(u.full_name.as_deref()).unwrap_or(uid);
                            match members().filter(|o| first(o) == first(u)).count() {
                                1 => (u.short_name(), full.to_string()),
                                _ => (full.to_string(), full.to_string()),
                            }
                        })
                        .unzip();
                    let label = if names.len() <= 4 {
                        trf("Seen by %s", &[&names.join(", ")])
                    } else {
                        let (first, more) = (names[..3].join(", "), names.len() - 3);
                        trn("Seen by %1$s & %2$d other", "Seen by %1$s & %2$d others", more, &[&first, &more])
                    };
                    Some((label, Some(full.join(", "))))
                } else {
                    let played = matches!(m.message_type.as_deref(), Some("VOICE" | "AUDIO"));
                    Some(match day_num(ts) == today() {
                        true => trf(if played { "Played at %s" } else { "Seen at %s" }, &[&format_short_ts(ts, h24)]),
                        false => trf(if played { "Played %s" } else { "Seen %s" }, &[&day_label(ts)]),
                    })
                    .map(|t| (t, None))
                }
            } else if m.send_error.is_some() || m.is_deleted == Some(true) || !mine {
                None
            } else if pending(st, m) {
                let queued = self.queued.iter().any(|q| q.sends.iter().any(|s| s.0 == m.id));
                let late = still_sending_at(m) <= now_unix_millis();
                let label = tr(if queued {
                    "Queued"
                } else if late {
                    "Still sending..."
                } else {
                    "Sending..."
                });
                Some((label.into(), None))
            } else {
                (Some(end - 1) == *last_live && !any_pending)
                    .then(|| (tr(if m.is_delivered { "Delivered" } else { "Sent" }).into(), None))
            };
            let split = chained.get(end).is_some_and(|c| !c);
            let replies = if !thread && split && !st.threaded(chat) { counts[end - 1] } else { 0 };
            let ms = &msgs[i..end];
            let hide_time = time_hidden(m, next, st.settings.pref("timeFormat") == 1);
            let flags = (first, last, chain, thread, hide_time);
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
        // The pill floats at `STICKY_DATE`, 4 px into the row of its day.
        list.sections(days.clone(), if thread { 0.0 } else { STICKY_DATE - 4.0 });
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

    /// Desktop's `MessageDate` pill of the day of `timestamp`, in the middle of its row, or its empty place.
    fn day_pill<'a>(&self, timestamp: &str, shown: bool) -> iced::widget::Container<'a, UiMessage> {
        let p = &self.pal;
        let [fg, bg, edge] = if shown { [p.on_secondary, p.card, p.chip] } else { [CLEAR; 3] };
        let label = semi(day_label(timestamp), 11.0).color(fg);
        container(container(label).padding([5, 9]).style(bordered_box(bg, edge, 1.0, 12.0))).center_x(Fill)
    }

    /// A message, or the gallery of the messages `ms`.
    fn view_message<'a>(
        &'a self,
        st: &BeeperStore,
        chat: &Chat,
        ms: &[&Message],
        (first, last, chain, thread, hide_time): (bool, bool, Chain, bool, bool),
        (status, replies): (Option<(String, Option<String>)>, usize),
        head: Column<'a, UiMessage>,
    ) -> El<'a> {
        let (p, m) = (&self.pal, ms[0]);
        let (sent, sent_fg, sent_meta) = self.sent_colors(st, Some(&chat.id));
        let bubble =
            chat_theme::resolve(&st.settings.chat_appearance, Some(&chat.id), self.dark, &st.config.data_dir).1;
        let on_wall = self.wallpaper_fg(st, chat, thread);
        let muted = on_wall.unwrap_or(p.muted);
        let mine = m.is_sender == Some(true);
        // Desktop's `Ve`: an open edit history goes before the message, which follows its last version, so it does
        // not name the sender again and shows its time.
        let open = self.pane().expanded_history_msg_id.as_ref() == Some(&m.id) && !m.edit_history.is_empty();
        let (named_first, hide_time) = (first && !open, hide_time && !open);
        let shows_time = st.settings.on("showTimestamps") && !hide_time;
        let italic = iced::Font { style: iced::font::Style::Italic, ..fw(Weight::Normal) };
        // Desktop keeps the row of a deleted message, with its sender, its menu and its selection.
        let deleted = (m.is_deleted == Some(true) && !m.view_once_expired()).then(|| {
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
        // Desktop's `isNotice` bubble (a bot's `m.notice`) at 0.7 opacity.
        let notice = if m.message_type.as_deref() == Some("NOTICE") { 0.7 } else { 1.0 };
        let dim = move |c: Color| (if selected { brightness(c, 0.7) } else { c }).scale_alpha(rise * notice);
        // Desktop's `isErrored` bubble takes the secondary container colors.
        let failed = m.send_error.as_ref().filter(|_| mine);
        let (bg, fg) = match (mine, failed) {
            (true, Some(_)) => (dim(p.received), dim(p.on_secondary)),
            (true, None) => (dim(sent), dim(sent_fg)),
            (false, _) => (dim(p.received), dim(p.strong)),
        };
        // Desktop's `is-edit-history` bubble: the sent color at half, or `--color-base-gray-40` at half.
        let history = m.id.contains(EDIT_HISTORY);
        let bg = match history {
            true => (if mine { sent } else { p.link_card }).scale_alpha(0.5),
            false => bg,
        };
        let poll = m.poll.as_ref().filter(|_| st.feature("enable-polls").on && chat.polls.is_some());
        // Desktop's `isE2EE` message that the keys did not decrypt yet.
        let utd = m.text.as_deref() == Some(beeper_core::matrix::UTD_TEXT);
        let text = match (&m.poll, poll) {
            (Some(p), None) => p.fallback.clone(),
            _ if utd => tr("This message is waiting for decryption.").into(),
            _ => m.text.clone().unwrap_or_default(),
        };
        let big = !utd && m.poll.is_none() && jumbo(&emoji_text(m));
        let mut parts = Column::new().spacing(2).align_x(if mine { Right } else { iced::Left });
        let named = (group || network_bot(chat, &m.sender_id)) && !mine && named_first;
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
            && !big
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
        // Desktop's `linkedMessage` with its 4 px margin below, and in a group the quoted sender's 24 px avatar at 0.6
        // beside a quote of someone else (`reply-avatar`), at the bottom of a 34 px column.
        let quote_row = |quote: El<'a>, q: Option<&Message>| -> El<'a> {
            let line: El<'a> = match q.is_some_and(|q| q.is_sender == Some(true)) {
                true => row![Space::new().width(Fill), quote].into(),
                false => {
                    let face = group.then(|| {
                        let u = q.and_then(|q| chat.participants.items.iter().find(|u| u.id == q.sender_id));
                        let name = q.filter(|_| u.is_some()).map(|q| sender_label(chat, q)).unwrap_or_default();
                        let face = self.face_alpha(u.and_then(|u| u.img_url.as_deref()), &name, 24.0, 1.0, 0.6);
                        container(face).center_x(FACE).align_bottom(FACE)
                    });
                    row![].push(face).push(quote).spacing(FACE_GAP).align_y(Bottom).into()
                }
            };
            container(line).padding(Padding::ZERO.bottom(4.0)).into()
        };
        let quote = linked.map(|r| {
            let Some(q) = st.loaded(r).or_else(|| cached(r)) else {
                // Desktop's `L1t`: "Loading…" while `loadMessage` runs, then "Message unavailable", in a dashed box.
                let loading = match self.quotes.get(r) {
                    None => {
                        self.wants.ask(&self.wants.quotes, &(chat.id.clone(), r.clone()));
                        true
                    }
                    Some(done) => done.is_none(),
                };
                let text = if loading { tr("Loading…") } else { tr("Message unavailable") };
                let pill = container(txt(text, 12.0).color(p.sub)).padding(8).style(fill_box(p.card, 17.0));
                let pill = stack![pill, crate::dashed::dashes(p.chip, 17.0, 2.0)];
                return ((!chain.prev).then(|| quote_row(pill.into(), None)), false);
            };
            let q_mine = q.is_sender == Some(true);
            let col = self.linked_message(chat, q, group, on_wall, bubble).filter(|_| !chain.prev);
            let quote = col.map(|c| {
                mouse_area(c)
                    .on_press(UiMessage::Message(Msg::ShowReplies(q.id.clone())))
                    .on_right_press(UiMessage::Message(Msg::LinkedMenu(m.id.clone(), q.id.clone())))
            });
            (quote.map(|c| quote_row(c.into(), Some(q))), q_mine)
        });
        let shown = quote.as_ref().and_then(|(q, q_mine)| q.is_some().then_some(*q_mine));
        let (segments, face) = chain.segments(linked.is_some(), shown, mine, group, last);
        // Desktop's `pM`: a link with a title, an image or a summary has a preview.
        for link in m.links.iter().filter(|l| !l.title.is_empty() || l.img.is_some() || l.summary.is_some()) {
            parts = parts.push(self.view_link(
                (&m.id, st.settings.on("autoplayGifs")),
                link,
                0.45 * self.pane().size.width,
                dim,
            ));
        }
        if ms.len() > 1 {
            parts = parts.push(self.view_gallery(ms));
        }
        // Desktop's `ViewLimitedMediaAttachment` takes the place of view-once media.
        if m.view_once() {
            parts = parts.push(self.view_once_pill(m, (bg, fg)));
        }
        // Desktop's `message-attachment-failed`: a failed message without text shows "Not sent" at its attachments,
        // over the bottom right of a lone photo or video, else in a row under them.
        let attached = ms.len() == 1 && !m.view_once() && !m.attachments.is_empty();
        let failed_files = failed.filter(|_| attached && m.text.as_deref().is_none_or(str::is_empty));
        let lone_media = matches!(&m.attachments[..], [a] if a.attachment_type == "img" || a.attachment_type == "video"
            || a.mime_type.as_deref().is_some_and(|t| t.starts_with("image/")));
        let not_sent = |content: El<'a>| -> El<'a> {
            match failed_files.filter(|_| lone_media) {
                Some(e) => stack![
                    content,
                    container(self.error_tooltip(e, tr("Not sent")))
                        .align_right(Fill)
                        .align_bottom(Fill)
                        .padding(Padding::ZERO.right(12.0))
                ]
                .into(),
                None => content,
            }
        };
        for (ai, a) in m.attachments.iter().enumerate().filter(|_| ms.len() == 1 && !m.view_once()) {
            let name = attachment_name(a);
            let content: El<'a> = match (a.is_voice_note == Some(true), a.attachment_type.as_str()) {
                (_, "audio") if self.playable(a) => {
                    let ts = shows_time.then(|| {
                        let time = format_short_ts(&m.timestamp, st.settings.pref("timeFormat") == 1);
                        (time, if mine { sent_meta } else { p.sub })
                    });
                    self.view_audio(m, a, mine, ts, fg, bg)
                }
                (false, "img" | "video") if m.unresolved_media.is_some() || self.needs_download(a).is_none() => {
                    let is_video = a.attachment_type == "video";
                    let size = a.size.as_ref().and_then(|s| Some((s.width? as f32, s.height? as f32)));
                    let share = if self.sidebar_hidden { 0.85 } else { 0.45 };
                    let (w, h, cover) = match size.filter(|_| a.is_sticker == Some(true)) {
                        Some(size) => sticker_box(self.pane().size.width, size, share),
                        None => media_box(self.pane().size.width, size, share),
                    };
                    // Desktop plays a video with `isGif` inline like a GIF, without sound, again and again.
                    let looped = is_video && a.is_gif == Some(true) && m.unresolved_media.is_none();
                    // Desktop's `Lottie` plays an animated sticker itself, not its still thumbnail.
                    let lottie = a.mime_type.as_deref().is_some_and(beeper_core::media::animated);
                    let thumb_url = match looped {
                        // The local echo of a GIF plays the frames of its poster until the homeserver has it.
                        true => a
                            .src_url
                            .as_ref()
                            .map(|u| format!("{u}{}", crate::media_view::LOOP))
                            .or_else(|| a.poster_img.clone()),
                        false if lottie => a.src_url.clone(),
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
                    // Desktop's `<img>`, whose right click gives the menu its image items.
                    let img = !is_video && matches!(thumb_url.as_ref().and_then(|u| self.media_at(u)), Some(Ok(_)));
                    // A cropped image shows from a baked cut, as the renderer draws `Cover` past its box: a still, or
                    // the frames of an animation once a worker cut them. Until then the whole image shows. A still
                    // much larger than its box shows from a cut too, like Chromium's decode to scale, so the renderer
                    // keeps and draws the pixels of the box and not those of a whole photo.
                    let px = (w.round() as u32, h.round() as u32);
                    let base: El<'_> = match thumb_url.as_ref().and_then(|u| Some((u, self.media_at(u)?))) {
                        Some((url, Ok(frames)))
                            if (cover || larger(&frames[0].0, px))
                                && !lottie
                                && frames.len() == 1
                                && let Some(t) = self.cover_tile(url, px) =>
                        {
                            image(t.clone()).width(w).height(h).into()
                        }
                        Some((url, Ok(frames))) => {
                            let cut =
                                (cover && !lottie && frames.len() > 1).then(|| self.cover_frames(url, px)).flatten();
                            let autoplay = st.settings.on("autoplayGifs");
                            let played = self.pane().played_gifs.get(&(m.id.clone(), ai)).filter(|_| !autoplay);
                            let (play, start) = match played {
                                Some(&at) => (self.shows(), at),
                                None => (self.autoplays(st), self.started),
                            };
                            let gif = match cut {
                                // The cut has the corners of the bubble.
                                Some(cut) => anim::gif(cut, play, start, move |f| image(f).width(w).height(h)),
                                None => anim::gif(frames, play, start, move |f| {
                                    image(f).width(w).height(h).border_radius(17.0)
                                }),
                            };
                            match !autoplay && frames.animated() {
                                true => gif_toggle(gif, played.is_some(), Msg::ToggleGif(m.id.clone(), ai)),
                                false => gif,
                            }
                        }
                        Some((_, Err(_))) if m.unresolved_media.is_none() => self.image_error(w, h, 17.0),
                        // Desktop's `img.loading`: the shimmering `--skeleton-bg` in the box of the image.
                        _ => anim::skeleton(w, h, 17.0, p.skeleton, self.started),
                    };
                    let (slot, press): (El<'_>, _) =
                        if let Some(raw) = m.unresolved_media.as_ref().filter(|_| !is_video) {
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
                        } else if let Some(src) = a.src_url.as_ref().filter(|_| is_video && a.is_gif != Some(true)) {
                            let video = self.view_video(&m.id, src, base, (w, h), video::Place::Inline(ai));
                            parts = parts.push(not_sent(dimmed(video, selected)));
                            continue;
                        } else if is_video && a.is_gif != Some(true) {
                            (base, UiMessage::OpenQuickLook(name, "Video attachment".into()))
                        } else {
                            (base, UiMessage::Media(media_view::Msg::OpenCarousel(m.id.clone(), ai, false)))
                        };
                    let slot = button(slot).padding(0).style(btn_style([CLEAR; 3], [p.text; 3], 17.0)).on_press(press);
                    match img {
                        true => {
                            mouse_area(slot).on_right_press(UiMessage::Message(Msg::ImageMenu(m.id.clone()))).into()
                        }
                        false => slot.into(),
                    }
                }
                // Desktop's `gx` attachments show as `Wfe` contact cards, once their file is read.
                _ if is_vcard(a) && a.src_url.as_ref().and_then(|u| self.vcard_at(u)).is_none_or(Result::is_ok) => {
                    match a.src_url.as_ref().and_then(|u| self.vcard_at(u)) {
                        Some(Ok(card)) => self.view_contact_attachment(m, ai, card, mine, (bg, fg)),
                        _ => Space::new().into(),
                    }
                }
                // Desktop's file row of a file, and of media that a bridge loads only when the user asks for it.
                _ => {
                    // Desktop's disappearing timer goes after the details of a file without a caption, when the
                    // message shows its time.
                    let timer = disappears(chat, m).filter(|_| !hide_time && text.is_empty());
                    self.view_file_row(m, ai, a, timer)
                }
            };
            parts = parts.push(not_sent(dimmed(content, selected)));
        }
        if let Some(e) = failed_files.filter(|_| !lone_media) {
            // The column puts it at the right of an own message.
            parts =
                parts.push(container(self.error_tooltip(e, tr("Not sent"))).padding(Padding::ZERO.right(BUBBLE_PAD_X)));
        }
        if let Some(poll) = poll {
            parts = parts.push(self.view_poll(st, chat, m, poll, (bg, fg)));
        }
        if let Some(pill) = deleted {
            parts = parts.push(pill);
        } else if m.view_once_expired() {
            parts = parts.push(self.view_once_indicator(m, view_once::Variant::Outlined, (p.card, p.text)));
        } else if m.message_type.as_deref() == Some("HIDDEN") {
            let bold = |s: String| span::<(), _>(s).font(fw(Weight::Bold));
            let head = container(txt(tr("Hidden event"), 13.0).font(fw(Weight::Bold)))
                .padding([2.6, 5.2])
                .style(fill_box(p.chip, 0.0));
            let why = tr("Shown because you enabled Settings > General > Show hidden events in timeline");
            let body = rich_text![bold(text.clone()), span::<(), _>(" from "), bold(m.sender_id.clone())]
                .size(rem(13.0))
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
        } else if big {
            // Desktop's `.is-selected.type-emoji .message-contents`: `brightness(0.8)`, which iced draws in images.
            let emoji = |t: String| -> El<'a> {
                match selected {
                    true => {
                        let (handle, size) = crate::style::dimmed_emoji(&t, 48.0, 48.0 * 1.3, 0.8);
                        image(handle).width(size.width).height(size.height).into()
                    }
                    false => txt(t, 48.0).font(EMOJI_FONT).into(),
                }
            };
            // Custom emoji are 1em high images among the 48 px emoji.
            let custom = m.html.as_deref().filter(|h| h.contains("data-mx-emoticon"));
            let runs = custom.map(beeper_core::codec::html_blocks).unwrap_or_default().into_iter().flat_map(|b| b.runs);
            let glyphs: Vec<El<'a>> = runs
                .map(|r| match r.emoticon.as_deref() {
                    Some(url) => {
                        // A selected animation plays dimmed once a worker dimmed its frames, and shows its first
                        // frame dimmed until then.
                        let dimmed = |f: &crate::Frames| match f.len() > 1 {
                            true => self.dimmed_frames(url).map(|d| self.emoji(d, |h| image(h).height(48.0))),
                            false => None,
                        };
                        let img: El<'a> = match self.frames(url) {
                            Some(f) if selected && let Some(d) = dimmed(&f) => d,
                            Some(f) if selected => image(crate::style::dimmed_image(&f[0].0, 0.8)).height(48.0).into(),
                            Some(f) => self.emoji(&f, |h| image(h).height(48.0)),
                            None => Space::new().width(48.0).height(48.0).into(),
                        };
                        let menu = Msg::EmoticonMenu(m.id.clone(), r.title.clone(), url.to_string());
                        mouse_area(img).on_right_press(UiMessage::Message(menu)).into()
                    }
                    None => emoji(r.text),
                })
                .collect();
            let line: El<'a> = match glyphs.is_empty() {
                true => emoji(text.clone()),
                false => Row::with_children(glyphs).align_y(Bottom).into(),
            };
            // Desktop's `title` of the `message-text` of an emoji message: its time.
            let time = format_short_ts(&m.timestamp, st.settings.pref("timeFormat") == 1);
            let line = self.tip(line, time, vec![], tooltip::Position::Top);
            // `.bubble-mode .type-emoji .message-contents`: the bubble's padding, without the bottom one.
            parts = parts.push(container(line).padding(Padding::from([BUBBLE_PAD_Y, BUBBLE_PAD_X]).bottom(0.0)));
        } else if utd {
            let line = txt(text.clone(), 13.0).line_height(Pixels(rem(16.0))).font(italic).color(fg.scale_alpha(0.8));
            let time = shows_time.then(|| {
                let time = format_short_ts(&m.timestamp, st.settings.pref("timeFormat") == 1);
                crate::style::fixed(time, 11.0, Weight::Normal).color(if mine { sent_meta } else { p.sub })
            });
            let body = column![].push(inside).push(row![line].push(time).spacing(4).align_y(Bottom));
            parts = parts.push(container(body).padding([BUBBLE_PAD_Y, BUBBLE_PAD_X]).style(fill_box(bg, 17.0)));
        } else if !text.is_empty() && poll.is_none() && m.message_type.as_deref() != Some("VOICE") {
            let ts = shows_time.then(|| format_short_ts(&m.timestamp, st.settings.pref("timeFormat") == 1));
            let room = ts.as_deref().map_or(0.0, |t| crate::style::fixed_width(t, 11.0, Weight::Normal) + 4.0);
            let time: Option<El<'a>> = ts.map(|t| {
                crate::style::fixed(t, 11.0, Weight::Normal).color(if mine { sent_meta } else { p.sub }).into()
            });
            // Desktop's `MessageErrorTooltip` takes the place of the time.
            let (room, time) = match failed {
                Some(e) => {
                    let label = if m.edited() { tr("Failed to edit") } else { tr("Not sent") };
                    (crate::style::fixed_width(label, 11.0, Weight::Normal) + 28.0, Some(self.error_tooltip(e, label)))
                }
                // Desktop's `BookmarkIndicator` and `ReminderIndicator` (`E3`) go before the time.
                None => {
                    let color = if mine { sent_meta } else { p.sub };
                    let marker = |handle: &svg::Handle, color: Color, label: String| {
                        let icon = svg(handle.clone()).width(10).height(10);
                        let icon = icon.style(move |_, _| svg::Style { color: Some(color) });
                        self.tip(icon.into(), label, vec![], tooltip::Position::Left)
                    };
                    let bookmark =
                        (m.is_bookmarked == Some(true)).then(|| marker(&BOOKMARK_ICON, color, tr("Bookmarked").into()));
                    let reminder =
                        st.message_reminders.get(&m.id).filter(|_| crate::reminders::message_reminders_on(st));
                    let reminder = reminder.filter(|_| m.is_deleted != Some(true)).map(|r| {
                        let at = r.remind_at_ms;
                        let h24 = st.settings.pref("timeFormat") == 1;
                        let label = trf("Reminder: %1$s %2$s", &[&reminder_day(at), &clock(at, h24)]);
                        // Desktop's `ReminderIndicator-module__due`: a due reminder takes the warning color.
                        marker(&OUTLINE_CLOCK, if at <= now_unix_millis() { ORANGE } else { color }, label)
                    });
                    // Desktop's disappearing timer goes after the time.
                    let timer = disappears(chat, m).map(|label| marker(&TIMER, color, label));
                    let n =
                        [bookmark.is_some(), reminder.is_some(), timer.is_some()].into_iter().filter(|&b| b).count();
                    match n {
                        0 => (room, time),
                        n => {
                            let meta = row![].push(bookmark).push(reminder).push(time).push(timer);
                            let meta = meta.spacing(4).align_y(Center);
                            (room + 14.0 * n as f32, Some(meta.into()))
                        }
                    }
                }
            };
            let me = |id: &str| id == st.beeper_state.user_id;
            // Desktop's `DJ`: "@room" in a message that mentions the room, or has no `m.mentions`, is a highlighted
            // mention.
            let room_mention = m.text.as_deref().is_some_and(|t| t.contains(ROOM_MENTION))
                && m.mentions.as_ref().is_none_or(|ids| ids.iter().any(|id| id == ROOM_MENTION));
            let highlighted = |s: iced::widget::text::Span<'a, String>| {
                let pill = if mine { Color { a: 0.3, ..fg } } else { p.accent };
                s.font(crate::style::fw(Weight::Bold))
                    .color(Color::WHITE)
                    .background(pill)
                    .border(border::rounded(4))
                    .padding(2)
            };
            // Desktop's `.ext-link-with-icon:after`: the arrow at 0.7 opacity, 2 px from the link and the text after it.
            let arrow = || {
                let arrow = span(icon::ARROW_UP_RIGHT.to_string()).font(iced::Font::with_name("icons")).size(rem(11.0));
                arrow.color(fg.scale_alpha(0.7)).padding(Padding::from([0, 2]))
            };
            let rich = |(r, room): (beeper_core::codec::Run, bool)| {
                if r.emoticon.is_some() {
                    return [Some(span(crate::selectable::IMAGE_SLOT).size(rem(13.0))), None].into_iter().flatten();
                }
                // Desktop's `--font-weight-emphasized` bold, and its inline `code`: `--font-family-mono` at 0.9em and
                // weight 500, on `--color-on-surface` at 0.1 with a 4 px radius and `.1em .33em` of padding.
                let style = if r.italic { iced::font::Style::Italic } else { iced::font::Style::Normal };
                let font = match r.code {
                    true => iced::Font { weight: Weight::Medium, ..iced::Font::MONOSPACE },
                    false => {
                        iced::Font { style, ..crate::style::fw(if r.bold { Weight::Semibold } else { Weight::Normal }) }
                    }
                };
                let mention = r.link.as_deref().and_then(mentioned_user);
                // Desktop's `data-mx-color` paints the text.
                let ink = r.color.as_deref().and_then(|c| c.parse::<Color>().ok()).unwrap_or(fg);
                let s =
                    span(r.text).size(rem(13.0)).color(ink).font(font).strikethrough(r.strike).underline(r.underline);
                let s = match r.code {
                    true => s
                        .size(rem(13.0 * 0.9))
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
                // Desktop's `.mention-hover-card-trigger` is not an external link, so it has no arrow.
                let external = mention.is_none();
                let s = match mention {
                    _ if room => highlighted(s),
                    Some(id) if me(&id) => highlighted(s),
                    Some(id) => s.font(bold).color(if mine { fg } else { p.accent }).link(format!(
                        "{}{} {id}",
                        crate::MENTION,
                        m.chat_id
                    )),
                    None => s.underline(r.underline || r.link.is_some()).link_maybe(r.link.clone()),
                };
                let arrow = (external && s.link.is_some()).then(arrow);
                [Some(s), arrow].into_iter().flatten()
            };
            let word = |w: &str| span(w.to_string()).size(rem(13.0)).color(fg);
            // The words of plain text, with a highlighted "@room".
            let words = |text: &str| -> Vec<iced::widget::text::Span<'a, String>> {
                let parts = if room_mention { room_parts(text) } else { vec![(text, false)] };
                parts
                    .into_iter()
                    .flat_map(|(t, room)| match room {
                        true => vec![highlighted(word(t))],
                        false => t.split_inclusive(' ').map(word).collect(),
                    })
                    .collect()
            };
            // Plain text with Desktop's linkify links: each with its underline and arrow, the rest word by word.
            let plain = |text: &str| {
                let mut spans = Vec::new();
                let mut at = 0;
                for (range, href) in crate::linkify::links(text) {
                    spans.extend(words(&text[at..range.start]));
                    spans.push(
                        span(text[range.clone()].to_string()).size(rem(13.0)).color(fg).underline(true).link(href),
                    );
                    spans.push(arrow());
                    at = range.end;
                }
                spans.extend(words(&text[at..]));
                spans
            };
            // Each block: its list marker, spans, plain text, blockquote depth, heading level and custom emoji.
            // Each block: its list marker, spans, plain text, blockquote depth, heading level, custom emoji and whether it
            // is a code block.
            type Block<'s> = (
                Option<String>,
                Vec<iced::widget::text::Span<'s, String>>,
                String,
                u8,
                u8,
                Vec<(Option<crate::Frames>, Option<String>, String)>,
                bool,
            );
            let blocks: Vec<Block> = match m.html.as_deref() {
                Some(html) => beeper_core::codec::html_blocks(html)
                    .into_iter()
                    .map(|mut b| {
                        at_mentions(&mut b.runs);
                        let plain = b.runs.iter().map(|r| r.text.as_str()).collect();
                        // Each custom emoji's image, `title` and `data-original-src`.
                        let images = b.runs.iter().filter_map(|r| Some((r.emoticon.as_deref()?, r.title.clone())));
                        let images = images.map(|(u, title)| (self.frames(u), title, u.to_string())).collect();
                        let runs = b.runs.into_iter().flat_map(|r| match room_mention && r.link.is_none() && !r.code {
                            true => room_parts(&r.text)
                                .into_iter()
                                .map(|(t, room)| (beeper_core::codec::Run { text: t.into(), ..r.clone() }, room))
                                .collect(),
                            false => vec![(r, false)],
                        });
                        let mut spans = with_emoji_font(runs.flat_map(rich).collect());
                        // Desktop's `pre code` is one padded block, so its spans have no box of their own.
                        if b.pre {
                            for s in &mut spans {
                                (s.highlight, s.padding) = (None, Padding::ZERO);
                            }
                        }
                        (b.marker, heading(spans, b.heading), plain, b.quote, b.heading, images, b.pre)
                    })
                    .collect(),
                None => {
                    let spans = with_emoji_font(plain(&text));
                    vec![(None, spans, text.clone(), 0, 0, Vec::new(), false)]
                }
            };
            let max = 0.45 * self.pane().size.width - 2.0 * BUBBLE_PAD_X;
            // Desktop's `.message-text` clamps a message at 40 lines, and "Show more" opens it. The whole blocks that
            // fit show as they are, and the block that crosses the limit shows its first lines as plain text.
            let clamp = clamp_blocks(blocks.iter().map(|b| b.2.as_str()), max);
            let open = self.pane().unclamped.contains(&m.id);
            // Desktop's `pre code` is a block as wide as its longest line, and the time floats under a last one.
            let code_pad = 0.75 * 13.0 * 0.9;
            let code_lines = blocks.iter().filter(|b| b.6).flat_map(|b| b.2.lines());
            let code_w =
                code_lines.map(|l| l.chars().count() as f32 * 0.6 * 13.0 * 0.9 + 2.0 * code_pad).fold(0.0, f32::max);
            let last_pre = blocks.last().is_some_and(|b| b.6);
            let (room, time, footer_time) = match clamp {
                Some(_) => (0.0, None, time),
                None if last_pre => (0.0, None, time),
                None => (room, time, None),
            };
            let (footer_time, below) = if clamp.is_some() { (footer_time, None) } else { (None, footer_time) };
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
            // Desktop's `message-text--balanced`: CSS `text-wrap: balance`, then the bubble shrinks to its widest line.
            // A message with block HTML wraps as usual. A wrapped message without balance takes the whole width.
            // The layout lets trailing spaces hang past the width, so the room of the time is measured with digits.
            let zero = crate::style::text_width("0", 13.0, Weight::Normal);
            let padded = format!("{text}\u{2060}{}", "0".repeat((room / zero).ceil() as usize));
            let blocks_html =
                m.html.as_deref().is_some_and(|h| BLOCK_TAGS.iter().any(|t| h.contains(&format!("<{t}"))));
            let width = match code_w > 0.0 || crate::style::wraps(&padded, max, 13.0) {
                _ if code_w > 0.0 => {
                    let lines = blocks.iter().filter(|b| !b.6).flat_map(|b| b.2.lines());
                    let text_w = lines.map(|l| crate::style::text_width(l, 13.0, Weight::Normal)).fold(0.0, f32::max);
                    Length::Fixed(code_w.max(text_w).min(max))
                }
                false => Length::Shrink,
                true if st.settings.on("balanceText") && !blocks_html => {
                    Length::Fixed(crate::style::balanced_width(&padded, max, 13.0))
                }
                true => Length::Fixed(max),
            };
            // Desktop's `ol` indent: `max(1.8em, var(--ol-number-digits) * 1ch + 0.65em)`, with the digits of its
            // longest number.
            let digits =
                blocks.iter().filter_map(|b| b.0.as_deref()?.strip_suffix('.')).map(str::len).max().unwrap_or(1);
            let ol_indent =
                (1.8 * 13.0f32).max(digits as f32 * crate::style::text_width("0", 13.0, Weight::Normal) + 0.65 * 13.0);
            let items: Vec<(u8, El<'_>)> = blocks
                .into_iter()
                .enumerate()
                .map(|(i, (marker, mut spans, _, quote, level, images, pre))| {
                    // iced does not draw an empty last line.
                    if spans.last().is_some_and(|s| s.text.ends_with('\n')) {
                        spans.push(span("\u{200b}").size(rem(13.0)));
                    }
                    if i == last && room > 0.0 {
                        spans.push(span(pad.clone()).size(rem(13.0)));
                    }
                    let id = m.id.clone();
                    let tipped = tipped_links(&spans, m.html.is_some());
                    let tip = move |href: &String| -> Option<El<'a>> {
                        let host = url_host(href).filter(|_| tipped.contains(href) && !self.menu_open())?;
                        Some(self.tip_card(med(host, 13.0).color(p.strong)).into())
                    };
                    let line = crate::selectable::selectable(spans)
                        .key(vlist::key((&m.id, i)))
                        .images(
                            images.iter().map(|i| i.0.clone()).collect(),
                            self.animate.get().then_some(self.started),
                        )
                        .on_image_menu({
                            let (id, images) = (m.id.clone(), images);
                            move |k| {
                                let (_, title, src) = images[k].clone();
                                UiMessage::Message(Msg::EmoticonMenu(id.clone(), title, src))
                            }
                        })
                        .line_height(iced::widget::text::LineHeight::Absolute(rem(16.0).into()))
                        .wrapping(iced::widget::text::Wrapping::WordOrGlyph)
                        .on_link_click(UiMessage::OpenUrl)
                        .on_link_menu(move |href| UiMessage::Message(Msg::LinkMenu(id.clone(), href)))
                        .link_tip(tip);
                    let gap = match (prev_item.replace(marker.is_some()), marker.is_some()) {
                        (None, _) => 0.0,
                        (Some(true), true) => 6.0,
                        _ => 9.0,
                    } + if level > 0 { 9.0 } else { 0.0 };
                    let indent = |m: &String| if m == "•" { 15.6 } else { ol_indent };
                    // Desktop's `pre code`: a block on the code background, 0.75em around the text.
                    let line: El<'_> = match pre {
                        true => container(line)
                            .padding(0.75 * 13.0 * 0.9)
                            .width(Fill)
                            .style(fill_box(Color { a: 0.1, ..p.strong }, 4.0))
                            .into(),
                        false => line.into(),
                    };
                    let marker = marker.map(|m| {
                        let w = indent(&m);
                        container(txt(m, 13.0).color(fg)).width(w).align_right(w).padding(Padding::ZERO.right(5.0))
                    });
                    (quote, container(row![].push(marker).push(line)).padding(Padding::ZERO.top(gap)).into())
                })
                .collect();
            // Desktop's `dir="auto"`: a right-to-left text starts its lines at the right of the bubble, and its time
            // floats to its left, where its room is.
            let rtl = right_to_left(&text);
            let start = if rtl { Right } else { iced::Left };
            let body = Column::with_children(quotes(items, 0, p.strong)).width(width).align_x(start);
            let cut = cut.map(|(t, left)| {
                txt(cut_lines(&t, max, left), 13.0).color(fg).wrapping(iced::widget::text::Wrapping::WordOrGlyph)
            });
            let body = stack![column![body].push(cut).align_x(start)].push(time.map(|t| match rtl {
                true => container(t).align_left(Fill).align_bottom(Fill),
                false => container(t).align_right(Fill).align_bottom(Fill),
            }));
            // Desktop's `read-more-button`: the primary color, white on a sent bubble, with the time at its end.
            let more = clamp.map(|_| {
                let label = semi(if open { tr("Show less") } else { tr("Show more") }, 14.0);
                let color = if mine { fg } else { p.accent };
                let toggle = button(label.line_height(Pixels(rem(18.0))).color(color)).bare(color);
                let toggle = toggle.on_press(UiMessage::Message(Msg::ToggleClamp(m.id.clone())));
                // A Fill gap would widen the bubble past its text, so the time follows the label.
                row![toggle].push(footer_time).spacing(8).align_y(Bottom)
            });
            parts = parts.push(
                container(
                    column![]
                        .push(inside)
                        .push(body)
                        .push(below.map(|t| container(t).align_right(width).padding(Padding::ZERO.top(4.0))))
                        .push(more),
                )
                .padding([BUBBLE_PAD_Y, BUBBLE_PAD_X])
                .max_width(0.45 * self.pane().size.width)
                .style(fill_box(bg, 17.0)),
            );
        }
        let label = |t: String| txt(t, 12.0).color(muted);
        // Desktop's `EditedLabel` shows on an edited message that is not deleted, failed or retrying.
        let edited = (m.edited() && m.is_deleted != Some(true) && m.send_error.is_none()).then(|| -> El<'a> {
            let small = |t: &str| txt(t.to_string(), 11.0);
            let Some(last) = m.edit_history.last() else {
                return small(tr("Edited")).color(muted).into();
            };
            let toggle = small(if open { tr("Hide Edits") } else { tr("Edited") }).font(fw(Weight::Semibold));
            let toggle = button(toggle.color(p.accent))
                .padding(0)
                .style(|_, _| button::Style::default())
                .on_press(UiMessage::Message(Msg::ToggleEditHistory(m.id.clone())));
            // Desktop's `u2t`: "Edited after" the time from the message to its last edit, unless that is under 1 s.
            let after = millis_from_rfc3339(&last.timestamp).zip(millis_from_rfc3339(&m.timestamp));
            let after = after.map(|(edit, sent)| short_distance(edit.abs_diff(sent))).filter(|d| d != "0s");
            match after {
                Some(after) => {
                    self.tip(toggle.into(), trf("Edited after %s", &[&after]), vec![], tooltip::Position::Top).into()
                }
                None => toggle.into(),
            }
        });
        let replies = (replies > 0).then(|| -> El<'a> {
            let n = trnx("%d Reply", "%d Replies", replies, "button label for replies to a message", &[&replies]);
            // `.MessageFooter-module__messageInfoLine .message-show-replies-button`: label-md, regular.
            button(txt(n, 12.0).line_height(Pixels(rem(15.0))).color(muted))
                .padding(0)
                .style(|_, _| button::Style::default())
                .on_press(UiMessage::Message(Msg::ShowReplies(m.id.clone())))
                .into()
        });
        let broadcast =
            (in_pane && m.broadcast).then(|| -> El<'a> { txt(tr("Also sent to the chat"), 11.0).color(muted).into() });
        let mut footer: Vec<El<'a>> = [status.map(|(t, tip)| -> El<'a> {
            match tip {
                Some(tip) => self.tip(label(t).into(), tip, vec![], tooltip::Position::Left).into(),
                None => label(t).into(),
            }
        })]
        .into_iter()
        .chain([replies, broadcast, edited])
        .flatten()
        .collect();
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
        let versions = open.then(|| self.edit_history(st, chat, m, first, thread));
        let parts: El<'a> = if reactions.is_empty() {
            parts.into()
        } else {
            let min_w = reaction_groups(ms).len() as f32 * 36.0 + 2.0 * BUBBLE_PAD_X;
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
                button(txt(label, 12.0).line_height(Pixels(rem(16.0))))
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
            .push(self.action_buttons(st, m))
            .push(actions)
            .push(thread_button)
            .push(footer)
            .push(transcribe)
            .align_x(if mine { Right } else { iced::Left });
        // Desktop's flyover and download button do not show on a call.
        let tools = (self.pane().hovered_msg.as_ref() == Some(&m.id) && m.call.is_none()).then(|| {
            // Desktop's flyover (`Yfe`): its items appear one after another, each from 85% of its size and clear to
            // its full size in 0.2 s, the next one once the one before is a quarter in. The quick reactions follow
            // once the pointer is on the flyover.
            let shown = |at: Option<std::time::Instant>, k: usize| {
                let t = at.map_or(1.0, |at| at.elapsed().as_secs_f32() - k as f32 * FLYOVER_STAGGER) / 0.2;
                crate::style::cubic_bezier(t.clamp(0.0, 1.0), [0.0, 0.0, 0.58, 1.0])
            };
            let hover_at = self.pane().hover_msg_at;
            // Desktop's flyover buttons: a 20 px Material icon, drawn at its 1.2 scale, in 6 px of padding.
            let b = |c: char, msg: UiMessage, v: f32| -> El<'a> {
                let style = move |_: &Theme, s| button::Style {
                    text_color: match s {
                        button::Status::Hovered => brightness(muted, 0.8),
                        _ => muted,
                    }
                    .scale_alpha(v),
                    ..Default::default()
                };
                let glyph = ic(c, 20.0 * 1.2 * (0.85 + 0.15 * v));
                button(container(glyph).center(32)).padding(0).style(style).on_press(msg).into()
            };
            let live = m.is_deleted != Some(true);
            // Desktop's flyover shows on no pending message, has no items on a gallery, and only React in the
            // thread pane.
            let items = !pending && ms.len() == 1;
            let base = [
                (items && !in_pane && chat.messages.reply && live).then_some(0),
                (items && !in_pane && can_thread(st, chat, m)).then_some(1),
                (items && chat.messages.react && m.reactable()).then_some(2),
            ];
            let base: Vec<u8> = base.into_iter().flatten().collect();
            let item = |kind: u8, k: usize| -> El<'a> {
                let v = shown(hover_at, k);
                let (b, tip) = match kind {
                    0 => (
                        b(icon::REPLY, UiMessage::Composer(composer::Msg::StartReply(m.id.clone())), v),
                        trx("Reply", "message command"),
                    ),
                    1 => (
                        b(icon::VIEW_SIDEBAR, UiMessage::Thread(threads::Msg::Show(m.id.clone())), v),
                        trx("Reply in thread", "message command"),
                    ),
                    _ => (
                        b(icon::MOOD, UiMessage::Message(Msg::React(m.id.clone())), v),
                        trx("React", "attachment carousel action"),
                    ),
                };
                // Desktop's `q3t`: the tooltips of the flyover wait a second.
                self.tip(b, tip, vec![], tooltip::Position::Top).delay(std::time::Duration::from_secs(1)).into()
            };
            let flyover: Vec<El<'a>> = base.iter().enumerate().map(|(k, &kind)| item(kind, k)).collect();
            // Desktop's quick reactions go only on received messages, after the other items.
            let quick = (self.quick_open && st.settings.on("quickReactions") && !mine && !in_pane && base.contains(&2))
                .then(|| crate::emoji::quick_reactions(st, &chat.allowed_reactions, chat.custom_emoji));
            let quick: Vec<String> = quick.unwrap_or_default();
            let after = hover_at.map(|t| t + std::time::Duration::from_secs_f32(base.len() as f32 * FLYOVER_STAGGER));
            let start = self.pane().flyover_at.max(after);
            // `disableReactions`: a quick reaction takes clicks 120 ms after the last one is in.
            let ready = start.is_none_or(|t| {
                t.elapsed().as_secs_f32() > (quick.len().saturating_sub(1) as f32 * FLYOVER_STAGGER) + 0.2 + 0.12
            });
            let quick = quick.into_iter().enumerate().map(|(j, e)| -> El<'a> {
                let v = shown(start, j);
                let msg = UiMessage::Message(Msg::ToggleReaction(chat.id.clone(), m.id.clone(), e.clone()));
                // Desktop's custom emoji among the quick reactions: its image 16 px square.
                let glyph: El<'a> = match crate::emoji::is_mxc(&e) {
                    true => match self.frames(&e) {
                        Some(f) => self.emoji(&f, move |h| {
                            image(h).width(16.0 * (0.85 + 0.15 * v)).height(16.0 * (0.85 + 0.15 * v)).opacity(v)
                        }),
                        None => Space::new().width(16.0).height(16.0).into(),
                    },
                    false => txt(e, 20.0 * (0.85 + 0.15 * v)).font(EMOJI_FONT).color(p.text.scale_alpha(v)).into(),
                };
                button(container(glyph).center(32))
                    .padding(0)
                    .style(btn_style([CLEAR; 3], [p.text; 3], 8.0))
                    .on_press_maybe(ready.then_some(msg))
                    .into()
            });
            // `@container messageContainer (max-width: 24rem)`: a narrow chat shows the first two items only. The
            // `.message-container` is as wide as the chat less 8 px.
            let room = if self.conversation_width() - 8.0 <= 384.0 { 2 } else { usize::MAX };
            let flyover: Vec<El<'a>> = flyover.into_iter().chain(quick).take(room).collect();
            let flyover = (!flyover.is_empty()).then(|| -> El<'a> {
                let row = match mine {
                    true => Row::with_children(flyover.into_iter().rev()),
                    false => Row::with_children(flyover),
                };
                mouse_area(row).on_enter(UiMessage::Message(Msg::ShowQuickReactions)).into()
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
                crate::style::fixed(
                    format_short_ts(&m.timestamp, st.settings.pref("timeFormat") == 1),
                    11.0,
                    Weight::Normal,
                )
                .color(muted)
                .into()
            });
            let all: Vec<El<'a>> = match mine {
                true => [flyover, time, download].into_iter().flatten().collect(),
                false => [download, time, flyover].into_iter().flatten().collect(),
            };
            Row::with_children(all)
        });
        let side = (group && !mine).then(|| -> El<'a> {
            if face {
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
        // Desktop's reply line segments, under the quote, along the bubble down to its middle (above the footer and
        // the reactions), and along the footer. The avatar takes the bottom of the row here, so a line above it stops
        // 4 px over it, and the footer's line starts in the gap below.
        let ink = on_wall.unwrap_or(p.reply_line);
        let Segments { quote: beside, body, above_face, footer: below } = segments;
        let lines = [
            body.map(|s| reply_line(s, ink, 0.0, tail)),
            above_face.map(|s| reply_line(s, ink, 0.0, FACE + 4.0)),
            below.filter(|_| !face && tail > 0.0).map(|s| -> El<'a> {
                // Desktop's `spaceAbove` of a received message in a DM, and the end of the line before another day.
                let above = if !mine && !group { 2.0 } else { 0.0 };
                let s = Segment { round_bottom: s.round_bottom && chain.next_day, ..s };
                column![Space::new().height(Fill), container(reply_line(s, ink, above, 0.0)).height(tail)].into()
            }),
        ];
        let line = lines.into_iter().flatten().fold(stack![line], |s, l| s.push_under(l));
        let quote = quote.and_then(|(q, _)| q).map(|q| -> El<'a> {
            match beside {
                // Desktop's `margin-bottom: 4px` of the line beside the quote of a received message in a DM.
                Some(s) => {
                    stack![q].push_under(reply_line(s, ink, 0.0, if !group && !mine { 4.0 } else { 0.0 })).into()
                }
                None => q,
            }
        });
        let line: El<'a> = column![].push(quote).push(line).into();
        // Desktop's edit history rows take no hover, click or menu.
        if history {
            return column![head, line].into();
        }
        let (menu, click) = match ms.len() {
            1 => (UiMessage::Message(Msg::OpenMenu(m.id.clone())), UiMessage::Message(Msg::Click(m.id.clone()))),
            _ => {
                let ids: Vec<String> = ms.iter().map(|g| g.id.clone()).collect();
                (UiMessage::Gallery(gallery::Msg::Menu(ids.clone())), UiMessage::Gallery(gallery::Msg::Click(ids)))
            }
        };
        // Inside the mouse area, whose double click takes every press.
        let line = mouse_area(crate::click::click(line, click))
            .on_enter(UiMessage::Message(Msg::HoverEnter(m.id.clone())))
            .on_exit(UiMessage::Message(Msg::HoverExit(m.id.clone())))
            .on_right_press(menu)
            .on_double_click(UiMessage::Message(Msg::DoubleClick(m.id.clone())));
        column![head].push(versions).push(line).into()
    }

    /// Desktop's `MessageEditHistory` (`x6t`) above an expanded edited message: its earlier versions oldest first,
    /// each a message row at the time it was sent or edited, without attachments or reactions, and with the gap of
    /// Desktop's `gapBelow` to the next one. The first version keeps the quote, and it names the sender in place of
    /// the message.
    fn edit_history<'a>(&'a self, st: &BeeperStore, chat: &Chat, m: &Message, first: bool, thread: bool) -> El<'a> {
        let times = std::iter::once(&m.timestamp).chain(m.edit_history.iter().map(|e| &e.timestamp));
        let versions: Vec<Message> = m
            .edit_history
            .iter()
            .zip(times)
            .enumerate()
            .map(|(k, (e, at))| Message {
                id: format!("{}{EDIT_HISTORY}{k}", m.id),
                timestamp: at.clone(),
                text: Some(e.text.clone()),
                html: None,
                linked_message_id: m.linked_message_id.clone().filter(|_| k == 0),
                attachments: Vec::new(),
                links: Vec::new(),
                reactions: Vec::new(),
                edit_history: Vec::new(),
                seen: Default::default(),
                is_edited: None,
                is_bookmarked: None,
                is_deleted: None,
                poll: None,
                call: None,
                timer: None,
                send_error: None,
                disappearing: None,
                unresolved_media: None,
                view_limit: None,
                broadcast: false,
                ..m.clone()
            })
            .collect();
        let nexts = versions.iter().skip(1).chain([m]);
        let rows = versions.iter().zip(nexts).enumerate().map(|(k, (version, next))| {
            let flags = (first && k == 0, false, Chain::default(), thread, false);
            let row = self.view_message(st, chat, &[version], flags, (None, 0), Column::new());
            column![row, Space::new().height(gap_below(version, next))].into()
        });
        Column::with_children(rows).into()
    }

    /// Desktop's `ActionButtons` (`W4t`) under a message of the system user that is not deleted: the buttons that
    /// can open their link or post to their URL, in a wrapping row 4 px apart and 4 px below the message.
    fn action_buttons<'a>(&self, st: &BeeperStore, m: &Message) -> Option<El<'a>> {
        if m.is_deleted == Some(true) || !system_user(&m.sender_id) {
            return None;
        }
        let home = &st.beeper_state.homeserver;
        let buttons: Vec<El<'a>> = (m.buttons.iter().filter(|b| b.usable(home)))
            .map(|b| {
                let loading = self.pressing.iter().any(|(mid, id)| *mid == m.id && *id == b.id);
                let press = UiMessage::Message(Msg::PressButton(m.id.clone(), b.id.clone()));
                action_button(&self.pal, b, (!loading).then_some(press))
            })
            .collect();
        (!buttons.is_empty()).then(|| {
            let row = Row::with_children(buttons).spacing(4).wrap().vertical_spacing(4);
            container(row).padding(Padding::ZERO.top(4.0)).into()
        })
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
        bubble: Option<chat_theme::Pair>,
    ) -> Option<Column<'a, UiMessage>> {
        let p = &self.pal;
        let deleted = q.is_deleted == Some(true);
        if deleted && q.message_type.as_deref() == Some("HIDDEN") {
            return None;
        }
        let q_mine = q.is_sender == Some(true);
        // Desktop's quote of your own message has the border of your bubbles, and its text in `--bubble-out-quote-fg`:
        // the primary color, or the text color under a chat bubble color. Its box is the timeline's own color, mixed
        // with 30% of the bubble color in the dark theme or under a chat bubble color (`has-bubble-color`).
        let sent = bubble.map_or(p.sent, |b| b.0);
        let (edge, fill, fg) = match q_mine {
            true => {
                let fill = if bubble.is_some() || self.dark { mix(p.card, sent, 0.3) } else { p.card };
                (sent, fill, if bubble.is_some() { p.strong } else { p.accent })
            }
            false => (p.received, p.card, p.text),
        };
        let name_fg = if q_mine { fg } else { p.strong };
        let max = 0.45 * self.pane().size.width;
        // Desktop's `isDeleted` box has a dashed border.
        let boxed = move |c: El<'a>, dashed: bool| -> El<'a> {
            let b = container(c).padding(8).max_width(max);
            match dashed {
                true => stack![b.style(fill_box(fill, 17.0)), crate::dashed::dashes(edge, 17.0, 2.0)].into(),
                false => b.style(bordered_box(fill, edge, 2.0, 17.0)).into(),
            }
        };
        // Desktop's view-once media, which can be viewed or expired, shows as its pill, and its text as its preview.
        let view = q.view_once() || q.view_once_expired();
        let visual = view || q.attachments.iter().any(|a| quote_tile(a) || a.attachment_type == "audio");
        let name = || semi(sender_label(chat, q), 12.0).line_height(Pixels(rem(16.0)));
        let items: Vec<El<'a>> = match view {
            true => vec![self.view_once_indicator(q, view_once::Variant::Quote, (fill, p.text))],
            false => q.attachments.iter().map(|a| self.linked_attachment(a, fg)).collect(),
        };
        let media = (!items.is_empty() || deleted).then(|| {
            let gone = tr(if q.removed { "Removed from Beeper" } else { "Message Deleted" });
            let gone = (deleted && !view).then(|| med(gone, 13.0).color(fg));
            boxed(Row::with_children(items).push(gone).spacing(4).align_y(Center).into(), deleted && !view)
        });
        let text = q.text.as_deref().filter(|_| !deleted).map(|t| match view_once::preview(q) {
            Some(v) => v.to_string(),
            None => t.split_whitespace().collect::<Vec<_>>().join(" "),
        });
        let text = text.filter(|t| !t.is_empty()).map(|t| match t.chars().count() > 128 {
            true => format!("{}...", t.chars().take(125).collect::<String>()),
            false => t,
        });
        let within = (group && !visual).then(|| name().color(name_fg));
        let text = text.map(|t| boxed(column![].push(within).push(med(t, 13.0).color(fg)).spacing(2).into(), false));
        let above = (group && visual).then(|| {
            container(name().color(on_wall.unwrap_or(name_fg))).padding(Padding::new(0.0).left(11.0).right(11.0))
        });
        let shown = media.is_some() || text.is_some();
        let col = column![].push(above).push(media).push(text).spacing(8);
        shown.then(|| col.align_x(if q_mine { Right } else { iced::Left }))
    }

    /// An attachment in Desktop's `bfe`: a 100 px tile of an image or a video at half opacity, a tag of a sticker, a
    /// GIF, a voice note, an audio file or a contact card, or the file row of Desktop's `FileAttachment` at 0.6, in
    /// the `fg` of the quote.
    fn linked_attachment<'a>(&'a self, a: &Attachment, fg: Color) -> El<'a> {
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
            return med(tag, 13.0).color(fg).into();
        }
        let meta = file_label(a).into_iter().chain(a.file_size.map(human_size)).collect::<Vec<_>>();
        let (text, sub) = (p.text.scale_alpha(0.6), p.sub.scale_alpha(0.6));
        let info = column![med(name.to_string(), 13.0).color(text)]
            .push((!meta.is_empty()).then(|| txt(meta.join(" ・ "), 11.0).color(sub)));
        row![tinted(file_icon(a), 20.0, text), info].spacing(8).align_y(Center).into()
    }

    /// Desktop's `TimelineNote`: a centered line of the timeline.
    fn timeline_note<'a>(&self, text: String) -> El<'a> {
        let note = container(semi(text, 11.0).color(self.pal.on_secondary)).center_x(Fill).center_y(24);
        container(note).padding(Padding::ZERO.bottom(12.0).left(GUTTER)).into()
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

    /// Desktop's `needsUserDownload`: the URL of the attachment, or of its poster, that a bridge loads only when the
    /// user asks for it.
    pub(crate) fn needs_download<'s>(&self, a: &'s Attachment) -> Option<&'s String> {
        [&a.src_url, &a.poster_img].into_iter().flatten().find(|u| self.needs_download.contains(*u))
    }

    /// The state of Desktop's `AttachmentFileRow`.
    pub(crate) fn file_state(&self, a: &Attachment) -> FileState {
        match self.needs_download(a) {
            Some(u) if self.fetching.contains(u) => FileState::Fetching,
            Some(_) => FileState::Unfetched,
            None => FileState::Ready,
        }
    }

    /// Desktop's `AttachmentFileRow`: at most 240 px wide, the name in `body-sm` on two lines at most, and the details
    /// in `label-sm` with the disappearing `timer`. Unfetched, it has the download icon in the brand color and asks
    /// for the media. Fetching, it has a muted spinner and is disabled. Ready, a PDF opens in the carousel and another
    /// file downloads.
    pub(crate) fn view_file_row<'a>(&'a self, m: &Message, ai: usize, a: &Attachment, timer: Option<String>) -> El<'a> {
        let p = &self.pal;
        let meta = file_label(a).into_iter().chain(a.file_size.map(human_size)).collect::<Vec<_>>();
        let timer = timer.map(|label| self.tip(tinted(&TIMER, 10.0, p.sub), label, vec![], tooltip::Position::Left));
        let details = (!meta.is_empty()).then(|| txt(meta.join(" · "), 11.0).color(p.sub));
        let details =
            (details.is_some() || timer.is_some()).then(|| row![].push(details).push(timer).spacing(4).align_y(Center));
        let name = attachment_name(a);
        let info = column![clamp(&name, 186.0, 2, 13.0, Weight::Semibold).line_height(Pixels(rem(16.0)))]
            .push(details)
            .spacing(4);
        let (icon, press) = match self.file_state(a) {
            FileState::Fetching => (file_row_spinner(p.weak), None),
            FileState::Unfetched => {
                let url = self.needs_download(a).expect("an unfetched attachment needs a download").clone();
                (tinted(&crate::MAT_DOWNLOAD, 24.0, p.accent), Some(UiMessage::Loaded(loads::Msg::Fetch(url))))
            }
            // Desktop's `onOpenPreview`: a PDF opens in the carousel.
            FileState::Ready => (
                tinted(file_icon(a), 24.0, p.strong),
                Some(match a.src_url.is_some() && crate::pdf::is_pdf(a) {
                    true => UiMessage::Media(media_view::Msg::OpenCarousel(m.id.clone(), ai, false)),
                    false => UiMessage::Message(Msg::Download(m.id.clone(), Some(ai))),
                }),
            ),
        };
        let line = row![icon, info].spacing(8).align_y(Center);
        // The disabled row of a fetching file keeps its colors.
        let style = btn_style([p.received; 3], [p.strong; 3], 17.0);
        let style =
            move |t: &Theme, s| style(t, if s == button::Status::Disabled { button::Status::Active } else { s });
        button(container(line).max_width(240.0 - 2.0 * BUBBLE_PAD_X))
            .padding([8, 11])
            .style(style)
            .on_press_maybe(press)
            .into()
    }

    /// Desktop's `ImageError` in the place of a media that failed to load: the info icon on the secondary container,
    /// with the "Image failed to load" tooltip.
    pub(crate) fn image_error<'a>(&self, w: impl Into<Length>, h: impl Into<Length>, radius: f32) -> El<'a> {
        let p = &self.pal;
        let icon = tinted(&crate::INFO_CIRCLE, 24.0, p.strong);
        let icon = self.tip(icon, tr("Image failed to load"), vec![], tooltip::Position::Top);
        container(icon).center_x(w).center_y(h).style(fill_box(p.chip, radius)).into()
    }

    /// Desktop's `Wfe` in the timeline: the contact card in the colors of the bubble, its first 10 phone numbers and
    /// emails, which start a chat on an account that can message them and else open their Call or Email and Copy menu,
    /// and its Download button.
    fn view_contact_attachment<'a>(
        &self,
        m: &Message,
        ai: usize,
        card: &crate::vcard::VCard,
        mine: bool,
        (bg, fg): (Color, Color),
    ) -> El<'a> {
        let p = &self.pal;
        let account = m.account_id.clone();
        let entry = |phone: bool, s: &str| -> El<'a> {
            let shown = if phone { crate::fmt_phone(s) } else { s.to_string() };
            let reach = self.can(&account, if phone { "lookup_phone" } else { "lookup_email" });
            let (label, msg) = match reach {
                true => {
                    let user = match phone {
                        true => User { id: s.into(), phone_number: Some(s.into()), ..Default::default() },
                        false => User { id: s.into(), email: Some(s.into()), ..Default::default() },
                    };
                    (format!("{shown} →"), UiMessage::Message(Msg::MessageContact(account.clone(), Box::new(user))))
                }
                false => {
                    let kind = if phone { "phone" } else { "email" };
                    (shown, UiMessage::ChatList(chat_list::Msg::OpenMenu(format!("vcard:{kind} {s}"))))
                }
            };
            let text = crate::style::fixed(label, 12.0, Weight::Normal).line_height(Pixels(rem(16.0)));
            let text = text.color(if reach { fg } else { p.link });
            button(text).padding(0).style(btn_style([CLEAR; 3], [fg; 3], 0.0)).on_press(msg).into()
        };
        let body = crate::contact_card(p, card, fg, 10, &entry);
        let glyph: El<'a> = ic(icon::DOWNLOAD, 16.0).color(fg).into();
        let download = button(container(glyph).center_x(Fill))
            .padding([4, 6])
            .width(Fill)
            .style(crate::style::bordered_btn([CLEAR; 3], [fg; 3], fg, 1.0, 4.0))
            .on_press(UiMessage::Message(Msg::Download(m.id.clone(), Some(ai))));
        let body = body.push(container(download).padding(Padding::ZERO.top(12.0)));
        // `.contact-attachment` is as wide as its details, at least 200 px, and its button stretches across it.
        let w = |s: &str, size: f32, weight| crate::style::fixed_width(s, size, weight);
        let entries =
            card.phones.iter().take(10).map(|n| w(&format!("{} →", crate::fmt_phone(n)), 12.0, Weight::Normal));
        let entries = entries.chain(card.emails.iter().take(10).map(|e| w(&format!("{e} →"), 12.0, Weight::Normal)));
        let text =
            [w(&card.name, 14.0, Weight::Medium), w(card.org.as_deref().unwrap_or_default(), 12.0, Weight::Normal)]
                .into_iter()
                .chain(entries)
                .fold(w(&trn("Phone Number", "Phone Numbers", 2, &[]), 12.0, Weight::Semibold), f32::max);
        let fill = if mine { bg } else { p.received };
        container(body)
            .width((32.0 + 8.0 + text.ceil()).max(180.0) + 20.0)
            .padding([8, 10])
            .style(fill_box(fill, 17.0))
            .into()
    }

    /// The frames of the image at `url`, once it has loaded.
    pub(crate) fn frames(&self, url: &str) -> Option<crate::Frames> {
        self.media_at(url).and_then(|m| m.as_ref().ok()).cloned()
    }

    /// Desktop's `Tfe`: a custom emoji's image 16 high, or the emoji.
    fn reaction_glyph<'a>(&self, x: &Reaction, size: f32, line: f32) -> El<'a> {
        // A key that is itself an image, as stored before keys had their shortcode, shows the image too.
        let url = x.img_url.as_deref().or(Some(x.reaction_key.as_str()).filter(|k| k.starts_with("mxc://")));
        match url {
            Some(url) => match self.frames(url) {
                Some(f) => self.emoji(&f, |h| image(h).height(16.0)),
                None => Space::new().width(16.0).height(16.0).into(),
            },
            None => txt(x.reaction_key.clone(), size).line_height(Pixels(rem(line))).font(EMOJI_FONT).into(),
        }
    }

    /// Desktop's reactions list (`Efe`) of messages `ms`: each reaction with the face and the name of its sender, by
    /// group, and "and N more" after 19 when there are more than 20.
    pub(crate) fn reaction_list<'a>(&self, chat: &Chat, ms: &[&Message]) -> El<'a> {
        let p = &self.pal;
        let all: Vec<&Reaction> = reaction_groups(ms).into_iter().flat_map(|(_, rs)| rs).collect();
        let (shown, more) = if all.len() <= 20 { (&all[..], 0) } else { (&all[..19], all.len() - 19) };
        let list = shown.iter().fold(Column::new(), |col, x| {
            let u = chat.participants.items.iter().find(|u| u.id == x.participant_id);
            let name = u.and_then(|u| u.full_name.as_deref()).unwrap_or(&x.participant_id);
            let line = row![
                container(self.reaction_glyph(x, 17.0, 17.0 * 1.3)).padding(Padding::ZERO.right(10.0)),
                container(self.face(u.and_then(|u| u.img_url.as_deref()), name, 20.0, 0.5))
                    .padding(Padding::ZERO.right(8.0)),
                med(name.to_string(), 13.0).color(p.text),
            ]
            .align_y(Center);
            col.push(container(line).padding([4, 8]))
        });
        let more = (more > 0).then(|| trn("and %s more", "and %s more", more, &[&short_count(more)]));
        let list = list.push(more.map(|more| container(txt(more, 13.0)).padding([4, 8])));
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
        reaction_groups(ms).into_iter().fold(Row::new().spacing(look.gap), |r, (group, mut reactors)| {
            // Desktop's `Tfe` for each key of the group: a custom emoji's image, or the emoji.
            let mut keys: Vec<&Reaction> = Vec::new();
            for x in &reactors {
                if keys.iter().all(|k| k.reaction_key != x.reaction_key) {
                    keys.push(x);
                }
            }
            let glyphs = keys.iter().map(|x| self.reaction_glyph(x, 16.0, 15.0));
            // Your own reaction shows first among the faces.
            reactors.sort_by_key(|x| !st.is_me(&x.participant_id));
            let n = reactors.len();
            let own = reactors.iter().any(|x| st.is_me(&x.participant_id));
            let mut pill = Row::with_children(glyphs.collect::<Vec<El<'a>>>()).spacing(4).align_y(Center);
            if look.group || n > 1 {
                // `.message-reaction-avatars`: each face 6 px over the one after it, the first on top.
                let faces = reactors.iter().take(3).enumerate().rev().map(|(k, x)| {
                    let u = chat.participants.items.iter().find(|u| u.id == x.participant_id);
                    let name = u.and_then(|u| u.full_name.as_deref()).unwrap_or(&x.participant_id);
                    pin(self.face(u.and_then(|u| u.img_url.as_deref()), name, 16.0, 0.5)).x(10.0 * k as f32).into()
                });
                let shown = n.min(3) as f32;
                pill = pill.push(Stack::with_children(faces).width(16.0 + 10.0 * (shown - 1.0)).height(16.0));
                if n > 3 {
                    // `.message-reaction-count`: `label-xs`, semibold.
                    let count = semi(format!("+{}", n - 3), 10.0).line_height(Pixels(rem(12.0)));
                    pill = pill.push(count.color(look.fg.1));
                }
            }
            let fill = if own { look.own } else { look.other };
            let hover = look.hover.unwrap_or(fill);
            let style = bordered_btn([fill, hover, hover], [look.fg.0; 3], look.ring.0, look.ring.1, 14.0);
            let pill = button(pill).padding([4.0 + look.ring.1, 5.0 + look.ring.1]).style(style);
            let pill = pill.on_press(toggle_reaction(st, chat, ms, &group));
            // Desktop's `.message-reaction-container` of a custom emoji: its right click has the item that adds it.
            let custom = keys.first().and_then(|x| Some((x.reaction_key.clone(), x.img_url.clone()?)));
            match (custom.filter(|(_, u)| crate::emoji::is_mxc(u)), ms.first()) {
                (Some((key, url)), Some(m)) => {
                    let menu = UiMessage::Message(Msg::ReactionMenu(m.id.clone(), key, url));
                    r.push(mouse_area(pill).on_right_press(menu))
                }
                _ => r.push(pill),
            }
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
    /// A right click on a link of a message, by its `href`.
    LinkMenu(String, String),
    /// A right click on the quote in a message, by the quoted message.
    LinkedMenu(String, String),
    /// A right click on a shown image of a message.
    ImageMenu(String),
    /// A right click on a custom emoji in the text of a message: its `title` and its image.
    EmoticonMenu(String, Option<String>, String),
    /// A right click on a reaction of a message with a custom emoji's image: its key (the shortcode) and the image.
    ReactionMenu(String, String, String),
    /// Desktop's `addCustomEmoji` of the message menu: the name and the image.
    AddCustomEmoji(String, String),
    /// The write of the own image pack after an emoji was added, by its name, and the pack before it.
    CustomEmojiAdded(String, serde_json::Value, Result<(), String>),
    /// Desktop's `createThreadWithPhoneNumber` and `createThreadWithEmail` of a contact card.
    MessageContact(String, Box<User>),
    /// Select the message, or unselect it when it is selected.
    Click(String),
    Select(Vec<String>),
    /// Copy the text of the selected messages, with their timestamps or not.
    CopySelected(bool),
    /// Desktop's `quoteAndReply` of the selected messages: each line quoted in the composer.
    QuoteSelected,
    /// Do the "On double-clicking a message" action.
    DoubleClick(String),
    /// Select the message above (`false`) or below (`true`) the selected one, as with the arrow keys.
    /// Select the next or the previous message, or with `extend`, add it to the selection.
    Step(bool, bool),
    /// Open the emoji picker at the cursor to react to a message.
    React(String),
    /// Open the emoji picker at a point to react to a message.
    ReactAt(String, Point),
    /// Open the emoji picker at the top left of a message to react to it, like Desktop's
    /// `showReactionMenuForMessage` of React… and → on a selected message.
    ReactOn(String),
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
    /// The text of the "Downloading" toast, and the path of the last saved file, none when the user cancelled the
    /// save dialog, or the URL of the file that failed and why.
    Downloaded(String, Result<Option<std::path::PathBuf>, (String, String)>),
    /// Desktop's "Delete message?" dialog of the messages.
    AskDelete(Vec<String>),
    /// Desktop's `retryErroredMessage` and `removeErroredMessage` of a failed send.
    RetrySend(String),
    RemoveUnsent(String),
    ToggleEditHistory(String),
    /// The play or pause button of a GIF of a message, by its attachment, while "Autoplay GIFs" is off.
    ToggleGif(String, usize),
    /// The play or pause button of the image of a link preview of a message, by its URL.
    ToggleLinkGif(String, String),
    /// "Show more" or "Show less" of a message over 40 lines.
    ToggleClamp(String),
    /// A press of an action button, by its message and its ID.
    PressButton(String, String),
    /// The end of the callback of an action button.
    Pressed(String, String, Pressed),
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
                    // A custom emoji reacts with its image again.
                    let img = stored(&st, &mid)
                        .reactions
                        .iter()
                        .find(|r| r.reaction_key == key)
                        .and_then(|r| r.img_url.clone());
                    st.add_reaction(&cid, &mid, &key, img, None).expect("add reaction");
                    st.add_recent_emoji(&key);
                    if let Some(c) = st.matrix.clone() {
                        let content = serde_json::json!({ "recent_emoji": st.recent_emoji });
                        let write = beeper_core::matrix::set_global(c, "io.element.recent_emoji", content);
                        return Task::perform(write, UiMessage::Pushed);
                    }
                }
            }
            Msg::ToggleBookmark(mid) => {
                self.list.chat_menu = None;
                self.store.write().toggle_bookmark(&mid);
            }
            Msg::MarkUnreadFromHere(cid, mid) => {
                let mut st = self.store.write();
                st.mark_unread_from_message(&cid, &mid);
                // Desktop's `preventAutoMarkAsRead`: the open chat stays unread until you leave it.
                let open = self.pane.selected_chat_id.as_deref().and_then(|c| st.resolve_chat_idx(c));
                self.pane.keep_unread |= open.is_some() && open == st.resolve_chat_idx(&cid);
                drop(st);
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
            Msg::ShowReplies(id) => {
                (self.pane.selected_msgs, self.pane.reply_thread) = (Vec::new(), Some(id));
                self.pane.reply_opened = Some(std::time::Instant::now());
            }
            Msg::HideReplies => (self.pane.selected_msgs, self.pane.reply_thread) = (Vec::new(), None),
            Msg::EndPoll(chat, mid) => self.store.write().end_poll(&chat, &mid),
            Msg::OpenPollVotes(chat, mid) => {
                if let Some(w) = self.vote_windows.iter().find(|w| w.2 == mid) {
                    return self.raise(w.0);
                }
                let (id, open) = self.open_child(Size::new(342.0, 563.0));
                self.vote_windows.push((id, chat, mid));
                return open;
            }
            Msg::LinkMenu(mid, href) if href.starts_with("https://") || href.starts_with("mailto:") => {
                // Desktop's menu of a link leaves the selection, and has the link's own items.
                let open = self.update(UiMessage::Message(Msg::OpenMenu(mid.clone())));
                self.pane.menu_selection = None;
                if !self.pane.selected_msgs.contains(&mid) {
                    self.pane.selected_msgs = vec![mid];
                }
                self.pane.menu_link = Some(href);
                return open;
            }
            Msg::LinkMenu(mid, href) if href.starts_with(crate::MENTION) => {
                return self.update(UiMessage::Message(Msg::OpenMenu(mid)));
            }
            // Desktop leaves the right click on another link to Electron, which has no menu for it.
            Msg::LinkMenu(..) => {}
            Msg::MessageContact(account, user) => return self.open_dm(account, *user, None, None),
            Msg::LinkedMenu(mid, linked) => {
                let open = self.update(UiMessage::Message(Msg::OpenMenu(mid)));
                self.pane.menu_linked = Some(linked);
                return open;
            }
            Msg::ImageMenu(mid) => {
                let open = self.update(UiMessage::Message(Msg::OpenMenu(mid)));
                self.pane.menu_image = true;
                return open;
            }
            // Desktop's `selectedImage` with `dataset.mxEmoticon`: the name is its `title` without the first and the
            // last character (the colons of ":name:").
            Msg::EmoticonMenu(mid, title, src) => {
                let open = self.update(UiMessage::Message(Msg::OpenMenu(mid)));
                self.pane.menu_image = true;
                let name = title.map(|t| t.chars().skip(1).collect::<String>()).map(|t| {
                    let mut t = t;
                    t.pop();
                    t
                });
                self.pane.menu_emoticon = name.map(|n| (n, src));
                return open;
            }
            // Desktop's `selectedReaction` with an image (`dataset.reactionImgUrl`) and its key.
            Msg::ReactionMenu(mid, key, url) => {
                let open = self.update(UiMessage::Message(Msg::OpenMenu(mid)));
                self.pane.menu_emoticon = Some((key, url));
                return open;
            }
            Msg::AddCustomEmoji(name, url) => {
                let mut st = self.store.write();
                match beeper_core::matrix::with_custom_emoji(&st.image_pack, &name, &url) {
                    Err(e) => {
                        drop(st);
                        self.show(Toast::new(trf("Couldn't add the emoji: %s", &[&e])));
                    }
                    // Desktop's `set` shows the pack at once, and writes it.
                    Ok(pack) => {
                        let old = std::mem::replace(&mut st.image_pack, pack.clone());
                        let Some(c) = st.matrix.clone() else {
                            drop(st);
                            return self.update(UiMessage::Message(Msg::CustomEmojiAdded(name, old, Ok(()))));
                        };
                        let write = beeper_core::matrix::set_global(c, beeper_core::matrix::IMAGE_PACK, pack);
                        return Task::perform(write, move |r| {
                            UiMessage::Message(Msg::CustomEmojiAdded(name.clone(), old.clone(), r))
                        });
                    }
                }
            }
            Msg::CustomEmojiAdded(name, _, Ok(())) => self.show(Toast::new(trf("Added emoji :%s:", &[&name]))),
            // Desktop's `setAccountData` puts the old pack back with its own toast, then the menu shows its own.
            Msg::CustomEmojiAdded(_, old, Err(e)) => {
                self.store.write().image_pack = old;
                self.fail(Toast::new(format!("Settings couldn't be saved: {e}")));
                self.show(Toast::new(trf("Couldn't add the emoji: %s", &[&e])));
            }
            Msg::OpenMenu(mid) => {
                (self.pane.menu_link, self.pane.video_menu, self.pane.menu_linked) = (None, false, None);
                (self.pane.menu_image, self.pane.menu_emoticon) = (false, None);
                // With text selected, Desktop leaves the menu to Electron's, which offers the selection.
                self.pane.menu_selection = crate::selectable::has_selection().then(crate::selectable::text);
                // Desktop's `selectOnly`: a right click on a message outside the selection selects it alone.
                if self.pane.menu_selection.is_none() && !self.pane.selected_msgs.contains(&mid) {
                    self.pane.selected_msgs = vec![mid.clone()];
                }
                self.video.iter_mut().for_each(|v| v.sub = false);
                (self.audio.sub, self.audio.lang, self.pane.remind_sub) = (false, false, false);
                self.pane.message_menu = Some((mid, self.cursor.get()));
                self.pane.gallery_menu.clear();
            }
            Msg::Select(mids) => self.pane.selected_msgs = mids,
            Msg::QuoteSelected => {
                let st = self.store.read();
                let (msgs, _) = self.timeline(&st);
                let picked: Vec<&Message> =
                    msgs.into_iter().filter(|m| self.pane.selected_msgs.contains(&m.id)).collect();
                let quote: Vec<String> = picked
                    .iter()
                    .flat_map(|m| {
                        crate::copy_text(m, false, true).lines().map(|l| format!("> {l}")).collect::<Vec<_>>()
                    })
                    .filter(|l| l.len() > 2)
                    .collect();
                drop(st);
                if quote.is_empty() {
                    return Task::none();
                }
                let text = format!("{}{}\n\n", self.pane.composer.text(), quote.join("\n"));
                return self.update(UiMessage::Composer(composer::Msg::Changed(text)));
            }
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
            Msg::Click(_) if crate::selectable::has_selection() => {}
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
            // Desktop's double-click action waits for the selection: a double click on the text selects a word.
            Msg::DoubleClick(_) if crate::selectable::has_selection() => {}
            Msg::DoubleClick(mid) => {
                let st = self.store.read();
                let action = st.settings.pref("messageDoubleClick");
                // Desktop's `$`: the chat takes reactions; a quick reaction also needs a message that takes one.
                let react = st.loaded(&mid).is_some_and(|m| st.chat(&m.chat_id).messages.react);
                let reactable = react && st.loaded(&mid).is_some_and(Message::reactable);
                drop(st);
                return match action {
                    1 if !react => Task::none(),
                    5 if !reactable => Task::none(),
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
                    // Desktop's ↓ with nothing selected goes to the first message after the unread divider.
                    (None, true) => self.first_unread(&msgs).and_then(|u| (u..msgs.len()).find(live)),
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
                self.follow_member_selection();
                return match self.pane.selected_msgs.last() {
                    Some(mid) => {
                        self.pane.timeline.reveal(vlist::key(mid));
                        locate(format!("msg-{mid}"), self.pane.scroll_id(), crate::Align::Nearest)
                    }
                    None => operation::focus("composer-input"),
                };
            }
            Msg::React(mid) => return self.open_emoji(Some((mid, self.cursor.get()))),
            // A message off screen opens it at the cursor.
            Msg::ReactOn(mid) => {
                let find = iced::widget::selector::find(iced::widget::Id::from(format!("msg-{mid}")));
                return find.map(move |t| match t.and_then(|t| t.visible_bounds()) {
                    Some(b) => UiMessage::Message(Msg::ReactAt(mid.clone(), b.position())),
                    None => UiMessage::Message(Msg::React(mid.clone())),
                });
            }
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
                        let px = beeper_core::media::decode_upright(&bytes)?.into_rgba8();
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
                let main = self.raise(self.main_window.expect("the main window is open"));
                // Desktop's `jumpToMessage` of the media viewer selects the message and does not flash it.
                return Task::batch([self.reveal(chat, mid, false), main]);
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
                        let name = attachment_name(a);
                        (a.src_url.clone().expect("an attachment has a source"), name)
                    })
                    .collect();
                let client = st.matrix.clone();
                let busy = trf("Downloading %s", &[&files[0].1]);
                self.status_banner = busy.clone();
                self.sticky_banner = Some(busy.clone());
                let (dir, ask) = (downloads_dir(&st.settings), st.settings.on("alwaysAskDownload"));
                return Task::perform(
                    async move {
                        let mut last = None;
                        for (url, name) in files {
                            let save = async {
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
                                            None => return Ok(None),
                                        }
                                    }
                                    false => {
                                        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                                        unused_path(&dir, &name)
                                    }
                                };
                                let bytes = beeper_core::matrix::media(client.clone(), url.clone()).await?;
                                std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
                                Ok(Some(path))
                            };
                            if let Some(path) = save.await.map_err(|e| (url.clone(), e))? {
                                last = Some(path);
                            }
                        }
                        Ok(last)
                    },
                    move |r| UiMessage::Message(Msg::Downloaded(busy.clone(), r)),
                );
            }
            // Desktop's `downloadFile` removes its "Downloading" toast, then shows the saved file, or why it failed.
            Msg::Downloaded(busy, r) => {
                if self.sticky_banner.take_if(|t| *t == busy).is_some() && self.status_banner == busy {
                    self.status_banner.clear();
                }
                match r {
                    Ok(None) => {}
                    Ok(Some(path)) => {
                        let text = trf("Downloaded %s", &[&path.file_name().expect("a file path").to_string_lossy()]);
                        let open = UiMessage::OpenUrl(path.to_string_lossy().into_owned());
                        let buttons = vec![(tr("Open"), open), ("Show in folder", UiMessage::ShowInFolder(path))];
                        self.show(Toast { buttons, ..Toast::new(text) });
                    }
                    Err((url, e)) => {
                        // Like Desktop's media manager, a file that the bridge only loads when the user asks needs a
                        // download.
                        if beeper_core::matrix::reload_needed(&e) {
                            self.needs_download.insert(url);
                        }
                        self.fail(Toast::new(trf("Download failed: %s", &[&e])));
                    }
                }
            }
            Msg::AskDelete(mids) => {
                (self.confirm, self.delete_target) = (Some(ASK_DELETE_MESSAGE), Some(mids.join(" ")))
            }
            Msg::RetrySend(id) => {
                self.store.write().retry_send(&id);
                // Desktop's `retryErroredMessage` closes the error toast of the send.
                self.remove_toast(&id);
                self.retried.insert(id);
            }
            Msg::RemoveUnsent(id) => {
                self.store.write().remove_unsent(&id);
                // So does its `removeErroredMessage`.
                self.remove_toast(&id);
            }
            Msg::ToggleClamp(mid) => {
                if !self.pane.unclamped.remove(&mid) {
                    self.pane.unclamped.insert(mid);
                }
            }
            Msg::PressButton(mid, id) => return self.press_button(mid, id),
            // Desktop's `z4t`: a toast when the homeserver answered 429, and an error toast when the callback failed.
            Msg::Pressed(mid, id, r) => {
                self.pressing.remove(&(mid, id));
                match r {
                    Pressed::Ok => {}
                    Pressed::RateLimited => {
                        self.show(Toast::new(tr("We're on it. If nothing arrives, try again in a few minutes.").into()))
                    }
                    Pressed::Failed => self.fail(Toast::new(tr("Couldn't reach Beeper. Try again.").into())),
                }
            }
            Msg::ToggleGif(mid, ai) => {
                if self.pane.played_gifs.remove(&(mid.clone(), ai)).is_none() {
                    self.pane.played_gifs.insert((mid, ai), std::time::Instant::now());
                }
            }
            Msg::ToggleLinkGif(mid, url) => {
                if self.pane.played_links.remove(&(mid.clone(), url.clone())).is_none() {
                    self.pane.played_links.insert((mid, url), std::time::Instant::now());
                }
            }
            Msg::ToggleEditHistory(mid) => {
                self.pane.expanded_history_msg_id =
                    (self.pane.expanded_history_msg_id.as_ref() != Some(&mid)).then_some(mid);
            }
            Msg::HoverEnter(id) => {
                self.quick_open &= self.pane.hovered_msg.as_ref() == Some(&id);
                if self.pane.hovered_msg.as_ref() != Some(&id) {
                    (self.pane.hover_msg_at, self.pane.flyover_at) = (Some(std::time::Instant::now()), None);
                }
                self.pane.hovered_msg = Some(id);
            }
            Msg::ShowQuickReactions if !self.quick_open => {
                self.quick_open = true;
                self.pane.flyover_at = Some(std::time::Instant::now());
            }
            Msg::ShowQuickReactions => {}
            Msg::HoverExit(id) => {
                if self.pane.hovered_msg.as_ref() == Some(&id) {
                    self.pane.hovered_msg = None;
                }
            }
        }
        self.settle()
    }

    /// Desktop's `W4t` press: a link button opens its link, and a callback button posts once at a time, with its
    /// spinner for 500 ms at least.
    fn press_button(&mut self, mid: String, id: String) -> Task<UiMessage> {
        let st = self.store.read();
        let home = &st.beeper_state.homeserver;
        let live = st.loaded(&mid).filter(|m| m.is_deleted != Some(true) && system_user(&m.sender_id));
        let b = live.and_then(|m| m.buttons.iter().find(|b| b.id == id));
        let link = b.filter(|b| b.action == ButtonAction::Link).and_then(ActionButton::link);
        let url = b.filter(|b| b.action == ButtonAction::Callback).and_then(|b| b.callback_url(home));
        let token = st.beeper_state.access_token.clone();
        drop(st);
        if let Some(link) = link {
            return match link.starts_with(crate::deeplink::SCHEME) {
                true => self.open_link(&link),
                false => self.update(UiMessage::OpenUrl(link)),
            };
        }
        let Some(url) = url else { return Task::none() };
        if !self.pressing.insert((mid.clone(), id.clone())) {
            return Task::none();
        }
        let press = async move {
            let wait = tokio::time::sleep(std::time::Duration::from_millis(500));
            tokio::join!(beeper_core::matrix::press_button(url, token), wait).0
        };
        Task::perform(press, move |r| UiMessage::Message(Msg::Pressed(mid, id, r)))
    }
}

impl BeeperGuiApp {
    /// Desktop's `ScheduledMessages`: the canceled messages, the "Send Later" toggle, then the messages to send.
    /// Desktop's `Poll` message, for a chat that can vote.
    pub(crate) fn view_poll<'a>(
        &'a self,
        st: &BeeperStore,
        chat: &Chat,
        m: &Message,
        poll: &PollData,
        (bg, fg): (Color, Color),
    ) -> El<'a> {
        let p = &self.pal;
        let mine = m.is_sender == Some(true);
        let (sub, footer, fill, fill_mine, ring) = match mine {
            true => (fg.scale_alpha(0.6), fg, fg.scale_alpha(0.22), fg.scale_alpha(0.38), fg.scale_alpha(0.85)),
            false => (p.on_secondary, p.accent, fg.scale_alpha(0.14), fg.scale_alpha(0.24), fg.scale_alpha(0.4)),
        };
        let total = poll.voters();
        let picks = poll.picks(&st.me().id);
        let shown = !poll.undisclosed || poll.ended;
        let multi = poll.max_selections != 1;
        let full = multi && picks.len() >= poll.max_selections as usize;
        let spread = if shown { total > 0 } else { !picks.is_empty() };
        let label_alpha = if poll.ended { 0.55 } else { 1.0 };
        let votes = trn("%d vote", "%d votes", total, &[&total]);
        let status = match (poll.ended, multi) {
            (true, _) => tr("Poll ended"),
            (_, true) => tr("Select one or more options"),
            _ => tr("Select one option"),
        };
        let question = if poll.question.is_empty() { tr("Poll").into() } else { poll.question.clone() };
        let results = tr("Results will be displayed when the poll ends");
        // Desktop's `.Poll-module__wrapper` is as wide as its content, from 240 to 380 px with its 14 px sides.
        let width = {
            let (semi_w, txt_w) =
                (|s: &str| text_width(s, 13.0, Weight::Semibold), |s| text_width(s, 13.0, Weight::Normal));
            let side = if spread { 16.0 } else { 8.0 };
            let rows = poll.options.iter().map(|o| {
                let n = o.voter_ids.len();
                let faces = 23.0 + 14.0 * n.clamp(1, 3).saturating_sub(1) as f32;
                let right = if shown && n > 0 { 10.0 + faces + 6.0 + semi_w(&n.to_string()) } else { 0.0 };
                side + 16.0 + 8.0 + txt_w(&o.label) + right
            });
            let foot = match shown {
                true if total > 0 => semi_w(&votes) + 16.0,
                true => 0.0,
                false => text_width(results, 11.0, Weight::Normal).max(if total > 0 { semi_w(&votes) } else { 0.0 }),
            };
            let head = [text_width(&question, 15.0, Weight::Semibold), 22.0 + txt_w(status), foot];
            (rows.chain(head).fold(0.0, f32::max) + 28.0).clamp(240.0, 380.0)
        };
        let head = column![
            semi(question, 15.0).line_height(Pixels(rem(20.0))).color(fg),
            row![
                tinted(if poll.ended { &POLL_ENDED } else { &COMPOSER_ICONS[8] }, 16.0, sub),
                txt(status, 13.0).color(sub)
            ]
            .spacing(6)
            .align_y(Center),
        ]
        .spacing(4);
        let options = poll.options.iter().map(|o| -> El<'a> {
            let n = o.voter_ids.len();
            let voted = picks.contains(&o.id);
            let width = match (shown, if shown { n > 0 } else { voted }) {
                (_, false) => 0,
                (true, true) => u16::try_from(n * 100 / total).expect("a percentage"),
                (false, true) => 100,
            };
            let bar = container(Space::new()).height(Fill).style(fill_box(if voted { fill_mine } else { fill }, 16.0));
            let bar: El<'a> = match width {
                0 => Space::new().into(),
                100 => bar.width(Fill).into(),
                w => row![bar.width(Length::FillPortion(w)), Space::new().width(Length::FillPortion(100 - w))].into(),
            };
            let (lf, ring) = (fg.scale_alpha(label_alpha), ring.scale_alpha(label_alpha));
            let radius = if multi { 4.0 } else { 8.0 };
            let check: El<'a> = match voted {
                true => container(tinted(&POLL_CHECK, 16.0, bg)).style(fill_box(lf, radius)).into(),
                false => {
                    container(Space::new().width(16).height(16)).style(bordered_box(CLEAR, ring, 1.5, radius)).into()
                }
            };
            let name = txt(o.label.clone(), 13.0).color(lf);
            let faces = o.voter_ids.iter().rev().take(3).rev().enumerate().fold(Stack::new(), |s, (i, id)| {
                let u = chat.participants.items.iter().find(|u| u.id == *id);
                let name = u.and_then(|u| u.full_name.as_deref()).unwrap_or(id);
                let face = container(self.face(u.and_then(|u| u.img_url.as_deref()), name, 20.0, 0.5))
                    .padding(1.5)
                    .style(fill_box(bg, 11.5));
                s.push(container(face).padding(Padding::ZERO.left(14.0 * i as f32)))
            });
            let right =
                (shown && n > 0).then(|| row![faces, semi(n.to_string(), 13.0).color(fg)].spacing(6).align_y(Center));
            let line =
                row![row![check, name].spacing(8).align_y(Center).width(Fill)].push(right).spacing(10).align_y(Center);
            let answers = match (multi, voted) {
                (true, true) => picks.iter().filter(|x| **x != o.id).cloned().collect(),
                (true, false) => picks.iter().cloned().chain([o.id.clone()]).collect(),
                (false, true) => Vec::new(),
                (false, false) => vec![o.id.clone()],
            };
            let vote =
                (!poll.ended && !(full && !voted)).then(|| UiMessage::Message(Msg::VotePoll(m.id.clone(), answers)));
            let body = container(line).padding([8.0, if spread { 8.0 } else { 4.0 }]);
            button(stack![body].push_under(bar))
                .padding(0)
                .width(Fill)
                .style(btn_style([CLEAR; 3], [fg; 3], 16.0))
                .on_press_maybe(vote)
                .into()
        });
        let options = Column::with_children(options).spacing(if spread { 6 } else { 2 });
        let foot: El<'a> = match shown {
            true if total > 0 => button(row![semi(votes, 13.0), tinted(&CHEVRON_RIGHT, 16.0, footer)].align_y(Center))
                .bare(footer)
                .on_press(UiMessage::Message(Msg::OpenPollVotes(chat.id.clone(), m.id.clone())))
                .into(),
            true => Space::new().into(),
            false => column![]
                .push((total > 0).then(|| semi(votes, 13.0).color(fg)))
                .push(txt(results, 11.0).color(sub))
                .spacing(4)
                .into(),
        };
        let foot = (!shown || total > 0).then(|| container(foot).padding(Padding::ZERO.top(10.0)));
        let body = column![head, Space::new().height(10), options].push(foot);
        container(body).padding(Padding::new(14.0).bottom(12.0)).width(width).style(fill_box(bg, 17.0)).into()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use iced::window;
    use iced_test::simulator::simulator;

    /// Desktop's right click on a selection: it acts on all the selected messages, and a right click outside the
    /// selection selects that message alone.
    #[test]
    fn menu_of_a_selection_like_desktop() {
        let mut app = crate::fuzz::demo_app("selection-menu");
        let _ = app.update(UiMessage::SelectChat("!note_self:beeper.com".into()));
        let ids: Vec<String> = {
            let st = app.store.read();
            app.timeline(&st).0.iter().rev().take(3).map(|m| m.id.clone()).collect()
        };
        app.pane.selected_msgs = ids[..2].to_vec();
        let _ = app.update(UiMessage::Message(Msg::OpenMenu(ids[0].clone())));
        assert_eq!(app.pane.selected_msgs, ids[..2], "the selection stays");
        {
            let (menu, _, _) = app.view_message_menu(&ids[0]);
            let mut sim = simulator(menu);
            assert!(sim.find("Quote reply 2 messages…").is_ok() && sim.find("Copy with Timestamp").is_ok());
        }
        let _ = app.update(UiMessage::Message(Msg::OpenMenu(ids[2].clone())));
        assert_eq!(app.pane.selected_msgs, [ids[2].clone()], "a message outside the selection is selected alone");
        app.pane.selected_msgs = ids[..2].to_vec();
        let _ = app.update(UiMessage::Message(Msg::QuoteSelected));
        assert!(app.pane.composer.text().starts_with("> "), "the selection is quoted in the composer");
    }

    /// Desktop's `downloadFile`: the "Downloading" toast goes when the file is saved, and the "Downloaded" toast opens
    /// the file or shows it in its folder. A cancelled save dialog shows nothing, and a failure is an error toast.
    #[test]
    fn download_toasts_like_desktop() {
        let mut app = crate::fuzz::demo_app("download-toasts");
        let busy = "Downloading a.png".to_string();
        let start =
            |app: &mut BeeperGuiApp| (app.status_banner, app.sticky_banner) = (busy.clone(), Some(busy.clone()));
        let path = std::path::PathBuf::from("/tmp/saved/a (1).png");
        start(&mut app);
        let _ = app.update(UiMessage::Message(Msg::Downloaded(busy.clone(), Ok(Some(path.clone())))));
        assert_eq!(app.sticky_banner, None, "the Downloading toast is gone, and New Chat can start a chat again");
        let t = app.shown_toast().expect("the Downloaded toast");
        assert_eq!(t.text, "Downloaded a (1).png");
        assert!(
            matches!(&t.buttons[..], [("Open", UiMessage::OpenUrl(o)), ("Show in folder", UiMessage::ShowInFolder(s))]
            if o == "/tmp/saved/a (1).png" && *s == path)
        );
        start(&mut app);
        let _ = app.update(UiMessage::Message(Msg::Downloaded(busy.clone(), Ok(None))));
        assert_eq!((app.status_banner.as_str(), app.sticky_banner.as_deref()), ("", None), "a cancel says nothing");
        start(&mut app);
        let failed = Err(("mxc://x/y".into(), "boom".into()));
        let _ = app.update(UiMessage::Message(Msg::Downloaded(busy.clone(), failed)));
        let t = app.shown_toast().expect("the failure toast");
        assert!(t.error && t.text == "Download failed: boom");
        // A toast that waits for an answer, like a verification request, stays.
        (app.status_banner, app.sticky_banner) = ("Verification request from Pixel".into(), Some("Pixel".into()));
        let _ = app.update(UiMessage::Message(Msg::Downloaded(busy, Ok(Some(path)))));
        assert_eq!(app.sticky_banner.as_deref(), Some("Pixel"));
    }

    /// Desktop's `.reply-thread` fades in over 0.2 s while its messages rise 10 px into place over 0.6 s, both on
    /// `cubic-bezier(.165, .84, .44, 1)`, and a click on it still closes it while it comes in.
    #[test]
    fn reply_chain_fades_in_like_desktop() {
        let near = |(a, b): (f32, f32), (x, y): (f32, f32)| (a - x).abs() < 1e-3 && (b - y).abs() < 1e-3;
        assert!(near(replies_reveal(0.0), (0.0, 10.0)), "it starts clear and 10 px low");
        let (shown, below) = replies_reveal(0.1);
        assert!(shown > 0.5 && shown < 1.0 && below > 3.0, "the fast curve: {shown}, {below}");
        let (shown, below) = replies_reveal(0.2);
        assert!(shown > 0.999 && below > 0.0 && below < 3.0, "the background is in before the messages: {below}");
        assert!(near(replies_reveal(REPLIES_RISE.as_secs_f32()), (1.0, 0.0)), "all in place after 0.6 s");
        let mut app = crate::fuzz::demo_app("reply-fade");
        let _ = app.update(UiMessage::SelectChat("!note_self:beeper.com".into()));
        let mid = app.timeline(&app.store.read()).0.last().expect("a message").id.clone();
        let _ = app.update(UiMessage::Message(Msg::ShowReplies(mid)));
        assert!(app.pane.reply_opened.is_some_and(|t| t.elapsed() < REPLIES_RISE), "it starts to fade in");
        let size = iced::Size::new(1000.0, 720.0);
        let mut sim = iced_test::simulator::Simulator::with_size(Default::default(), size, app.view_main());
        sim.point_at(Point::new(700.0, 250.0));
        let _ = sim.simulate(iced_test::simulator::click());
        assert!(sim.into_messages().any(|m| matches!(m, UiMessage::Message(Msg::HideReplies))));
    }

    /// Desktop's link items: "Copy Link" and "Copy Entire Message" only for a right click on an `https://` link, the
    /// address items only for a `mailto:` one, and no menu for another link.
    #[test]
    fn menu_of_a_clicked_link_like_desktop() {
        let mut app = crate::fuzz::demo_app("link-menu");
        let _ = app.update(UiMessage::SelectChat("!note_self:beeper.com".into()));
        let mid = {
            let st = app.store.read();
            app.timeline(&st).0.iter().find(|m| !m.links.is_empty()).expect("a message with a preview").id.clone()
        };
        let finds = |app: &BeeperGuiApp, label: &str| simulator(app.view_message_menu(&mid).0).find(label).is_ok();
        let _ = app.update(UiMessage::Message(Msg::OpenMenu(mid.clone())));
        assert!(!finds(&app, "Copy Link") && finds(&app, "Copy") && !finds(&app, "Copy Entire Message"));
        let _ = app.update(UiMessage::Message(Msg::LinkMenu(mid.clone(), "https://a.io/x".into())));
        assert!(finds(&app, "Copy Link") && finds(&app, "Copy Entire Message"), "the link's items");
        assert_eq!(app.pane.selected_msgs, std::slice::from_ref(&mid));
        let _ = app.update(UiMessage::Message(Msg::LinkMenu(mid.clone(), "mailto:me@x.com".into())));
        assert!(finds(&app, "Copy me@x.com") && finds(&app, "Email me@x.com…") && !finds(&app, "Copy Link"));
        let _ = app.update(UiMessage::Message(Msg::OpenMenu(mid.clone())));
        assert!(!finds(&app, "Email me@x.com…"), "another right click drops the link");
        app.pane.message_menu = None;
        let _ = app.update(UiMessage::Message(Msg::LinkMenu(mid.clone(), "http://a.io".into())));
        assert!(app.pane.message_menu.is_none(), "Desktop has no menu for another link");
    }

    /// Desktop's custom emoji (`data-mx-emoticon`): images in the text instead of their `alt`, and a message of them
    /// alone is a jumbo emoji like one of emoji.
    #[test]
    fn custom_emoji_like_desktop() {
        let img = |alt: &str| format!(r#"<img data-mx-emoticon src="mxc://x/{alt}" alt=":{alt}:" height="32">"#);
        let mut app = crate::fuzz::demo_app("custom-emoji");
        let base = app.store.read().messages[0].clone();
        let msg = |html: String| Message { text: Some(":a:".into()), html: Some(html), ..base.clone() };
        assert!(jumbo(&emoji_text(&msg(format!("{} {}", img("a"), img("b"))))), "custom emoji alone are jumbo");
        assert!(jumbo(&emoji_text(&msg(format!("{}😀", img("a"))))), "with emoji too");
        assert!(!jumbo(&emoji_text(&msg(format!("hi {}", img("a"))))), "not with text");
        let _ = app.update(UiMessage::SelectChat("!note_self:beeper.com".into()));
        let mid = {
            let st = app.store.read();
            app.timeline(&st).0.last().expect("a message").id.clone()
        };
        {
            let mut st = app.store.write();
            let i = st.messages.iter().position(|m| m.id == mid).expect("the message");
            let m = &mut st.messages[i];
            (m.text, m.html) = (Some("hi :a:".into()), Some(format!("hi {}", img("a"))));
        }
        let mut sim = simulator(app.view_conversation());
        assert!(sim.find(":a:").is_err(), "the image takes the place of the alt text");
    }

    /// Desktop's flyover (`Yfe`): the quick reactions come once the pointer is on the flyover, only on received
    /// messages, and its items fade in one after another.
    #[test]
    fn flyover_quick_reactions_like_desktop() {
        let mut app = crate::fuzz::demo_app("flyover");
        let (chat, theirs, mine) = {
            let st = app.store.read();
            let pick = |c: &Chat| {
                let msgs = st.timeline_indices(&c.id).iter().map(|&i| &st.messages[i]).filter(|m| m.reactable());
                let msgs: Vec<&Message> = msgs.collect();
                let theirs = msgs.iter().rev().find(|m| m.is_sender != Some(true))?.id.clone();
                let mine = msgs.iter().rev().find(|m| m.is_sender == Some(true))?.id.clone();
                Some((c.id.clone(), theirs, mine))
            };
            st.chats.iter().filter(|c| c.messages.react).find_map(pick).expect("a chat with both senders")
        };
        let _ = app.update(UiMessage::SelectChat(chat));
        app.pane.size = Size::new(900.0, 700.0);
        let quick_shown = |app: &BeeperGuiApp| simulator(app.view_conversation()).find("👍").is_ok();
        let _ = app.update(UiMessage::Message(Msg::HoverEnter(theirs.clone())));
        assert!(app.pane.hover_msg_at.is_some() && !quick_shown(&app), "not before the flyover has the pointer");
        let _ = app.update(UiMessage::Message(Msg::ShowQuickReactions));
        assert!(app.pane.flyover_at.is_some() && quick_shown(&app), "then on a received message");
        let _ = app.update(UiMessage::Message(Msg::HoverEnter(mine)));
        let _ = app.update(UiMessage::Message(Msg::ShowQuickReactions));
        assert!(!quick_shown(&app), "never on your own");
    }

    /// Desktop's `ActionButtons` under a message of the system user: the buttons that can open their link or post to
    /// the homeserver, a deep link that opens, and a callback that runs once at a time, then a toast for a 429 and an
    /// error toast for a failure.
    #[test]
    fn system_user_buttons_like_desktop() {
        let mut app = crate::fuzz::demo_app("action-buttons");
        let path = "/_matrix/client/unstable/com.beeper.tips/download_links/email";
        let button = |id: &str, action, url: String| ActionButton {
            id: id.into(),
            label: format!("Label {id}"),
            action,
            url,
            icon: Some("mail".into()),
        };
        {
            let mut st = app.store.write();
            // Nothing answers there, so no callback leaves the machine.
            st.beeper_state.homeserver = "https://127.0.0.1:9".into();
            let m = st.messages[*st.timeline_indices(crate::NOTE_CHAT_ID).last().expect("a message")].clone();
            let buttons = vec![
                button("tips", ButtonAction::Link, "beeper://tips/add-account".into()),
                button("email", ButtonAction::Callback, format!("https://127.0.0.1:9{path}")),
                button("elsewhere", ButtonAction::Callback, format!("https://x.io{path}")),
                button("plain", ButtonAction::Link, "http://x.io".into()),
            ];
            let system = "@:beeper.local".to_string();
            let tip = Message { id: "$tip".into(), sender_id: system, is_sender: Some(false), buttons, ..m.clone() };
            st.upsert_message(tip);
            let fake = vec![button("fake", ButtonAction::Link, "https://x.io".into())];
            st.upsert_message(Message { id: "$fake".into(), is_sender: Some(false), buttons: fake, ..m });
        }
        let _ = app.update(UiMessage::SelectChat(crate::NOTE_CHAT_ID.into()));
        app.pane.size = Size::new(900.0, 700.0);
        let shown = |app: &BeeperGuiApp, label: &str| simulator(app.view_conversation()).find(label).is_ok();
        assert!(shown(&app, "Label tips") && shown(&app, "Label email"));
        assert!(!shown(&app, "Label elsewhere") && !shown(&app, "Label plain"), "only the usable buttons");
        assert!(!shown(&app, "Label fake"), "only the system user's messages have buttons");
        let mut sim = simulator(app.view_conversation());
        sim.click("Label tips").expect("the link button");
        let msgs: Vec<UiMessage> = sim.into_messages().collect();
        let press =
            |m: &UiMessage| matches!(m, UiMessage::Message(Msg::PressButton(mid, id)) if mid == "$tip" && id == "tips");
        assert!(msgs.iter().any(press), "a press of the button");
        msgs.into_iter().for_each(|m| _ = app.update(m));
        assert!(app.networks.window.is_some(), "the deep link of the tip opens the Add an Account window");
        let email = || UiMessage::Message(Msg::PressButton("$tip".into(), "email".into()));
        let _ = app.update(email());
        let _ = app.update(email());
        assert_eq!(app.pressing.len(), 1, "one callback at a time");
        let mut sim = simulator(app.view_conversation());
        let _ = sim.click("Label email");
        assert!(!sim.into_messages().any(|m| matches!(m, UiMessage::Message(Msg::PressButton(..)))), "disabled");
        let done = |r| UiMessage::Message(Msg::Pressed("$tip".into(), "email".into(), r));
        let _ = app.update(done(Pressed::RateLimited));
        assert!(app.pressing.is_empty());
        assert_eq!(app.status_banner, "We're on it. If nothing arrives, try again in a few minutes.");
        assert!(app.toast.as_ref().is_some_and(|t| !t.error));
        let _ = app.update(done(Pressed::Failed));
        assert_eq!(app.status_banner, "Couldn't reach Beeper. Try again.");
        assert!(app.toast.as_ref().is_some_and(|t| t.error));
        {
            let mut st = app.store.write();
            let i = st.loaded_idx("$tip").expect("the tip");
            st.update_message(i, |m| m.is_deleted = Some(true));
        }
        assert!(!shown(&app, "Label tips"), "a deleted message has no buttons");
    }

    /// HTML's `dir="auto"` takes the direction of the first strong character.
    #[test]
    fn text_direction_like_html() {
        assert!(super::right_to_left("مرحبا بالعالم") && super::right_to_left("12 שלום world"));
        assert!(!super::right_to_left("hello שלום") && !super::right_to_left("123 !?"));
    }

    /// Desktop's `DJ` splits the text around each "@room".
    #[test]
    fn room_parts_like_desktop() {
        assert_eq!(
            super::room_parts("hi @room, see @roommate"),
            [("hi ", false), ("@room", true), (", see ", false), ("@room", true), ("mate", false)]
        );
        assert_eq!(super::room_parts("no mention"), [("no mention", false)]);
    }

    /// Desktop's bridge request: no thread header, and a group request names who invited you on which network.
    #[test]
    fn request_head_names_the_inviter_like_desktop() {
        let mut app = crate::fuzz::demo_app("request-inviter");
        let group = {
            let st = app.store.read();
            let free = |c: &&Chat| c.chat_type == "group" && c.account_id != "hungryserv" && c.is_pinned != Some(true);
            st.chats.iter().find(free).expect("a bridged group").id.clone()
        };
        let inviter = {
            let mut st = app.store.write();
            let i = st.resolve_chat_idx(&group).expect("the group");
            let inviter =
                st.chats[i].participants.items.iter().find(|u| u.is_self != Some(true)).expect("a member").clone();
            let me = User {
                id: "@me:beeper.local".into(),
                username: Some("alex".into()),
                is_self: Some(true),
                added_by: Some(inviter.id.clone()),
                ..Default::default()
            };
            st.update_chat(i, |c| {
                (c.is_request, c.flags) = (Some(true), c.flags | CHAT_REQUEST);
                c.participants.items.retain(|u| u.is_self != Some(true));
                c.participants.items.push(me);
            });
            inviter.full_name.clone().expect("a name")
        };
        let _ = app.update(UiMessage::SelectChat(group.clone()));
        let title = app.store.read().chat(&group).title.clone();
        let mut sim = simulator(app.view_conversation());
        assert!(sim.find(inviter.as_str()).is_ok() && sim.find(" invited you (").is_ok(), "the inviter line");
        assert!(sim.find("Feedback?").is_err(), "the feedback button goes with the thread header");
        // Desktop's `RequestNotice` asks about the invitation from the person who added you.
        let ask = format!("Accept group invitation for \"{title}\" from ");
        assert!(sim.find(ask.as_str()).is_ok(), "{ask}");
    }

    /// Desktop's "Jump to Message" comes with a right click on a quote, which jumps to the quoted message, and not
    /// with a right click on the message elsewhere.
    #[test]
    fn jump_to_a_quoted_message_like_desktop() {
        let mut app = crate::fuzz::demo_app("jump-quote");
        let _ = app.update(UiMessage::SelectChat("!note_self:beeper.com".into()));
        let ids: Vec<String> = {
            let st = app.store.read();
            app.timeline(&st).0.iter().rev().take(2).map(|m| m.id.clone()).collect()
        };
        let has = |app: &BeeperGuiApp| simulator(app.view_message_menu(&ids[0]).0).find("Jump to Message").is_ok();
        let _ = app.update(UiMessage::Message(Msg::OpenMenu(ids[0].clone())));
        assert!(!has(&app));
        let _ = app.update(UiMessage::Message(Msg::LinkedMenu(ids[0].clone(), ids[1].clone())));
        assert!(has(&app) && app.pane.menu_linked.as_deref() == Some(ids[1].as_str()));
        let _ = app.update(UiMessage::Message(Msg::OpenMenu(ids[0].clone())));
        assert!(!has(&app), "another right click forgets the quote");
    }

    /// The pixels of a frame of `sim`, at its scale of 2. The simulator does not find the text of overlays.
    pub(crate) fn pixels(sim: &mut iced_test::Simulator<'_, UiMessage>) -> ::image::RgbaImage {
        crate::focus::tests::pixels(sim, &iced::Theme::Light, "msg")
    }

    /// Whether the pixel is `c`, give or take antialiasing.
    fn near(px: [u8; 4], c: Color) -> bool {
        let c = c.into_rgba8();
        (0..3).map(|i| px[i].abs_diff(c[i]) as u32).sum::<u32>() < 24
    }

    /// The message `mid` alone, the first and the last of its sender, in a pane 900 px wide.
    fn alone<'a>(app: &'a BeeperGuiApp, mid: &str) -> iced_test::Simulator<'a, UiMessage> {
        let st = app.store.read();
        let m = st.message(mid);
        let flags = (true, true, Chain::default(), false, false);
        let view = app.view_message(&st, st.chat(&m.chat_id), &[m], flags, (None, 0), Column::new());
        iced_test::Simulator::with_size(Default::default(), Size::new(900.0, 400.0), view)
    }

    /// Desktop's `replyLineSegments` (`C4t`): a curve into a reply that quotes the other side, a curve out of a
    /// message that its reply follows, loops along your own messages between replies of others, and lines along
    /// the rest of the chain.
    #[test]
    fn reply_line_segments_like_desktop() {
        use crate::reply_line::Shape::*;
        let line = |shape, top, bottom| Some(Segment::new(shape, top, bottom));
        let chain = |prev, next, next_mine| Chain { prev, next, next_mine, next_same: next_mine, next_day: false };
        // Your reply in a DM to a message of theirs ends the line with a curve into it.
        let (s, _) = chain(false, false, false).segments(true, Some(false), true, false, true);
        assert_eq!(s, Segments { body: line(End, true, true), ..Default::default() });
        // Their reply in a group to yours: a curve beside the quote, and a line down to the avatar.
        let (s, face) = chain(false, false, false).segments(true, Some(true), false, true, false);
        let quote = line(Start, false, false);
        assert_eq!(s, Segments { quote, above_face: line(Line, false, true), ..Default::default() }, "{s:?}");
        assert!(face, "the end of a chain shows the avatar");
        // Your message that your reply follows starts the line, which goes on along its footer.
        let (s, _) = chain(false, true, true).segments(false, None, true, false, false);
        assert_eq!(
            s,
            Segments { body: line(Start, true, false), footer: line(Line, false, false), ..Default::default() }
        );
        // Your message between replies of others loops.
        let (s, _) = chain(true, true, false).segments(false, None, true, true, true);
        assert_eq!(s.body, line(Loop, false, false));
        // A received message in a group without its avatar has the line in the avatar's place.
        let (s, face) = chain(true, true, false).segments(false, None, false, true, false);
        assert_eq!((s.body, face), (line(Line, false, false), false));
        assert_eq!(chain(false, false, false).segments(false, None, true, true, true).0, Segments::default());
    }

    /// Desktop draws the reply line along a chain of your own messages too, 15 px into the timeline.
    #[test]
    fn reply_lines_join_a_chain_like_desktop() {
        let mut app = crate::fuzz::demo_app("reply-lines");
        let _ = app.update(UiMessage::SelectChat("!note_self:beeper.com".into()));
        app.pane.size = Size::new(900.0, 700.0);
        // The last two messages, as plain text.
        let ids: Vec<String> = {
            let st = app.store.read();
            app.timeline(&st).0.iter().rev().take(2).map(|m| m.id.clone()).collect()
        };
        for id in &ids {
            change(&app, id, |m| {
                (m.text, m.html, m.linked_message_id, m.message_type) = (Some("Hi".into()), None, None, None);
                (m.attachments, m.links, m.poll, m.call, m.timer) = (vec![], vec![], None, None, None);
                (m.is_sender, m.is_deleted, m.reactions) = (Some(true), None, vec![]);
            });
        }
        // The rows of the timeline with the line's color 15 px in from its 8 px padding.
        let line_rows = |app: &BeeperGuiApp| {
            let st = app.store.read();
            let chat = st.chat("!note_self:beeper.com");
            let rows = app.conversation_rows(&st, chat);
            let msgs: Vec<&Message> = rows.msgs.iter().map(|&i| &st.messages[i]).collect();
            let view = app.view_messages(&st, chat, &msgs, &rows, false);
            let shot = pixels(&mut iced_test::Simulator::with_size(Default::default(), app.pane.size, view));
            (0..shot.height()).filter(|&y| near(shot.get_pixel(2 * (8 + 15) + 2, y).0, app.pal.reply_line)).count()
        };
        assert_eq!(line_rows(&app), 0, "no line without a chain");
        change(&app, &ids[0], |m| m.linked_message_id = Some(ids[1].clone()));
        assert!(line_rows(&app) > 2 * 22, "a line joins a message and your reply to it from curve to curve");
    }

    /// Desktop's `bfe` of a deleted message: "Message Deleted", or "Removed from Beeper", in a dashed box like the
    /// "Loading…" one of a message that did not load (`L1t`). A group shows the quoted sender's avatar beside a quote
    /// of someone else (`reply-avatar`).
    #[test]
    fn quotes_like_desktop() {
        let mut app = crate::fuzz::demo_app("quotes");
        app.pane.size = Size::new(900.0, 700.0);
        let (cid, quoted, reply) = {
            let mut st = app.store.write();
            let i = st.chats.iter().position(|c| c.chat_type == "group").expect("a group");
            let zed = User { id: "@zq:x".into(), full_name: Some("Zed Quinn".into()), ..Default::default() };
            st.update_chat(i, |c| c.participants.items.push(zed));
            let id = st.chats[i].id.clone();
            let ids: Vec<String> =
                st.timeline_indices(&id).iter().rev().take(2).map(|&k| st.messages[k].id.clone()).collect();
            (id, ids[1].clone(), ids[0].clone())
        };
        change(&app, &quoted, |m| {
            (m.sender_id, m.is_sender, m.attachments) = ("@zq:x".into(), Some(false), vec![]);
            m.text = Some("the quoted text".into());
        });
        change(&app, &reply, |m| m.linked_message_id = Some(quoted.clone()));
        assert_eq!(alone(&app, &reply).find("Z").expect("the reply avatar").bounds().height, 12.0, "24 px");
        // Whether the top edge of the box around `label` has gaps.
        let dashed = |app: &BeeperGuiApp, label: &str| {
            let mut sim = alone(app, &reply);
            let b = sim.find(label).expect(label).bounds();
            let shot = pixels(&mut sim);
            let y = ((b.y - 7.0) * 2.0).round() as u32;
            let edge: Vec<[u8; 4]> =
                ((b.x as u32 + 10) * 2..(b.x + b.width) as u32 * 2 - 20).map(|x| shot.get_pixel(x, y).0).collect();
            assert!(edge.iter().any(|&px| !near(px, app.pal.card)), "a border around {label}");
            edge.iter().any(|&px| px == app.pal.card.into_rgba8())
        };
        assert!(!dashed(&app, "Zed Quinn"), "a quote has a solid border");
        change(&app, &quoted, |m| m.is_deleted = Some(true));
        assert!(dashed(&app, "Message Deleted"));
        change(&app, &quoted, |m| m.removed = true);
        assert!(dashed(&app, "Removed from Beeper"));
        change(&app, &reply, |m| m.linked_message_id = Some("$nowhere".into()));
        assert!(dashed(&app, "Loading…"));
        change(&app, &reply, |m| m.linked_message_id = Some(quoted.clone()));
        {
            let mut st = app.store.write();
            let i = st.resolve_chat_idx(&cid).expect("the group");
            st.update_chat(i, |c| c.chat_type = "single".into());
        }
        assert!(alone(&app, &reply).find("Z").is_err(), "a DM has no reply avatar");
    }

    /// Desktop's `dir="auto"`: the wrapped lines of a right-to-left message start at the right of its bubble, so its
    /// ragged edge is on the left.
    #[test]
    fn rtl_lines_start_at_the_right_like_desktop() {
        let mut app = crate::fuzz::demo_app("rtl-lines");
        app.pane.size = Size::new(1000.0, 700.0);
        let mid =
            plain_received(&app, "שלום עולם, זו הודעה בעברית עם מילים רבות כדי לבדוק גלישה של שורות ארוכות בממשק");
        let shot = pixels(&mut alone(&app, &mid));
        // The rightmost columns of the bubble and of the dark pixels of its text, at the scale of 2.
        let rightmost = |hit: &dyn Fn([u8; 4]) -> bool| {
            (0..shot.width()).rev().find(|&x| (0..shot.height()).any(|y| hit(shot.get_pixel(x, y).0))).expect("pixels")
        };
        let bubble = rightmost(&|px| near(px, app.pal.received));
        let ink = rightmost(&|px| px[..3].iter().map(|&c| u32::from(c)).sum::<u32>() < 200);
        assert!(bubble - ink <= 2 * (BUBBLE_PAD_X as u32 + 2), "the lines end at the padding: {bubble} {ink}");
    }

    /// Desktop's quote of your own message in the light theme is a white box with the border of your bubbles and its
    /// text in the primary color, so it does not look like your message again. The dark theme and a chat bubble color
    /// mix 30% of the bubble color into the box, and a bubble color draws the text in the text color.
    #[test]
    fn own_quotes_like_desktop() {
        let mut app = crate::fuzz::demo_app("own-quotes");
        // Eric's reply quotes your message with a file and a text.
        let reply = "msg_1001";
        let text = app.store.read().message("msg_1000b").text.clone().expect("a text");
        assert_eq!(app.store.read().message(reply).linked_message_id.as_deref(), Some("msg_1000b"));
        // The box beside the quoted text and the color of its text that differs the most from the box.
        let look = |app: &BeeperGuiApp| {
            let mut sim = alone(app, reply);
            let b = sim.find(text.as_str()).expect("the quoted text").bounds();
            let shot = pixels(&mut sim);
            let fill = shot.get_pixel((2.0 * (b.x - 3.0)) as u32, (2.0 * b.center_y()) as u32).0;
            let off = |px: &[u8; 4]| (0..3).map(|i| u32::from(px[i].abs_diff(fill[i]))).sum::<u32>();
            let ink = (2 * b.y as u32..2 * (b.y + b.height) as u32)
                .flat_map(|y| (2 * b.x as u32..2 * (b.x + 60.0) as u32).map(move |x| (x, y)))
                .map(|(x, y)| shot.get_pixel(x, y).0)
                .max_by_key(off)
                .expect("the text");
            (fill, ink)
        };
        let (fill, ink) = look(&app);
        assert!(near(fill, app.pal.card), "a white box: {fill:?}");
        assert!(near(ink, app.pal.accent), "the text in the primary color: {ink:?}");
        app.dark = true;
        app.pal = crate::style::palette(true);
        let (fill, ink) = look(&app);
        assert!(near(fill, mix(app.pal.card, app.pal.sent, 0.3)), "the dark theme mixes the box: {fill:?}");
        assert!(near(ink, app.pal.accent), "{ink:?}");
        app.dark = false;
        app.pal = crate::style::palette(false);
        let swatch = Some(beeper_core::models::Bubble::Swatch { id: "blue".into() });
        let _ = app.update(UiMessage::ChatTheme(chat_theme::Msg::Bubble(None, swatch)));
        let (fill, ink) = look(&app);
        assert!(near(fill, mix(app.pal.card, crate::style::hex(0x3478f6), 0.3)), "a bubble color mixes it: {fill:?}");
        assert!(near(ink, app.pal.strong), "{ink:?}");
    }

    /// A received message of a DM, as plain text.
    fn plain_received(app: &BeeperGuiApp, text: &str) -> String {
        let mid = {
            let st = app.store.read();
            let dm = st.chats.iter().find(|c| c.chat_type == "single" && !st.timeline_indices(&c.id).is_empty());
            let last = *st.timeline_indices(&dm.expect("a DM").id).last().expect("a message");
            st.messages[last].id.clone()
        };
        change(app, &mid, |m| {
            (m.text, m.html, m.linked_message_id, m.message_type) = (Some(text.into()), None, None, None);
            (m.attachments, m.links, m.poll, m.call, m.timer) = (vec![], vec![], None, None, None);
            (m.is_sender, m.is_deleted, m.reactions, m.edit_history) = (Some(false), None, vec![], vec![]);
            (m.is_edited, m.send_error, m.unresolved_media, m.view_limit) = (None, None, None, None);
        });
        mid
    }

    /// The box of the dark pixels on light ones, the text of a tooltip, that pointing at `at` adds to the message `mid`
    /// alone, in logical pixels.
    fn hover_change(app: &BeeperGuiApp, mid: &str, at: Point) -> Option<iced::Rectangle> {
        let still = pixels(&mut alone(app, mid));
        let mut sim = alone(app, mid);
        sim.point_at(at);
        let _ = sim.simulate([iced::Event::Mouse(mouse::Event::CursorMoved { position: at })]);
        let shot = pixels(&mut sim);
        let changed: Vec<(u32, u32)> = (shot.enumerate_pixels())
            .filter(|&(x, y, px)| {
                still.get_pixel(x, y).0[..3].iter().all(|&c| c > 192) && px.0[..3].iter().all(|&c| c < 128)
            })
            .map(|(x, y, _)| (x, y))
            .collect();
        let (xs, ys) = (changed.iter().map(|c| c.0), changed.iter().map(|c| c.1));
        let (x, y) = (xs.clone().min()? as f32 / 2.0, ys.clone().min()? as f32 / 2.0);
        let (right, bottom) = ((xs.max()? + 1) as f32 / 2.0, (ys.max()? + 1) as f32 / 2.0);
        Some(iced::Rectangle::new(Point::new(x, y), Size::new(right - x, bottom - y)))
    }

    /// Desktop's `ExtLink` tooltip with the host of a link: each link that linkify finds in plain text, and an HTML
    /// anchor whose text is not a URL of its host. It is centered on the link, above it, or below it at the top.
    #[test]
    fn link_host_tooltips_like_desktop() {
        let link = |text: &str, href: &str| span(text.to_string()).link(href.to_string());
        let tipped = |spans: &[iced::widget::text::Span<'_, String>], html| {
            let mut links: Vec<String> = tipped_links(spans, html).into_iter().collect();
            links.sort();
            links
        };
        assert_eq!(url_host("https://Example.com:8443/a").as_deref(), Some("example.com:8443"));
        assert_eq!(url_host("https://example.com:443/a").as_deref(), Some("example.com"));
        assert_eq!(url_host("mailto:a@b.test"), None);
        let spans = [
            link("https://a.test/x", "https://a.test/x"),
            span(" ".to_string()),
            link("the docs", "https://b.test/"),
            link("https://a.test", "https://c.test/"),
            link("a room", "https://matrix.to/#/#r:x"),
            link("mail", "mailto:a@b.test"),
        ];
        assert_eq!(tipped(&spans, true), ["https://b.test/", "https://c.test/"]);
        assert_eq!(tipped(&spans[..1], false), ["https://a.test/x"], "a linkify link always shows its host");
        let anchor = [link("https://a.test", "https://a.test/"), link("more", "https://a.test/")];
        assert!(tipped(&anchor, true).is_empty(), "the first text of an anchor decides");

        let mut app = crate::fuzz::demo_app("link-tips");
        app.pane.size = Size::new(900.0, 700.0);
        let mid = plain_received(&app, "https://example.com/docs");
        // The link is the first line, 8 px into the bubble, so its tooltip goes below it, 6 px apart.
        let tip = hover_change(&app, &mid, Point::new(30.0, 17.0)).expect("a tooltip");
        let width = text_width("example.com", 13.0, Weight::Medium);
        assert!(tip.y >= 24.0 + 6.0 + 8.0 && (tip.width - width).abs() < 3.0, "{tip:?} is {width} wide below");
        let link = text_width("https://example.com/docs", 13.0, Weight::Normal);
        assert!((tip.center_x() - (11.0 + link / 2.0)).abs() < 3.0, "{tip:?} is centered on the link");
        change(&app, &mid, |m| m.text = Some("one\ntwo\nthree\nhttps://example.com/docs".into()));
        let tip = hover_change(&app, &mid, Point::new(30.0, 64.0)).expect("a tooltip");
        assert!(tip.y + tip.height <= 56.0 - 6.0 - 8.0 && tip.y > 8.0, "{tip:?} is above the fourth line");
        let anchor = |text: &str| {
            change(&app, &mid, |m| {
                m.html = Some(format!("<a href=\"https://example.com/docs\">{text}</a>"));
                m.text = Some(text.into());
            });
            hover_change(&app, &mid, Point::new(20.0, 17.0))
        };
        assert!(anchor("https://example.com/docs").is_none(), "an anchor with its own URL has no tooltip");
        assert!(anchor("the docs").is_some());
    }

    /// Desktop's `MessageMention`: "@" and the text of the link without its own "@", bold, with no arrow.
    #[test]
    fn mentions_like_desktop() {
        let run = |text: &str, link: &str| beeper_core::codec::Run {
            text: text.into(),
            link: Some(link.into()),
            ..Default::default()
        };
        let (ann, bob) = ("https://matrix.to/#/@ann:x", "https://matrix.to/#/%40bob:x");
        let mut runs = [run("Ann", ann), run(" Lee", ann), run("@Bob", bob), run("site", "https://a.test/")];
        at_mentions(&mut runs);
        assert_eq!(runs.map(|r| r.text), ["@Ann", " Lee", "@Bob", "site"]);

        let app = crate::fuzz::demo_app("at-mentions");
        let mid = plain_received(&app, "Ann Lee");
        // The width of the bubble on the line of the text.
        let width = |html: &str| {
            change(&app, &mid, |m| m.html = Some(html.into()));
            let shot = pixels(&mut alone(&app, &mid));
            let inked: Vec<u32> = (0..shot.width()).filter(|&x| shot.get_pixel(x, 34).0[..3] != [255; 3]).collect();
            (inked.last().unwrap() - inked.first().unwrap()) as f32 / 2.0
        };
        let mention = width(&format!("<a href=\"{ann}\">Ann Lee</a>"));
        assert_eq!(mention, width(&format!("<a href=\"{ann}\">@Ann Lee</a>")), "the own \"@\" of the text goes");
        let bold = width("<strong>@Ann Lee</strong>");
        assert!((mention - bold).abs() < 3.0, "a mention of {mention} px has no arrow after its {bold} px of text");
    }

    /// Desktop's `MessageEditHistory`: the earlier versions as message rows above the message, at their times, with
    /// see-through bubbles. The "Edited" button tells how long after the message its last edit came.
    #[test]
    fn edit_history_like_desktop() {
        let after = [400, 45_000, 300_000, 7_200_000, 259_200_000, 5_184_000_000, 34_560_000_000].map(short_distance);
        assert_eq!(after, ["0s", "45s", "5m", "2h", "3d", "2mo", "1y"]);
        let mut app = crate::fuzz::demo_app("edit-history");
        app.pane.size = Size::new(900.0, 700.0);
        let mid = plain_received(&app, "third");
        let at = |minute: u32| format!("2030-01-01T10:{minute:02}:00Z");
        change(&app, &mid, |m| {
            m.timestamp = at(0);
            let edit = |text: &str, minute| EditHistoryEntry { text: text.into(), timestamp: at(minute) };
            m.edit_history = vec![edit("first", 1), edit("second", 3)];
        });
        let h24 = app.store.read().settings.pref("timeFormat") == 1;
        let time = |minute| format_short_ts(&at(minute), h24);
        let label = alone(&app, &mid).find("Edited").expect("the Edited button").bounds();
        let tip = hover_change(&app, &mid, label.center()).expect("Edited after 3m");
        assert!(tip.y + tip.height <= label.y, "{tip:?} is above the button");
        assert!(alone(&app, &mid).find(time(1)).is_err());
        let _ = app.update(UiMessage::Message(Msg::ToggleEditHistory(mid.clone())));
        let mut sim = alone(&app, &mid);
        assert!(sim.find("Hide Edits").is_ok());
        assert!(sim.find(time(1)).is_ok(), "the second version shows the time of the first edit");
        // Two 32 px rows of earlier versions 2 px apart, then the message in its received bubble.
        let shot = pixels(&mut sim);
        let close = |(x, y): (u32, u32), c: Color| {
            let (px, c) = (shot.get_pixel(2 * x, 2 * y).0, c.into_rgba8());
            (0..3).all(|i| px[i].abs_diff(c[i]) <= 2)
        };
        let see_through = mix(Color::WHITE, app.pal.link_card, 0.5);
        assert!(close((6, 16), see_through) && close((6, 50), see_through), "Desktop's half gray-40 bubbles");
        assert!(close((6, 84), app.pal.received));
    }

    /// Desktop's `object-fit: cover` of a very tall GIF (`kf`): the 160 by 400 box is full, and the GIF plays. A worker
    /// cuts its frames to the box with the corners of the bubble, and until then the whole GIF shows.
    #[test]
    fn cropped_gifs_play_like_desktop() {
        let mut app = crate::fuzz::demo_app("cropped-gif");
        app.pane.size = Size::new(900.0, 700.0);
        let mid = plain_received(&app, "");
        let url = "mxc://x/tall-gif".to_string();
        change(&app, &mid, |m| {
            m.text = None;
            m.attachments = vec![Attachment {
                attachment_type: "img".into(),
                mime_type: Some("image/gif".into()),
                src_url: Some(url.clone()),
                size: Some(AttachmentSize { width: Some(100), height: Some(1000) }),
                ..Default::default()
            }];
        });
        assert_eq!(media_box(900.0, Some((100.0, 1000.0)), 0.45), (160.0, 400.0, true), "a very tall GIF crops");
        let fill = |c: [u8; 4]| image::Handle::from_rgba(100, 1000, c.repeat(100 * 1000));
        let (red, blue) = (0xff0000, 0x0000ff);
        let frames = crate::Frames::from([(fill([255, 0, 0, 255]), 1000), (fill([0, 0, 255, 255]), 1000)]);
        app.media.insert(url.clone(), Some(Ok(frames.clone())));
        // The width of the pixels of a color, at 2x.
        let wide = |shot: &::image::RgbaImage, c: u32| {
            let xs: Vec<u32> = spots(shot, c).into_iter().map(|p| p.0).collect();
            xs.iter().max().zip(xs.iter().min()).map_or(0, |(a, b)| a - b + 1)
        };
        let at = |app: &mut BeeperGuiApp, ms: u64| {
            app.started = std::time::Instant::now() - std::time::Duration::from_millis(ms);
            pixels(&mut alone(app, &mid))
        };
        let whole = at(&mut app, 0);
        assert!((70..=90).contains(&wide(&whole, red)), "the whole GIF while its cut waits: {}", wide(&whole, red));
        let wanted: Vec<_> = std::mem::take(&mut *app.wants.cuts.borrow_mut()).into_iter().collect();
        assert_eq!(wanted, [(url.clone(), crate::gallery::Cut::Cover(160, 400))], "the view asks for the cut");
        let k = format!("Cover(160, 400) {url}");
        app.cuts.insert(k, Some(crate::gallery::cut_frames(&frames, (160, 400))));
        let first = at(&mut app, 0);
        assert!((300..=322).contains(&wide(&first, red)), "the cut fills the box: {}", wide(&first, red));
        let second = at(&mut app, 1100);
        assert!((300..=322).contains(&wide(&second, blue)) && spots(&second, red).is_empty(), "and plays");
        assert!(matches!(alone_redraw(&app, &mid), window::RedrawRequest::At(_)), "the next frame is due");
        let iced::advanced::image::Handle::Rgba { pixels, .. } =
            &app.cuts.get(&format!("Cover(160, 400) {url}")).unwrap().as_ref().unwrap()[0].0
        else {
            panic!("RGBA")
        };
        assert_eq!(pixels[3], 0, "a round corner");
        app.store.write().settings.set_pref("autoplayGifs", 0);
        let paused = at(&mut app, 1100);
        assert!(spots(&paused, 0xefeff0).len() > 1000 && spots(&paused, blue).is_empty(), "the first frame, to play");
    }

    /// Desktop's link preview image (`LinkPreview.Image`), an `EnhancedImg playable`: a GIF plays with "Autoplay
    /// GIFs", and without it shows its first frame with the play button, which plays it and opens no link.
    #[test]
    fn link_preview_gifs_follow_autoplay_like_desktop() {
        let mut app = crate::fuzz::demo_app("link-gif");
        app.pane.size = Size::new(900.0, 700.0);
        let mid = plain_received(&app, "look");
        let url = "mxc://x/link-gif".to_string();
        change(&app, &mid, |m| {
            m.links = vec![MessageLink {
                url: "https://example.com/a".into(),
                title: "A GIF".into(),
                img: Some(url.clone()),
                img_size: Some(AttachmentSize { width: Some(300), height: Some(160) }),
                summary: None,
            }];
        });
        let fill = |c: [u8; 4]| image::Handle::from_rgba(300, 160, c.repeat(300 * 160));
        let frames = crate::Frames::from([(fill([255, 0, 0, 255]), 1000), (fill([0, 0, 255, 255]), 1000)]);
        app.media.insert(url.clone(), Some(Ok(frames)));
        let (red, blue, button) = (0xff0000, 0x0000ff, 0xefeff0);
        let at = |app: &mut BeeperGuiApp, ms: u64| {
            app.started = std::time::Instant::now() - std::time::Duration::from_millis(ms);
            pixels(&mut alone(app, &mid))
        };
        let first = at(&mut app, 0);
        let image = spots(&first, red);
        assert!(image.len() > 50_000 && count(&first, &image, button) == 0, "the GIF plays by itself");
        assert_eq!(count(&at(&mut app, 1100), &image, blue), image.len(), "its second frame");
        app.store.write().settings.set_pref("autoplayGifs", 0);
        let still = at(&mut app, 1100);
        assert!(count(&still, &image, red) * 10 > image.len() * 9, "the first frame");
        assert!(count(&still, &image, button) > 1000, "the play button");
        let (x, y) = (image.iter().map(|p| p.0).sum::<u32>(), image.iter().map(|p| p.1).sum::<u32>());
        let middle = Point::new(x as f32 / image.len() as f32 / 2.0, y as f32 / image.len() as f32 / 2.0);
        let mut sim = alone(&app, &mid);
        sim.point_at(middle);
        let _ = sim.simulate(iced_test::simulator::click());
        let clicks: Vec<UiMessage> = sim.into_messages().collect();
        let toggle =
            |m: &UiMessage| matches!(m, UiMessage::Message(Msg::ToggleLinkGif(m, u)) if *m == mid && *u == url);
        assert!(matches!(&clicks[..], [m] if toggle(m)), "the button plays it, without opening the link");
        let _ = app.update(clicks.into_iter().next().unwrap());
        app.pane
            .played_links
            .insert((mid.clone(), url.clone()), std::time::Instant::now() - std::time::Duration::from_millis(1100));
        let played = at(&mut app, 0);
        assert!(count(&played, &image, blue) * 10 > image.len() * 8, "it plays from the click");
        assert_eq!(count(&played, &image, button), 0, "the pause button shows only under the pointer");
    }

    /// Desktop's `EnhancedImg` with "Autoplay GIFs" off: a GIF shows its first frame with a play button, and once it
    /// plays, a pause button while the pointer is on it. A selected message's media is at 80% brightness.
    #[test]
    fn gifs_and_selected_media_like_desktop() {
        let mut app = crate::fuzz::demo_app("gif-button");
        app.pane.size = Size::new(900.0, 700.0);
        let mid = plain_received(&app, "");
        let url = "mxc://x/gif".to_string();
        change(&app, &mid, |m| {
            m.text = None;
            m.attachments = vec![Attachment {
                attachment_type: "img".into(),
                src_url: Some(url.clone()),
                size: Some(AttachmentSize { width: Some(200), height: Some(100) }),
                ..Default::default()
            }];
        });
        let fill = |c: [u8; 4]| image::Handle::from_rgba(200, 100, c.repeat(200 * 100));
        let frames = [(fill([255, 0, 0, 255]), 100), (fill([0, 0, 255, 255]), 100)];
        app.media.insert(url.clone(), Some(Ok(crate::Frames::from(frames))));
        // The 36 px circle of the button in the middle of the 200 by 100 image, left of its icon.
        let (circle, corner) = ((2 * (100 - 13), 2 * 50), (2 * 20, 2 * 20));
        let button = |sim: &mut iced_test::Simulator<'_, UiMessage>| {
            near(pixels(sim).get_pixel(circle.0, circle.1).0, hex(0xefeff0))
        };
        assert!(!button(&mut alone(&app, &mid)), "a GIF plays by itself");
        app.store.write().settings.set_pref("autoplayGifs", 0);
        let mut sim = alone(&app, &mid);
        assert!(button(&mut sim), "the play button");
        sim.point_at(Point::new(100.0, 50.0));
        let _ = sim.simulate(iced_test::simulator::click());
        let toggles: Vec<UiMessage> = sim.into_messages().collect();
        assert!(matches!(&toggles[..], [UiMessage::Message(Msg::ToggleGif(m, 0))] if *m == mid), "not the carousel");
        let _ = app.update(UiMessage::Message(Msg::ToggleGif(mid.clone(), 0)));
        assert!(!button(&mut alone(&app, &mid)), "the pause button hides without the pointer");
        {
            let mut sim = alone(&app, &mid);
            sim.point_at(Point::new(150.0, 50.0));
            let _ = sim.simulate([iced::Event::Mouse(mouse::Event::CursorMoved { position: Point::new(150.0, 50.0) })]);
            assert!(button(&mut sim), "the pause button");
        }

        app.media.insert(url, Some(Ok(crate::Frames::from([(fill([255; 4]), 0)]))));
        let white = |app: &BeeperGuiApp| pixels(&mut alone(app, &mid)).get_pixel(corner.0, corner.1).0[0];
        assert_eq!(white(&app), 255);
        app.pane.selected_msgs = vec![mid.clone()];
        assert!(white(&app).abs_diff(204) <= 1, "brightness(0.8)");
    }

    /// The demo chat with the animations: Maya's GIF avatar, her GIF, the custom emoji in her text and Alex's
    /// custom emoji reaction to the GIF.
    const ANIMATED_CHAT: &str = "!ig_design:beeper.com";

    /// The demo app with the animated chat open, its inline avatars and media decoded like their loads.
    fn animated_demo(name: &str) -> BeeperGuiApp {
        let mut app = crate::fuzz::demo_app(name);
        let _ = app.update(UiMessage::SelectChat(ANIMATED_CHAT.into()));
        let bytes = |u: &str| iced::futures::executor::block_on(beeper_core::matrix::media(None, u.into())).unwrap();
        for _ in 0..2 {
            drop(app.view(app.main_window.expect("the main window")));
            for u in std::mem::take(&mut *app.wants.avatars.borrow_mut()) {
                let frames = crate::round_avatar(&bytes(&u));
                app.avatars.insert(u, Some(frames.expect("the demo avatar decodes")));
            }
            for u in std::mem::take(&mut *app.wants.media.borrow_mut()).into_iter().filter(|u| u.starts_with("data:")) {
                let frames = crate::media_view::decode_media(bytes(&u));
                app.media.insert(u, Some(frames));
            }
        }
        // The frames of the demo, slowed to `SLOW` each, so that a slow draw of a debug build lands in the frame
        // that it is meant to draw.
        let slow = |f: &crate::Frames| -> crate::Frames {
            f.iter().map(|(h, d)| (h.clone(), if f.len() > 1 { SLOW } else { *d })).collect()
        };
        let avatars: Vec<(String, crate::Frames)> = (app.store.read().chat(ANIMATED_CHAT).participants.items.iter())
            .filter_map(|u| u.img_url.clone())
            .filter_map(|u| Some((u.clone(), app.avatars.get(&u)?.as_ref()?.clone())))
            .collect();
        for (u, f) in avatars {
            assert!(f.len() > 1, "Maya's avatar keeps its frames");
            app.avatars.insert(u, Some(slow(&f)));
        }
        let media: Vec<(String, crate::Frames)> = (app.store.read().messages.iter())
            .filter(|m| m.chat_id == ANIMATED_CHAT)
            .flat_map(|m| {
                m.attachments
                    .iter()
                    .filter_map(|a| a.src_url.clone())
                    .chain(m.reactions.iter().filter_map(|r| r.img_url.clone()))
            })
            .filter_map(|u| Some((u.clone(), app.media.get(&u)?.as_ref()?.as_ref().ok()?.clone())))
            .collect();
        for (u, f) in media {
            assert!(f.len() > 1, "the GIF and the custom emoji keep their frames");
            app.media.insert(u, Some(Ok(slow(&f))));
        }
        app
    }

    /// The delay of each frame of the demo's animations in these tests.
    const SLOW: u128 = 2000;

    /// The main window of `app` as the simulator draws it `ms` into the animations; a draw that takes so long that
    /// it may have drawn the next frame draws again.
    fn main_at(app: &mut BeeperGuiApp, ms: u64) -> ::image::RgbaImage {
        let end = u128::from(ms) / SLOW * SLOW + SLOW;
        for _ in 0..5 {
            app.started = std::time::Instant::now() - std::time::Duration::from_millis(ms);
            let main = app.main_window.expect("the main window");
            let mut sim = iced_test::Simulator::with_size(Default::default(), Size::new(1200.0, 800.0), app.view(main));
            let shot = pixels(&mut sim);
            if app.started.elapsed().as_millis() < end {
                return shot;
            }
        }
        panic!("five draws took more than {} ms each", end - u128::from(ms));
    }

    /// The places of the pixels of `shot` in the color `c`.
    fn spots(shot: &::image::RgbaImage, c: u32) -> Vec<(u32, u32)> {
        shot.enumerate_pixels().filter(|(_, _, px)| near(px.0, hex(c))).map(|(x, y, _)| (x, y)).collect()
    }

    /// How many of the places `at` of `shot` are in the color `c`.
    fn count(shot: &::image::RgbaImage, at: &[(u32, u32)], c: u32) -> usize {
        at.iter().filter(|&&(x, y)| near(shot.get_pixel(x, y).0, hex(c))).count()
    }

    /// When the window that `app` draws asks to be drawn again after a frame, with the pointer away: the next frame
    /// of an animation, or never.
    fn next_redraw(app: &BeeperGuiApp) -> window::RedrawRequest {
        use iced::advanced::renderer::Headless as _;
        // The frame starts before its view and its layout.
        let now = std::time::Instant::now();
        let mut renderer = iced::futures::executor::block_on(iced::Renderer::new(
            iced::Font::DEFAULT,
            Pixels(13.0),
            Some("tiny-skia"),
        ))
        .expect("a headless renderer");
        let view = app.view(app.main_window.expect("the main window"));
        let size = Size::new(1200.0, 800.0);
        let mut ui = iced_runtime::UserInterface::build(view, size, Default::default(), &mut renderer);
        let redraw = [iced::Event::Window(window::Event::RedrawRequested(now))];
        let (state, _) = ui.update(
            &redraw,
            mouse::Cursor::Unavailable,
            &mut renderer,
            &mut iced::advanced::clipboard::Null,
            &mut Vec::new(),
        );
        match state {
            iced_runtime::user_interface::State::Updated { redraw_request, .. } => redraw_request,
            iced_runtime::user_interface::State::Outdated => panic!("the interface is up to date"),
        }
    }

    /// Desktop's "Animate avatars and emoji" (`animateAvatarsAndEmoji`): the still image (`rs`) of an avatar, a custom
    /// emoji in a message and a custom emoji reaction plays its frames, which advance, and shows only its first
    /// frame while the setting is off. It does not touch the GIFs of attachments, which "Autoplay GIFs" plays.
    #[test]
    fn avatars_and_emoji_animate_like_desktop() {
        let mut app = animated_demo("animate-avatars");
        // The purple and teal frames of Maya's avatar, and the orange and green ones of the custom emoji: the places
        // of the first frames take the third ones.
        let (purple, teal, orange, green) = (0x7c5cff, 0x22b8cf, 0xff9f1c, 0x8ce99a);
        let first = main_at(&mut app, 500);
        let (avatar, emoji) = (spots(&first, purple), spots(&first, orange));
        assert!(avatar.len() > 400 && emoji.len() > 100, "the avatars and the emoji show");
        let third = main_at(&mut app, 4500);
        // Most of them: the purple of a still logo stays.
        let most = |n: usize, of: &[(u32, u32)]| n * 10 > of.len() * 8;
        assert!(most(count(&third, &avatar, teal), &avatar), "the avatars play");
        assert!(most(count(&third, &emoji, green), &emoji), "the emoji play");
        assert!(matches!(next_redraw(&app), window::RedrawRequest::At(_)), "the next frame is due");
        app.store.write().settings.set_pref("animateAvatars", 0);
        let still = main_at(&mut app, 4500);
        assert_eq!(count(&still, &avatar, purple), avatar.len(), "the first frame of the avatars");
        assert_eq!(count(&still, &emoji, orange), emoji.len(), "the first frame of the emoji");
        assert!(matches!(next_redraw(&app), window::RedrawRequest::At(_)), "the GIF attachment still plays");
        app.store.write().settings.set_pref("autoplayGifs", 0);
        assert_eq!(next_redraw(&app), window::RedrawRequest::Wait, "nothing plays, so nothing ticks");
    }

    /// Like Chromium's frames of a minimized or hidden window, the animations wait while the main window is
    /// minimized or in the tray, and a chat without animations asks for no frames.
    #[test]
    fn animations_wait_while_nothing_animated_shows() {
        let mut app = animated_demo("animate-waits");
        let avatar = spots(&main_at(&mut app, 500), 0x7c5cff);
        assert!(matches!(next_redraw(&app), window::RedrawRequest::At(_)));
        app.minimized = true;
        assert_eq!(next_redraw(&app), window::RedrawRequest::Wait, "minimized");
        assert_eq!(count(&main_at(&mut app, 2500), &avatar, 0x7c5cff), avatar.len(), "the first frame shows");
        app.minimized = false;
        let _ = app.update(UiMessage::Window(crate::windows::Msg::Minimized(true)));
        assert!(app.minimized, "the window manager minimized the window");
        let _ = app.update(UiMessage::Window(crate::windows::Msg::Focused(true)));
        assert!(!app.minimized, "a focused window is not minimized");
        app.hidden = true;
        assert_eq!(next_redraw(&app), window::RedrawRequest::Wait, "in the tray");
        app.hidden = false;
        // Maya's chat, in the archive, takes her avatar off the chat list.
        let idx = app.store.read().chat_idx(ANIMATED_CHAT);
        app.store.write().update_chat(idx, |c| c.is_archived = Some(true));
        let _ = app.update(UiMessage::SelectChat("!wa_eric:beeper.com".into()));
        assert_eq!(next_redraw(&app), window::RedrawRequest::Wait, "no animation shows");
    }

    /// The frames of an animated GIF, PNG and WebP decode once into the cache, and an avatar keeps at most
    /// `AVATAR_FRAMES` of them, each delay summed into the one it keeps, so that it plays as long.
    #[test]
    fn animated_images_decode_their_frames() {
        use ::image::codecs::gif::{GifEncoder, Repeat};
        use ::image::{Delay, Frame, Rgba, RgbaImage};
        let gif = |n: usize, ms: u32| {
            let mut out = Vec::new();
            let mut enc = GifEncoder::new(&mut out);
            enc.set_repeat(Repeat::Infinite).unwrap();
            let frames = (0..n).map(|i| {
                let px = RgbaImage::from_pixel(8, 8, Rgba([(i % 25) as u8 * 10, 0, 0, 255]));
                Frame::from_parts(px, 0, 0, Delay::from_numer_denom_ms(ms, 1))
            });
            enc.encode_frames(frames).unwrap();
            drop(enc);
            out
        };
        let two = crate::media_view::decode_media(gif(2, 70)).unwrap();
        assert_eq!(two.iter().map(|f| f.1).collect::<Vec<_>>(), [70, 70]);
        assert_eq!(crate::media_view::decode_media(gif(2, 0)).unwrap()[0].1, 100, "browsers play 0 ms at 100 ms");
        let long = crate::round_avatar(&gif(250, 20)).unwrap();
        assert_eq!(long.len(), 84, "every third frame of 250");
        assert_eq!(long.iter().map(|f| f.1).sum::<u128>(), 250 * 20, "as long as the GIF");
        let png = RgbaImage::from_pixel(4, 4, Rgba([0, 0, 255, 255]));
        let mut still = Vec::new();
        png.write_to(&mut std::io::Cursor::new(&mut still), ::image::ImageFormat::Png).unwrap();
        assert_eq!(crate::media_view::animated(&still), None, "a PNG without acTL");
        assert_eq!(crate::round_avatar(&still).unwrap().len(), 1);
        let mut webp = b"RIFF\0\0\0\0WEBPVP8X\x0a\0\0\0\x02".to_vec();
        webp.resize(30, 0);
        assert_eq!(crate::media_view::animated(&webp), Some(crate::media_view::Animated::WebP));
        webp[20] = 0;
        assert_eq!(crate::media_view::animated(&webp), None, "a WebP without the animation flag");
    }

    /// When the view of `mid` alone asks to be drawn again after a frame.
    fn alone_redraw(app: &BeeperGuiApp, mid: &str) -> window::RedrawRequest {
        use iced::advanced::renderer::Headless as _;
        let now = std::time::Instant::now();
        let mut renderer = iced::futures::executor::block_on(iced::Renderer::new(
            iced::Font::DEFAULT,
            Pixels(13.0),
            Some("tiny-skia"),
        ))
        .expect("a headless renderer");
        let st = app.store.read();
        let m = st.message(mid);
        let flags = (true, true, Chain::default(), false, false);
        let view = app.view_message(&st, st.chat(&m.chat_id), &[m], flags, (None, 0), Column::new());
        let size = Size::new(900.0, 400.0);
        let mut ui = iced_runtime::UserInterface::build(view, size, Default::default(), &mut renderer);
        let redraw = [iced::Event::Window(window::Event::RedrawRequested(now))];
        let (state, _) = ui.update(
            &redraw,
            mouse::Cursor::Unavailable,
            &mut renderer,
            &mut iced::advanced::clipboard::Null,
            &mut vec![],
        );
        match state {
            iced_runtime::user_interface::State::Updated { redraw_request, .. } => redraw_request,
            iced_runtime::user_interface::State::Outdated => panic!("the interface is up to date"),
        }
    }

    /// The demo's animated Telegram sticker, decoded like its load: an orange ball that goes right and turns teal
    /// in its first half second.
    fn demo_sticker(app: &mut BeeperGuiApp) -> std::sync::Arc<crate::lottie::Animation> {
        let url = app.store.read().message(TGS_STICKER).attachments[0].src_url.clone().expect("a sticker URL");
        let bytes = iced::futures::executor::block_on(beeper_core::matrix::media(None, url.clone())).unwrap();
        let frames = crate::media_view::decode_media(bytes).expect("the sticker decodes");
        let a = frames.animation().expect("a Lottie sticker").clone();
        app.media.insert(url, Some(Ok(frames)));
        a
    }

    const TGS_STICKER: &str = "msg_x6s";

    /// Waits until the workers drew the frames that `redraw` asks for `ms` into the animations, and gives true when
    /// they are frame `due` or one of the next three, which a slow draw of a debug build may reach.
    fn drawn_at(
        app: &mut BeeperGuiApp,
        a: &crate::lottie::Animation,
        (ms, due): (u64, Option<u64>),
        redraw: fn(&BeeperGuiApp),
    ) -> bool {
        app.started = std::time::Instant::now() - std::time::Duration::from_millis(ms);
        redraw(app);
        let t = std::time::Instant::now();
        while !a.idle() {
            assert!(t.elapsed().as_secs() < 20, "a worker draws the frame");
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        due.is_none_or(|due| a.shown().iter().all(|n| (due..=due + 3).contains(n)))
    }

    /// The sticker message alone `ms` into its animation, once its worker drew frame `due`.
    fn sticker_at(app: &mut BeeperGuiApp, a: &crate::lottie::Animation, ms: u64, due: u64) -> ::image::RgbaImage {
        let redraw = |app: &BeeperGuiApp| _ = alone_redraw(app, TGS_STICKER);
        assert!((0..20).any(|_| drawn_at(app, a, (ms, Some(due)), redraw)), "the frame that is due draws");
        pixels(&mut alone(app, TGS_STICKER))
    }

    /// Desktop's `Lottie` of an animated Telegram sticker (`application/x-tgsticker`, a gzipped Lottie) in a
    /// message, an `EnhancedImg` with `playable`: with "Autoplay GIFs" it plays from its first frame, drawn by a
    /// worker at the size that shows, and asks for the next one. Without the setting it shows its first frame with
    /// the play button, and asks for no frames until the button plays it.
    #[test]
    fn tgs_stickers_play_like_desktop() {
        let (orange, teal) = (0xff9f1c, 0x22b8cf);
        let mean_x = |at: &[(u32, u32)]| at.iter().map(|p| p.0 as usize).sum::<usize>() / at.len().max(1);
        let mut app = crate::fuzz::demo_app("tgs-sticker");
        app.pane.size = Size::new(900.0, 700.0);
        let a = demo_sticker(&mut app);
        // rlottie draws the out point too, which the ball's layer covers, so the loop does not blink.
        let image::Handle::Rgba { pixels, .. } = a.render(60, (64, 64)) else { panic!("an RGBA frame") };
        assert!(pixels.chunks(4).filter(|p| p[3] > 0).count() > 500, "the last frame draws the ball");
        let first = sticker_at(&mut app, &a, 0, 0);
        let ball = spots(&first, orange);
        assert!(ball.len() > 2000 && spots(&first, teal).is_empty(), "the orange ball of the first frame");
        let half = sticker_at(&mut app, &a, 500, 30);
        let moved = spots(&half, teal);
        assert!(moved.len() > 2000 && count(&half, &ball, orange) * 10 < ball.len(), "the ball moved and turned");
        assert!(mean_x(&moved) > mean_x(&ball) + 200, "to the right");
        assert!(matches!(alone_redraw(&app, TGS_STICKER), window::RedrawRequest::At(_)), "the next frame is due");

        let mut app = crate::fuzz::demo_app("tgs-sticker-still");
        app.pane.size = Size::new(900.0, 700.0);
        app.store.write().settings.set_pref("autoplayGifs", 0);
        let a = demo_sticker(&mut app);
        let still = sticker_at(&mut app, &a, 500, 0);
        assert!(count(&still, &ball, orange) * 20 > ball.len() * 19, "the first frame, drawn at its size");
        assert!(spots(&still, 0xefeff0).len() > 1000, "the play button");
        assert_eq!(alone_redraw(&app, TGS_STICKER), window::RedrawRequest::Wait, "a still sticker asks for nothing");
        let _ = app.update(UiMessage::Message(Msg::ToggleGif(TGS_STICKER.into(), 0)));
        assert!(matches!(alone_redraw(&app, TGS_STICKER), window::RedrawRequest::At(_)), "the button plays it");
    }

    /// Desktop's `StickerImage`, an `EnhancedImg` with `isEmoji`: an animated sticker of the sticker picker plays
    /// with "Animate avatars and emoji", whatever "Autoplay GIFs" says.
    #[test]
    fn picked_tgs_stickers_animate_like_emoji() {
        let mut app = crate::fuzz::demo_app("tgs-picker");
        let _ = app.update(UiMessage::SelectChat("!tg_pebble:beeper.com".into()));
        let a = demo_sticker(&mut app);
        let url = app.store.read().message(TGS_STICKER).attachments[0].src_url.clone().unwrap();
        let sticker = beeper_core::models::Sticker {
            url,
            shortcodes: vec!["bounce".into()],
            body: None,
            info: serde_json::json!({ "mimetype": "application/x-tgsticker", "w": 512, "h": 512 }),
            source: None,
        };
        {
            let mut st = app.store.write();
            st.settings.recent_stickers = vec![sticker];
            st.settings.set_pref("autoplayGifs", 0);
        }
        // Maya's chat, in the archive, takes her animated avatar off the chat list.
        let idx = app.store.read().chat_idx(ANIMATED_CHAT);
        app.store.write().update_chat(idx, |c| c.is_archived = Some(true));
        // The view asks for the frames that show, and draws them once the workers drew them.
        let settle = |app: &mut BeeperGuiApp, due: Option<u64>| {
            let redraw = |app: &BeeperGuiApp| _ = next_redraw(app);
            assert!((0..20).any(|_| drawn_at(app, &a, (0, due), redraw)), "the frame that is due draws");
            let main = app.main_window.expect("the main window");
            pixels(&mut iced_test::Simulator::with_size(Default::default(), Size::new(1200.0, 800.0), app.view(main)))
        };
        let before = spots(&settle(&mut app, Some(0)), 0xff9f1c);
        assert_eq!(next_redraw(&app), window::RedrawRequest::Wait, "nothing plays without the picker");
        app.pane.picker = Some(crate::Picker::Sticker);
        // The orange ball of the first frame.
        let ball = spots(&settle(&mut app, Some(0)), 0xff9f1c).into_iter().filter(|p| !before.contains(p)).count();
        assert!(ball > 1000, "the sticker shows in the picker: {ball}");
        assert!(matches!(next_redraw(&app), window::RedrawRequest::At(_)), "the picker plays it");
        app.store.write().settings.set_pref("animateAvatars", 0);
        // Like Desktop's paused Lottie, it keeps the frame that it shows.
        settle(&mut app, None);
        assert_eq!(next_redraw(&app), window::RedrawRequest::Wait, "with both settings off, nothing plays");
    }

    /// Desktop's emoji message: the bubble's padding without its bottom, its time in a tooltip, and at 80% brightness
    /// while selected.
    #[test]
    fn jumbo_emoji_like_desktop() {
        let mut app = crate::fuzz::demo_app("jumbo-emoji");
        app.pane.size = Size::new(900.0, 700.0);
        let mid = plain_received(&app, "😀");
        let glyph = alone(&app, &mid).find("😀").expect("the emoji").bounds();
        assert_eq!((glyph.x, glyph.y), (11.0, 8.0));
        assert!(hover_change(&app, &mid, glyph.center()).is_some(), "the time tooltip");
        // The red of the yellow face, the brightest part of the emoji.
        let red = |app: &BeeperGuiApp| {
            let shot = pixels(&mut alone(app, &mid));
            shot.pixels().filter(|px| px[2] < 150 && px[3] > 0).map(|px| px[0]).max().expect("the face")
        };
        assert!(red(&app) > 250);
        app.pane.selected_msgs = vec![mid.clone()];
        assert!(red(&app).abs_diff(204) <= 2, "brightness(0.8)");
    }

    /// Desktop's `filter: brightness(0.8)` of a selected emoji message dims an animated custom emoji, which goes on
    /// playing: a worker dims its frames, and its first frame shows dimmed until then.
    #[test]
    fn selected_big_custom_emoji_play_like_desktop() {
        let mut app = crate::fuzz::demo_app("jumbo-animated");
        app.pane.size = Size::new(900.0, 700.0);
        let mid = plain_received(&app, ":party:");
        let url = "mxc://x/party".to_string();
        change(&app, &mid, |m| {
            m.html = Some(format!(r#"<img data-mx-emoticon src="{url}" alt=":party:" title=":party:" height="32">"#));
        });
        let fill = |c: [u8; 4]| image::Handle::from_rgba(48, 48, c.repeat(48 * 48));
        let frames = crate::Frames::from([(fill([255, 0, 0, 255]), 1000), (fill([0, 0, 255, 255]), 1000)]);
        app.media.insert(url.clone(), Some(Ok(frames.clone())));
        // The brightest red and blue of the message at `ms` into the animation.
        let at = |app: &mut BeeperGuiApp, ms: u64| {
            app.started = std::time::Instant::now() - std::time::Duration::from_millis(ms);
            let shot = pixels(&mut alone(app, &mid));
            let most = |c: usize| shot.pixels().filter(|p| p[(c + 2) % 3] < 40 && p[3] > 0).map(|p| p[c]).max();
            (most(0).unwrap_or(0), most(2).unwrap_or(0))
        };
        assert_eq!(at(&mut app, 1100), (0, 255), "the emoji plays");
        app.pane.selected_msgs = vec![mid.clone()];
        let (red, _) = at(&mut app, 1100);
        assert!(red.abs_diff(204) <= 2, "the dimmed first frame while the worker dims the frames");
        let wanted: Vec<_> = std::mem::take(&mut *app.wants.cuts.borrow_mut()).into_iter().collect();
        assert_eq!(wanted, [(url.clone(), crate::gallery::Cut::Dim)], "the view asks for the dimmed frames");
        app.cuts.insert(format!("Dim {url}"), Some(crate::gallery::dim_frames(&frames, 0.8)));
        assert!(at(&mut app, 0).0.abs_diff(204) <= 2, "the first frame, dimmed");
        let (red, blue) = at(&mut app, 1100);
        assert!(red == 0 && blue.abs_diff(204) <= 2, "the second frame, dimmed: {red} {blue}");
        assert!(matches!(alone_redraw(&app, &mid), window::RedrawRequest::At(_)), "it goes on playing");
    }

    /// Desktop's reaction pill (`.message-reaction-avatars`): up to 3 faces, each 6 px over the next one, and "+N" in
    /// `label-xs`. Its reactions list ends with "and N more" past 20, in Desktop's `xgt` numbers.
    #[test]
    fn reaction_faces_like_desktop() {
        let counts = [999, 1000, 1049, 1250, 1350, 999_999, 1_125_000, 2_500_000].map(short_count);
        assert_eq!(counts, ["999", "1.0K", "1.0K", "1.3K", "1.4K", "1000.0K", "1.13M", "2.50M"]);
        let app = crate::fuzz::demo_app("reaction-faces");
        let st = app.store.read();
        let chat = st.chats.iter().find(|c| c.chat_type == "group").expect("a group");
        let react = |k: usize| Reaction {
            id: format!("r{k}"),
            participant_id: format!("@p{k}:x"),
            reaction_key: "👍".into(),
            animated: None,
            img_url: None,
            sort_key: String::new(),
        };
        let mut m = st.messages[0].clone();
        m.reactions = (0..4).map(react).collect();
        let p = &app.pal;
        let look = PillLook {
            group: true,
            own: p.card,
            other: p.received,
            hover: None,
            fg: (p.text, p.on_secondary),
            ring: (p.card, 2.0),
            gap: 2.0,
        };
        let mut sim = simulator(app.reaction_pills(&st, chat, &[&m], &look));
        let (glyph, more) = (sim.find("👍").unwrap().bounds(), sim.find("+1").expect("+1").bounds());
        let faces = more.x - (glyph.x + glyph.width) - 2.0 * 4.0;
        assert!((faces - (16.0 + 2.0 * 10.0)).abs() < 0.01, "three faces in 36 px, not {faces}");
        assert_eq!(more.height, 12.0, "`label-xs`");
        m.reactions = (0..22).map(react).collect();
        assert!(simulator(app.reaction_list(chat, &[&m])).find("and 3 more").is_ok());
    }

    /// Desktop's reaction pills: one pill a group of skin tones, in the order the groups first appear, and a click
    /// that adds the group's reaction or removes your own reactions of the group.
    #[test]
    fn reactions_group_like_desktop() {
        let app = crate::fuzz::demo_app("reaction-groups");
        let st = app.store.read();
        let me = st.self_id("beeper");
        let react = |who: &str, key: &str| Reaction {
            id: format!("{who}{key}"),
            participant_id: who.into(),
            reaction_key: key.into(),
            animated: None,
            img_url: None,
            sort_key: String::new(),
        };
        let mut m = st.messages[0].clone();
        m.reactions = vec![react("@a:x", "😂"), react("@b:x", "👍🏽"), react(&me, "👍️"), react("@c:x", "🎉")];
        let groups: Vec<(String, usize)> = reaction_groups(&[&m]).into_iter().map(|(g, rs)| (g, rs.len())).collect();
        assert_eq!(groups, [("😂".into(), 1), ("👍".into(), 2), ("🎉".into(), 1)], "first seen, tones together");
        let removes = |msg: UiMessage| match msg {
            UiMessage::Message(Msg::ToggleReaction(_, _, key)) => key,
            _ => panic!("one toggle"),
        };
        let mut chat = st.chats[0].clone();
        chat.messages.react = true;
        assert_eq!(removes(toggle_reaction(&st, &chat, &[&m], "👍")), "👍️", "a click removes your own reaction");
        assert_eq!(removes(toggle_reaction(&st, &chat, &[&m], "😂")), "😂", "or adds the group's");
        // Desktop's `Dte` and `D3e`: an emoji added from a pill is in your own skin tone, not the first reactor's.
        m.reactions.retain(|r| r.participant_id != me);
        assert_eq!(removes(toggle_reaction(&st, &chat, &[&m], "👍")), "👍", "the default tone");
        drop(st);
        app.store.write().skin_tone = 2;
        let st = app.store.read();
        assert_eq!(removes(toggle_reaction(&st, &chat, &[&m], "👍")), "👍🏼", "your own tone");
        chat.allowed_reactions = vec!["👍".into()];
        assert_eq!(removes(toggle_reaction(&st, &chat, &[&m], "👍")), "👍", "none where the network takes only some");
    }

    /// Desktop's `U3t`: the quick reactions of the flyover are in your skin tone when the network takes any emoji.
    #[test]
    fn quick_reactions_skin_tone_like_desktop() {
        let app = crate::fuzz::demo_app("quick-tone");
        app.store.write().skin_tone = 4;
        let st = app.store.read();
        assert_eq!(crate::emoji::quick_reactions(&st, &[], true), ["👍🏾", "❤️", "😂"]);
        let allowed = ["👍".to_string(), "😂".to_string()];
        assert_eq!(crate::emoji::quick_reactions(&st, &allowed, true), ["👍", "😂"]);
    }

    /// Desktop's `Seen by` of a group: the newest reader first, by first name when no other member has it, and all the
    /// full names in its tooltip on the left.
    #[test]
    fn seen_by_names_like_desktop() {
        let mut app = crate::fuzz::demo_app("seen-by");
        let chat = {
            let mut st = app.store.write();
            let i = st.chats.iter().position(|c| c.chat_type == "group").expect("a group");
            let user =
                |id: &str, name: &str| User { id: id.into(), full_name: Some(name.into()), ..Default::default() };
            let people = [user("@a:x", "Ann Lee"), user("@b:x", "Ann Smith"), user("@c:x", "Bo Jones")];
            st.update_chat(i, |c| c.participants.items.extend(people));
            let id = st.chats[i].id.clone();
            let last = *st.timeline_indices(&id).last().expect("a message");
            let seen =
                [("@a:x", "2030-01-01T10:00:00Z"), ("@b:x", "2030-01-01T10:01:00Z"), ("@c:x", "2030-01-01T10:02:00Z")];
            st.update_message(last, |m| {
                m.is_sender = Some(true);
                m.seen = seen.iter().map(|(u, t)| (u.to_string(), t.to_string())).collect();
            });
            id
        };
        let _ = app.update(UiMessage::SelectChat(chat));
        app.pane.size = Size::new(900.0, 700.0);
        assert!(simulator(app.view_conversation()).find("Seen by Bo, Ann Smith, Ann Lee").is_ok());
        // The timeline alone, without the composer over its bottom.
        let st = app.store.read();
        let chat = st.chat(app.pane.selected_chat_id.as_deref().unwrap());
        let rows = app.conversation_rows(&st, chat);
        let msgs: Vec<&Message> = rows.msgs.iter().map(|&i| &st.messages[i]).collect();
        let view = app.view_messages(&st, chat, &msgs, &rows, false);
        let mut sim = iced_test::Simulator::with_size(Default::default(), app.pane.size, view);
        let label = sim.find("Seen by Bo, Ann Smith, Ann Lee").expect("the label").bounds();
        sim.point_at(label.center());
        let _ = sim.simulate([iced::Event::Mouse(mouse::Event::CursorMoved { position: label.center() })]);
        // The text of the tooltip left of the label is as wide as the full names.
        let shot = pixels(&mut sim);
        let rows = (label.y * 2.0) as u32..((label.y + label.height) * 2.0) as u32;
        let ink: Vec<u32> = (0..(label.x * 2.0) as u32)
            .filter(|&x| rows.clone().any(|y| near(shot.get_pixel(x, y).0, app.pal.strong)))
            .collect();
        let width = (ink.last().unwrap() - ink[0]) as f32 / 2.0;
        let full = text_width("Bo Jones, Ann Smith, Ann Lee", 13.0, Weight::Medium);
        assert!((width - full).abs() < 3.0, "the tooltip text is {width} px wide, the full names {full} px");
    }

    /// Desktop's `BrandTypingIndicator`: who types in its tooltip, in a group the first 4 faces at
    /// `min(20, (34 + 10(n - 1)) / n)` px, and 0.32 s to show and to hide, but at once while scrolled up.
    #[test]
    fn typing_indicator_like_desktop() {
        let names = |n: usize| (1..=n).map(|k| format!("P{k}")).collect::<Vec<_>>();
        assert_eq!(typing_tip(&names(1)), "P1 is typing…");
        assert_eq!(typing_tip(&names(3)), "P1, P2 and P3 are typing…");
        assert_eq!(typing_tip(&names(7)), "P1, P2, P3, P4, P5, P6 and 1 other is typing…");
        assert_eq!(typing_tip(&names(9)), "P1, P2, P3, P4, P5, P6 and 3 others are typing…");
        let mut app = crate::fuzz::demo_app("typing-indicator");
        let people = [("@q:x", "Qi"), ("@x:x", "Xu"), ("@z:x", "Zo"), ("@j:x", "Jo"), ("@y:x", "Yu")];
        let ids: Vec<String> = people.iter().map(|p| p.0.to_string()).collect();
        let cid = {
            let mut st = app.store.write();
            let i = st.chats.iter().position(|c| c.chat_type == "group").expect("a group");
            let user = |(id, name): &(&str, &str)| User {
                id: id.to_string(),
                full_name: Some(name.to_string()),
                ..Default::default()
            };
            st.update_chat(i, |c| c.participants.items.extend(people.iter().map(user)));
            st.chats[i].id.clone()
        };
        let _ = app.update(UiMessage::SelectChat(cid.clone()));
        let shown = |app: &BeeperGuiApp| app.typing_now(&app.store.read(), &cid).map(|(ids, v)| (ids.len(), v));
        let typing = |app: &mut BeeperGuiApp, on: bool| {
            match on {
                true => app.store.write().typing.insert(cid.clone(), ids.clone()),
                false => app.store.write().typing.remove(&cid),
            };
            let _ = app.update(UiMessage::Typing(cid.clone()));
        };
        let later = |app: &mut BeeperGuiApp| app.typing_shown.get_mut(&cid).unwrap().2 -= TYPING_FADE;
        // Whether the indicator shows and started to show at `before` or after it.
        let started = |app: &BeeperGuiApp, before| app.typing_shown.get(&cid).is_some_and(|t| t.1 && t.2 >= before);
        app.pane.scrolled_up = Some((false, std::time::Instant::now()));
        let before = std::time::Instant::now();
        typing(&mut app, true);
        assert!(started(&app, before), "it starts to show at the bottom of the timeline");
        later(&mut app);
        assert_eq!(shown(&app), Some((5, 1.0)));
        {
            let st = app.store.read();
            let mut sim = simulator(app.typing_bubble(&st, st.chat(&cid), &ids, 1.0));
            for l in ["Q", "X", "Z", "J"] {
                assert_eq!(sim.find(l).expect(l).bounds().height, 8.0, "16 px faces for 4");
            }
            assert!(sim.find("Y").is_err(), "4 faces at most");
        }
        let before = std::time::Instant::now();
        typing(&mut app, false);
        let hiding = app.typing_shown.get(&cid).is_some_and(|t| t.0.len() == 5 && !t.1 && t.2 >= before);
        assert!(hiding, "it hides with who typed");
        later(&mut app);
        assert_eq!(shown(&app), None);
        app.pane.scrolled_up = Some((true, std::time::Instant::now()));
        typing(&mut app, true);
        assert_eq!(shown(&app), Some((5, 1.0)), "at once while scrolled up");
    }

    /// Whether the message menu of `mid` has `label`.
    fn menu_has(app: &BeeperGuiApp, mid: &str, label: &str) -> bool {
        simulator(app.view_message_menu(mid).0).find(label).is_ok()
    }

    /// The newest message of the open chat that `pick` takes.
    fn newest(app: &BeeperGuiApp, pick: impl Fn(&Message) -> bool) -> String {
        let st = app.store.read();
        app.timeline(&st).0.into_iter().rev().find(|m| pick(m)).expect("a message").id.clone()
    }

    /// Changes the message `mid` in the store.
    fn change(app: &BeeperGuiApp, mid: &str, f: impl FnOnce(&mut Message)) {
        let mut st = app.store.write();
        let i = st.message_idx(mid);
        st.update_message(i, f);
    }

    /// Desktop's `showReactionMenuForMessage`: React… of the menu and → on a selected message open the picker at the
    /// top left of the message, which the selector finds on screen, not where the pointer happens to be.
    #[test]
    fn react_from_the_menu_and_keys_like_desktop() {
        use iced::keyboard::{key::Named, Key, Modifiers};
        let note = "!note_self:beeper.com";
        let mut app = crate::fuzz::demo_app("react-on");
        let _ = app.update(UiMessage::SelectChat(note.into()));
        let mid = newest(&app, |m| m.reactable());
        let _ = app.update(UiMessage::Message(Msg::OpenMenu(mid.clone())));
        let mut sim = simulator(app.view_message_menu(&mid).0);
        let _ = sim.click("React…").expect("React…");
        let picked: Vec<UiMessage> = sim.into_messages().collect();
        let on = |m: &UiMessage| matches!(m, UiMessage::Message(Msg::ReactOn(m)) if *m == mid);
        assert!(matches!(&picked[..], [UiMessage::Message(Msg::Menu(m))] if on(m)), "React… opens it on the message");
        let _ = app.update(UiMessage::CloseModals);
        let _ = app.update(UiMessage::Message(Msg::Select(vec![mid.clone()])));
        let _ = app.update(UiMessage::KeyPressed(Key::Named(Named::ArrowRight), Modifiers::empty()));
        assert!(app.pane.emoji.is_none(), "→ waits for the place of the message before it opens the picker");
        let shown = simulator(app.view_main()).find(iced::widget::Id::from(format!("msg-{mid}"))).expect("the message");
        let at = shown.visible_bounds().expect("the message is on screen").position();
        assert!(at.x < 400.0, "the row of the message starts at the left of the timeline: {at:?}");
    }

    /// Desktop's `editing-message` backdrop: while you edit a message, which unselects the messages, a click on the
    /// timeline under the backdrop ends the edit (`clearEditingMessage`) and one on the edited message does not.
    #[test]
    fn edit_backdrop_like_desktop() {
        let note = "!note_self:beeper.com";
        let mut app = crate::fuzz::demo_app("edit-backdrop");
        let _ = app.update(UiMessage::SelectChat(note.into()));
        let mid = newest(&app, |m| m.is_sender == Some(true) && m.attachments.is_empty() && m.poll.is_none());
        let id = iced::widget::Id::from(format!("msg-{mid}"));
        let ends = |app: &BeeperGuiApp, at: Point| {
            let mut sim = simulator(app.view_main());
            sim.point_at(at);
            let press = mouse::Event::ButtonPressed(mouse::Button::Left);
            let _ = sim.simulate([
                iced::Event::Mouse(press),
                iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
            ]);
            sim.into_messages().any(|m| matches!(m, UiMessage::Composer(composer::Msg::CancelMode)))
        };
        let b = simulator(app.view_main()).find(id.clone()).expect("the message").visible_bounds().expect("on screen");
        let above = Point::new(b.center_x(), b.y - 20.0);
        assert!(!ends(&app, above), "no backdrop before the edit");
        let _ = app.update(UiMessage::Message(Msg::Select(vec![mid.clone()])));
        let _ = app.update(UiMessage::Composer(composer::Msg::StartEdit(mid.clone(), "old".into())));
        assert!(app.pane.selected_msgs.is_empty(), "the edit unselects the messages");
        let b = simulator(app.view_main()).find(id).expect("the message").visible_bounds().expect("on screen");
        assert!(!ends(&app, b.center()), "a click on the edited message keeps the edit");
        assert!(ends(&app, Point::new(b.center_x(), b.y - 20.0)), "one on the backdrop ends it");
        let _ = app.update(UiMessage::Composer(composer::Msg::CancelMode));
        assert!(app.pane.editing_message_id.is_none() && app.pane.stash.is_none());
    }

    /// Desktop's `D`: Edit…, Reply… and React… hide for the message that you edit, and `canStartEditMessage` offers
    /// Edit… for the caption of an own image where the chat takes one.
    #[test]
    fn edit_items_like_desktop() {
        let note = "!note_self:beeper.com";
        let mut app = crate::fuzz::demo_app("edit-items");
        let _ = app.update(UiMessage::SelectChat(note.into()));
        let mid = newest(&app, |m| m.is_sender == Some(true) && m.attachments.is_empty() && m.poll.is_none());
        let has = |app: &BeeperGuiApp, label| menu_has(app, &mid, label);
        let _ = app.update(UiMessage::Message(Msg::OpenMenu(mid.clone())));
        assert!(has(&app, "Edit…") && has(&app, "Reply…") && has(&app, "React…"));
        let _ = app.update(UiMessage::Composer(composer::Msg::StartEdit(mid.clone(), "old".into())));
        assert!(!has(&app, "Edit…") && !has(&app, "Reply…") && !has(&app, "React…"), "not while you edit it");
        app.pane.end_edit();
        let img =
            Attachment { attachment_type: "img".into(), mime_type: Some("image/jpeg".into()), ..Default::default() };
        change(&app, &mid, |m| (m.text, m.message_type, m.attachments) = (None, Some("IMAGE".into()), vec![img]));
        assert!(has(&app, "Edit…"), "the caption of an own image");
        {
            let mut st = app.store.write();
            let i = st.resolve_chat_idx(note).expect("the note chat");
            st.update_chat(i, |c| c.file.clear());
        }
        assert!(!has(&app, "Edit…"), "not where the chat takes no caption");
    }

    /// Desktop's `mhe`: Bookmark for a message that is not deleted, once the bookmarks room is read.
    #[test]
    fn bookmark_item_like_desktop() {
        let mut app = crate::fuzz::demo_app("bookmark-item");
        let _ = app.update(UiMessage::SelectChat("!note_self:beeper.com".into()));
        let mid = newest(&app, |m| m.is_deleted != Some(true));
        let _ = app.update(UiMessage::Message(Msg::OpenMenu(mid.clone())));
        assert!(menu_has(&app, &mid, "Bookmark"));
        change(&app, &mid, |m| m.is_deleted = Some(true));
        assert!(!menu_has(&app, &mid, "Bookmark"), "not on a deleted message");
        change(&app, &mid, |m| m.is_deleted = None);
        {
            let mut st = app.store.write();
            st.config.demo = false;
            let seeded: Vec<String> = st.messages.iter().map(|m| m.id.clone()).collect();
            st.sent.extend(seeded.into_iter().map(|id| (id.clone(), id)));
        }
        assert!(!menu_has(&app, &mid, "Bookmark"), "not before the bookmarks room is read");
        app.store.write().bookmarks_room = Some(beeper_core::models::BOOKMARKS_ROOM.into());
        assert!(menu_has(&app, &mid, "Bookmark"));
    }

    /// Desktop's Mark as Unread from a message of a bridged chat, also from an unread one, but not past the unread
    /// mark in incognito mode, which keeps the mark where it is.
    #[test]
    fn mark_unread_item_like_desktop() {
        let mut app = crate::fuzz::demo_app("unread-item");
        let chat = {
            let st = app.store.read();
            let bridged = st.chats.iter().filter(|c| c.account_id != "hungryserv");
            bridged.map(|c| c.id.clone()).find(|id| st.messages.iter().any(|m| m.chat_id == *id)).expect("a chat")
        };
        let _ = app.update(UiMessage::SelectChat(chat.clone()));
        let mid = newest(&app, |m| m.is_deleted != Some(true));
        let read_to = |app: &BeeperGuiApp, key: &str| {
            let mut st = app.store.write();
            let i = st.resolve_chat_idx(&chat).expect("the chat");
            st.update_chat(i, |c| c.last_read_message_sort_key = Some(key.into()));
        };
        change(&app, &mid, |m| m.is_unread = Some(true));
        read_to(&app, "0");
        let _ = app.update(UiMessage::Message(Msg::OpenMenu(mid.clone())));
        assert!(menu_has(&app, &mid, "Mark as Unread") && menu_has(&app, &mid, "Mark as Read"), "an unread message");
        let trial = serde_json::json!({ "started_at": "2020-01-01T00:00:00Z", "expires_at": "2999-01-01T00:00:00Z" });
        app.store.write().features.usage = serde_json::json!({ "incognito": trial });
        let _ = app.update(UiMessage::RunHotkey("Toggle Incognito Mode"));
        assert!(app.store.read().incognito());
        assert!(!menu_has(&app, &mid, "Mark as Unread"), "incognito mode keeps the mark");
        let key = app.store.read().message(&mid).sort_key.clone();
        read_to(&app, &key);
        assert!(menu_has(&app, &mid, "Mark as Unread"), "a message at the mark");
    }

    /// Desktop's `selectedImage`: a right click on the image gives Copy Image and Download Image in place of Copy with
    /// Timestamp, and Copy copies the image of a message of one image without text (`selectedImageAttachment`).
    #[test]
    fn image_items_like_desktop() {
        let mut app = crate::fuzz::demo_app("image-items");
        let _ = app.update(UiMessage::SelectChat("!note_self:beeper.com".into()));
        let mid = newest(&app, |m| m.is_deleted != Some(true) && m.attachments.is_empty());
        let img = Attachment { attachment_type: "img".into(), src_url: Some("mxc://x/a".into()), ..Default::default() };
        change(&app, &mid, |m| (m.text, m.attachments) = (Some("caption".into()), vec![img]));
        let has = |app: &BeeperGuiApp, label| menu_has(app, &mid, label);
        let copies_image = |app: &BeeperGuiApp| {
            let mut sim = simulator(app.view_message_menu(&mid).0);
            let _ = sim.click("Copy").expect("Copy");
            sim.into_messages().any(|m| match m {
                UiMessage::Message(Msg::Menu(m)) => matches!(*m, UiMessage::Message(Msg::CopyImage(..))),
                _ => false,
            })
        };
        let _ = app.update(UiMessage::Message(Msg::OpenMenu(mid.clone())));
        assert!(has(&app, "Copy with Timestamp") && !has(&app, "Copy Image") && has(&app, "Download Attachment"));
        let _ = app.update(UiMessage::Message(Msg::ImageMenu(mid.clone())));
        assert!(!has(&app, "Copy with Timestamp") && has(&app, "Copy Image") && has(&app, "Download Image"));
        assert!(!copies_image(&app), "Copy copies the text");
        change(&app, &mid, |m| m.text = None);
        assert!(copies_image(&app) && !has(&app, "Copy Image"), "Copy copies the one image");
        let _ = app.update(UiMessage::Message(Msg::OpenMenu(mid.clone())));
        assert!(copies_image(&app) && has(&app, "Copy with Timestamp"), "also without the image's right click");
    }

    /// Desktop's `AttachmentFileRow` states: an image that the bridge loads only when the user asks shows as an
    /// unfetched file row, whose click starts the interactive download; the fetching row is disabled; a download
    /// shows the image, and a failed one has Desktop's toast.
    #[test]
    fn file_row_states_like_desktop() {
        let mut app = crate::fuzz::demo_app("file-row-states");
        let _ = app.update(UiMessage::SelectChat("!note_self:beeper.com".into()));
        let mid = newest(&app, |m| m.is_deleted != Some(true) && m.attachments.is_empty());
        let url = "mxc://x/phone".to_string();
        let img = Attachment {
            attachment_type: "img".into(),
            src_url: Some(url.clone()),
            file_name: Some("photo.jpg".into()),
            ..Default::default()
        };
        change(&app, &mid, |m| (m.text, m.attachments) = (None, vec![img]));
        let a = app.store.read().message(&mid).attachments[0].clone();
        // The URLs that a click on the row fetches.
        let clicks = |app: &BeeperGuiApp| {
            let mut sim = simulator(app.view_conversation());
            let _ = sim.click("photo.jpg");
            let fetches = sim.into_messages().filter_map(|m| match m {
                UiMessage::Loaded(loads::Msg::Fetch(u)) => Some(u),
                _ => None,
            });
            fetches.collect::<Vec<_>>()
        };
        assert!(simulator(app.view_conversation()).find("photo.jpg").is_err(), "a loading image has no file row");
        let reload = format!("{} (HTTP 502): the phone has it", beeper_core::matrix::MEDIA_RELOAD_NEEDED);
        let _ = app.update(UiMessage::Loaded(loads::Msg::Media(url.clone(), Err(reload))));
        assert_eq!(app.file_state(&a), FileState::Unfetched);
        assert_eq!(clicks(&app), std::slice::from_ref(&url), "the row asks for the media");
        let _ = app.update(UiMessage::Loaded(loads::Msg::Fetch(url.clone())));
        assert_eq!(app.file_state(&a), FileState::Fetching);
        assert!(clicks(&app).is_empty(), "the fetching row is disabled");
        let _ = app.update(UiMessage::Loaded(loads::Msg::Fetched(url.clone(), Err("no phone".into()))));
        assert_eq!(app.file_state(&a), FileState::Unfetched, "a failed download asks again");
        let toast = app.shown_toast().map(|t| t.text.clone());
        let text = "Could not download this file. Check that the app is running on your phone and try again.";
        assert_eq!(toast.as_deref(), Some(text));
        let _ = app.update(UiMessage::Loaded(loads::Msg::Fetch(url.clone())));
        let _ = app.update(UiMessage::Loaded(loads::Msg::Fetched(url.clone(), Ok(()))));
        assert_eq!(app.file_state(&a), FileState::Ready);
        assert!(app.media.get(&url).is_none(), "the image loads again, from the cache");
        assert!(simulator(app.view_conversation()).find("photo.jpg").is_err(), "the image takes its place");
    }

    /// A still that is much larger than its box shows from a cut of its box that a worker bakes, like Chromium's
    /// decode to scale, and a small one shows whole.
    #[test]
    fn large_images_show_from_a_cut_of_their_box() {
        let mut app = crate::fuzz::demo_app("large-image");
        let _ = app.update(UiMessage::SelectChat("!note_self:beeper.com".into()));
        let mid = newest(&app, |m| m.is_deleted != Some(true) && m.attachments.is_empty());
        let cut = |app: &mut BeeperGuiApp, (w, h): (u32, u32)| {
            let url = format!("mxc://x/{w}");
            let size = Some(AttachmentSize { width: Some(w), height: Some(h) });
            let img =
                Attachment { attachment_type: "img".into(), src_url: Some(url.clone()), size, ..Default::default() };
            change(app, &mid, |m| (m.text, m.attachments) = (None, vec![img]));
            let px = crate::Frames::from([(image::Handle::from_rgba(w, h, vec![0; (w * h * 4) as usize]), 0)]);
            let _ = app.update(UiMessage::Loaded(loads::Msg::Media(url.clone(), Ok(px))));
            app.wants.tiles.borrow_mut().clear();
            drop(alone(app, &mid));
            let asked = app.wants.tiles.borrow().iter().any(|t| t.0 == url && t.3 == crate::gallery::Look::Inline);
            asked
        };
        assert!(cut(&mut app, (2400, 1800)), "a photo much larger than its box");
        assert!(!cut(&mut app, (300, 200)), "an image at its own size");
    }

    /// Desktop's `img.loading`: an image that loads shows the shimmering `--skeleton-bg` in the box of the image,
    /// which the image takes when it loads.
    #[test]
    fn loading_image_like_desktop() {
        let mut app = crate::fuzz::demo_app("loading-image");
        let _ = app.update(UiMessage::SelectChat("!note_self:beeper.com".into()));
        let mid = newest(&app, |m| m.is_deleted != Some(true) && m.attachments.is_empty());
        let url = "mxc://x/photo".to_string();
        let size = Some(AttachmentSize { width: Some(300), height: Some(200) });
        let img = Attachment { attachment_type: "img".into(), src_url: Some(url.clone()), size, ..Default::default() };
        change(&app, &mid, |m| (m.text, m.attachments) = (None, vec![img]));
        let loading = |app: &BeeperGuiApp| {
            let mut sim = alone(app, &mid);
            sim.find(|c: iced_test::selector::Candidate<'_>| match c {
                iced_test::selector::Candidate::Custom { bounds, state, .. } if state.is::<anim::Loading>() => {
                    Some(bounds)
                }
                _ => None,
            })
            .ok()
        };
        let shown = loading(&app).expect("the skeleton");
        assert_eq!((shown.width, shown.height), (300.0, 200.0), "the box of the image");
        assert!(alone(&app, &mid).find(icon::IMAGE.to_string()).is_err(), "no image icon");
        let px = crate::Frames::from([(image::Handle::from_rgba(1, 1, vec![0; 4]), 0)]);
        let _ = app.update(UiMessage::Loaded(loads::Msg::Media(url, Ok(px))));
        assert!(loading(&app).is_none(), "the image takes its place");
    }

    /// Desktop's `ImageError`: an image that failed to load shows the info icon, not the text of unresolved media,
    /// and its click opens the carousel, which loads it again.
    #[test]
    fn failed_image_like_desktop() {
        let mut app = crate::fuzz::demo_app("failed-image");
        let _ = app.update(UiMessage::SelectChat("!note_self:beeper.com".into()));
        let mid = newest(&app, |m| m.is_deleted != Some(true) && m.attachments.is_empty());
        let url = "mxc://x/broken".to_string();
        let img = Attachment { attachment_type: "img".into(), src_url: Some(url.clone()), ..Default::default() };
        change(&app, &mid, |m| (m.text, m.attachments) = (None, vec![img]));
        let _ = app.update(UiMessage::Loaded(loads::Msg::Media(url.clone(), Err("HTTP 500".into()))));
        assert!(simulator(app.view_conversation()).find("Could not load. Click to try again").is_err());
        assert!(app.needs_download.is_empty(), "another error needs no download");
        let _ = app.update(UiMessage::Media(media_view::Msg::OpenCarousel(mid, 0, false)));
        assert!(!matches!(app.media.get(&url), Some(Some(Err(_)))), "the carousel loads the image again");
    }

    /// Desktop's `canOpen` of the forward menu: Forward… also for a deleted message, but not for view-once media.
    #[test]
    fn forward_item_like_desktop() {
        let mut app = crate::fuzz::demo_app("forward-item");
        let _ = app.update(UiMessage::SelectChat("!note_self:beeper.com".into()));
        let mid = newest(&app, |m| m.is_deleted != Some(true));
        let _ = app.update(UiMessage::Message(Msg::OpenMenu(mid.clone())));
        change(&app, &mid, |m| m.is_deleted = Some(true));
        assert!(menu_has(&app, &mid, "Forward…"), "a deleted message");
        change(&app, &mid, |m| (m.is_deleted, m.view_limit) = (None, Some(serde_json::json!({ "type": "count" }))));
        assert!(!menu_has(&app, &mid, "Forward…"), "view-once media");
    }

    /// Desktop's flyover (`show` of `Yfe`) has no items on a pending message.
    #[test]
    fn flyover_items_like_desktop() {
        let note = "!note_self:beeper.com";
        let mut app = crate::fuzz::demo_app("flyover-items");
        let _ = app.update(UiMessage::SelectChat(note.into()));
        app.pane.size = Size::new(900.0, 700.0);
        let (_, sent, _) = app.store.write().send_message(note, Some("hi".into()), None, None).expect("a send");
        let reply = icon::REPLY.to_string();
        let shows = |app: &BeeperGuiApp| simulator(app.view_conversation()).find(reply.as_str()).is_ok();
        let _ = app.update(UiMessage::Message(Msg::HoverEnter(sent.clone())));
        assert!(shows(&app), "a sent message");
        app.store.write().config.demo = false;
        assert!(!shows(&app), "a pending message");
    }

    /// Desktop's double click reactions (`$`): none in a chat without reactions, and React with... maps to an
    /// allowed reaction.
    #[test]
    fn double_click_reactions_like_desktop() {
        let mut app = crate::fuzz::demo_app("double-click-react");
        let note = "!note_self:beeper.com";
        let _ = app.update(UiMessage::SelectChat(note.into()));
        let mid = newest(&app, Message::reactable);
        let rules = |app: &BeeperGuiApp, react: bool, allowed: &[&str]| {
            let mut st = app.store.write();
            let i = st.resolve_chat_idx(note).expect("the note chat");
            st.update_chat(i, |c| {
                c.messages.react = react;
                c.allowed_reactions = allowed.iter().map(|a| a.to_string()).collect();
            });
        };
        let pref = |app: &mut BeeperGuiApp, v| {
            let _ = app.update(UiMessage::Settings(crate::settings::Msg::SetPref("messageDoubleClick", v)));
        };
        let reactions = |app: &BeeperGuiApp| app.store.read().message(&mid).reactions.len();
        let before = reactions(&app);
        pref(&mut app, 1);
        rules(&app, false, &[]);
        let _ = app.update(UiMessage::Message(Msg::DoubleClick(mid.clone())));
        assert!(app.pane.emoji.is_none(), "React opens no picker without reactions");
        pref(&mut app, 5);
        let _ = app.update(UiMessage::Message(Msg::DoubleClick(mid.clone())));
        assert_eq!(reactions(&app), before, "React with... adds none without reactions");
        let quick = app.store.read().settings.quick_reaction.clone();
        assert_ne!(quick, "🎉");
        rules(&app, true, &["🎉"]);
        let _ = app.update(UiMessage::Message(Msg::DoubleClick(mid.clone())));
        assert_eq!(reactions(&app), before, "nor one that the chat does not allow");
        rules(&app, true, &[]);
        let _ = app.update(UiMessage::Message(Msg::DoubleClick(mid.clone())));
        assert!(app.store.read().message(&mid).reactions.iter().any(|r| r.reaction_key == quick));
        pref(&mut app, 1);
        let _ = app.update(UiMessage::Message(Msg::DoubleClick(mid.clone())));
        assert!(app.pane.emoji.is_some(), "React opens the picker");
    }
}
