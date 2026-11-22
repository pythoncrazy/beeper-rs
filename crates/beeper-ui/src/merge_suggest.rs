//! Desktop's merge suggestions (`unionsStore.mergeSuggestions`): the chats that look like the same person, which the
//! `MergeSuggestionBanner` under the chat header and Chat Info offer to merge, and the `TargetSwitcherNudge` over the
//! composer switcher of a merged chat.

use super::*;
use crate::composer::hidden_actions;
use crate::merge_editor::{merge_keys, mergeable, other, MERGE_HELP};
use crate::message::raised;
use iced::widget::{column, rich_text, span};
use std::collections::BTreeSet;
use std::sync::LazyLock;

/// Desktop's `BANNER_SHOW_LIMIT`: the merge suggestion banner shows in 3 chats at most.
const BANNER_SHOW_LIMIT: usize = 3;
/// Desktop's `canShowTargetSwitcherNudge`: the switcher nudge shows in 3 merged chats at most.
const NUDGE_SHOW_LIMIT: usize = 3;
/// Desktop's `bmt`: the most network icons of the banner.
const BANNER_ICONS: usize = 8;
/// Desktop's `yJ`: each network icon covers 6 px of the one before.
const ICON_OVERLAP: f32 = 6.0;
/// The height of the banner: 16 px around its title over its network icons.
pub(crate) const BANNER_H: f32 = 16.0 + 16.0 + 6.0 + 20.0 + 16.0;

/// The tails of `MergeSuggestionBanner-module__card:after` and `TargetSwitcherNudge-module__bubble:after`.
static TAIL_UP: LazyLock<svg::Handle> = LazyLock::new(|| {
    svg::Handle::from_memory(&br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 30 11"><path d="M0,11 C6,11 10.5,0 15,0 C19.5,0 24,11 30,11 Z"/></svg>"##[..])
});
static TAIL_DOWN: LazyLock<svg::Handle> = LazyLock::new(|| {
    svg::Handle::from_memory(&br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 30 11"><path d="M0,0 C6,0 10.5,11 15,11 C19.5,11 24,0 30,0 Z"/></svg>"##[..])
});
/// Desktop's `su` cutout of a network icon under the next one: a 2 px wider box of radius 10, here in the 20 px rows
/// of the icons.
static CUTOUT: LazyLock<svg::Handle> = LazyLock::new(|| {
    svg::Handle::from_memory(&br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 2 24 20"><rect width="24" height="24" rx="10"/></svg>"##[..])
});

#[derive(Debug, Clone)]
pub enum Msg {
    /// The banner's "Merge options": the merge editor with the chats, and the suggestion dismissed.
    Merge(Vec<String>, Vec<String>),
    /// Desktop's `dismissMergeSuggestion`: the keys of a suggestion hide it.
    Dismiss(Vec<String>),
    /// Desktop's `dismissChatInfoMergePromo`.
    DismissPromo,
    /// Desktop's `dismissTargetSwitcherNudge`.
    DismissNudge,
}

/// Desktop's merge suggestion (`hS`): chats that look like the same person, the name to show, and their keys.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Suggestion {
    /// The chats, in the order that found them.
    pub rooms: Vec<String>,
    /// The full name of the other person of the first chat with one, else the title of the first chat.
    pub title: String,
    /// The `tel:`, `mailto:` and `name:` keys of the chats, sorted.
    pub keys: Vec<String>,
}

/// Desktop's `isSuggestable`: a mergeable chat with a person you did not block.
fn suggestable(st: &BeeperStore, c: &Chat) -> bool {
    mergeable(c) && !st.user_blocked(c)
}

/// Desktop's `mergeSuggestions`: the groups of 2 chats or more that share a key, also through each other, the
/// biggest first, then the one of the newest activity. The views show the group of one chat, which
/// [`suggestion_of`] finds without the other groups; the tests check it against all of them.
#[cfg(test)]
pub(crate) fn merge_suggestions(st: &BeeperStore) -> Vec<Suggestion> {
    let keyed = keyed_chats(st);
    let (keys_of, by_key) = key_index(st, &keyed);
    let mut seen = vec![false; st.chats.len()];
    let mut groups: Vec<(Vec<usize>, BTreeSet<&str>)> = Vec::new();
    for &(start, _) in &keyed {
        if seen[start] {
            continue;
        }
        let (rooms, keys) = sharing(start, &keys_of, &by_key, &mut seen);
        if rooms.len() >= 2 {
            groups.push((rooms, keys));
        }
    }
    let newest = |rooms: &[usize]| rooms.iter().map(|&i| beeper_core::store::activity_ms(&st.chats[i])).max();
    groups.sort_by_cached_key(|g| std::cmp::Reverse((g.0.len(), newest(&g.0))));
    groups.into_iter().map(|(rooms, keys)| suggestion(st, rooms, keys)).collect()
}

/// Desktop's `suggestionByRoom` of the chat `id`, which Chat Info shows (`getChatInfoSuggestion`): the group of
/// `merge_suggestions` with the chat, found from the chat without the other groups.
pub(crate) fn suggestion_of(st: &BeeperStore, id: &str) -> Option<Suggestion> {
    let chat = st.resolve_chat_idx(id).filter(|&i| st.chats[i].id == id)?;
    if !suggestable(st, &st.chats[chat]) || merge_keys(&st.chats[chat]).is_empty() {
        return None;
    }
    let keyed = keyed_chats(st);
    // The chats that share a key with the chat, also through each other: each pass adds the chats with a key of the
    // chats before, until a pass adds none.
    let mut group: Vec<bool> = keyed.iter().map(|k| k.0 == chat).collect();
    let mut keys: HashSet<&str> = keyed.iter().filter(|k| k.0 == chat).flat_map(|k| &k.1).map(String::as_str).collect();
    let mut grew = true;
    while std::mem::take(&mut grew) {
        for (n, (_, ks)) in keyed.iter().enumerate() {
            if !group[n] && ks.iter().any(|k| keys.contains(k.as_str())) {
                (group[n], grew) = (true, true);
                keys.extend(ks.iter().map(String::as_str));
            }
        }
    }
    let group: Vec<(usize, Vec<String>)> = keyed.into_iter().zip(group).filter(|g| g.1).map(|g| g.0).collect();
    // `merge_suggestions` finds the group from its first chat, which gives the order of its chats. No other chat has
    // a key of the group, so the index of the group finds the same order.
    let (keys_of, by_key) = key_index(st, &group);
    let (rooms, keys) = sharing(group[0].0, &keys_of, &by_key, &mut vec![false; st.chats.len()]);
    (rooms.len() >= 2).then(|| suggestion(st, rooms, keys))
}

/// The suggestable chats with merge keys, as indices of `chats`, with their keys.
fn keyed_chats(st: &BeeperStore) -> Vec<(usize, Vec<String>)> {
    let keyed = st.chats.iter().enumerate().filter(|(_, c)| suggestable(st, c));
    keyed.map(|(i, c)| (i, merge_keys(c))).filter(|(_, k)| !k.is_empty()).collect()
}

/// The keys of each chat by its index in `chats`, and the chats of each key.
type KeyIndex<'a> = (Vec<&'a [String]>, HashMap<&'a str, Vec<usize>>);

fn key_index<'a>(st: &BeeperStore, keyed: &'a [(usize, Vec<String>)]) -> KeyIndex<'a> {
    let mut keys_of: Vec<&[String]> = vec![&[]; st.chats.len()];
    let mut by_key: HashMap<&str, Vec<usize>> = HashMap::with_capacity(keyed.len() * 3);
    for (i, keys) in keyed {
        keys_of[*i] = keys;
        for k in keys {
            by_key.entry(k).or_default().push(*i);
        }
    }
    (keys_of, by_key)
}

/// The chats that share a key with the chat `start`, also through each other, in the order that finds them, and
/// their keys. It marks them `seen`, and skips the chats that are seen already.
fn sharing<'a>(
    start: usize,
    keys_of: &[&'a [String]],
    by_key: &HashMap<&str, Vec<usize>>,
    seen: &mut [bool],
) -> (Vec<usize>, BTreeSet<&'a str>) {
    seen[start] = true;
    let (mut rooms, mut keys, mut todo) = (vec![start], BTreeSet::new(), vec![start]);
    while let Some(d) = todo.pop() {
        for k in keys_of[d] {
            keys.insert(k.as_str());
            for &l in &by_key[k.as_str()] {
                if !std::mem::replace(&mut seen[l], true) {
                    rooms.push(l);
                    todo.push(l);
                }
            }
        }
    }
    (rooms, keys)
}

/// The suggestion of the chats `rooms` with the keys `keys`.
fn suggestion(st: &BeeperStore, rooms: Vec<usize>, keys: BTreeSet<&str>) -> Suggestion {
    let full_name = |i: &usize| other(&st.chats[*i]).and_then(|u| u.full_name.clone()).filter(|n| !n.is_empty());
    Suggestion {
        title: rooms.iter().find_map(full_name).unwrap_or_else(|| st.chats[rooms[0]].title.clone()),
        rooms: rooms.into_iter().map(|i| st.chats[i].id.clone()).collect(),
        keys: keys.into_iter().map(String::from).collect(),
    }
}

impl BeeperGuiApp {
    /// [`suggestion_of`], computed once for each chat and store generation.
    pub(crate) fn suggestion_of(&self, st: &BeeperStore, id: &str) -> Option<Suggestion> {
        let Some(memo) = self.memo() else { return suggestion_of(st, id) };
        let known = memo.suggestions.borrow().get(id).cloned();
        known.unwrap_or_else(|| {
            let s = suggestion_of(st, id);
            memo.suggestions.borrow_mut().insert(id.to_string(), s.clone());
            s
        })
    }

    /// Desktop's `getMergeSuggestionForThread`, which the banner shows: the suggestion of the chat unless one of its
    /// keys was dismissed (`isSuggestionDismissed`).
    pub(crate) fn banner_suggestion(&self, st: &BeeperStore, id: &str) -> Option<Suggestion> {
        let dismissed = &st.settings.merge_suggestion_dismissed_keys;
        self.suggestion_of(st, id).filter(|s| !s.keys.iter().any(|k| dismissed.contains(k)))
    }
}

/// An SVG of `w` by `h` in one color.
fn paint<'a>(handle: &svg::Handle, w: f32, h: f32, color: Color) -> El<'a> {
    svg(handle.clone()).width(w).height(h).style(move |_, _| svg::Style { color: Some(color) }).into()
}

/// Desktop's `PlusPill` in its `bold` variant: "+ Plus" on the brand gradient.
fn bold_pill<'a>() -> El<'a> {
    let label = row![semi("+", 12.0).line_height(1.0).color(Color::WHITE), semi("Plus", 10.0).color(Color::WHITE)];
    container(label.spacing(2).align_y(Center))
        .padding([4, 6])
        .style(|_| container::Style {
            background: Some(crate::login::brand_gradient()),
            border: border::rounded(25),
            ..Default::default()
        })
        .into()
}

/// Desktop's `Button` of `size: "sm"` and `hierarchy: "secondary"`: 24 px high, 10 px to the sides, radius 6.
fn small_button<'a>(p: &Palette, label: &'static str, msg: UiMessage) -> (El<'a>, f32) {
    let w = text_width(label, 13.0, Weight::Normal) + 20.0;
    let label = txt(label, 13.0).line_height(1.0).color(p.main_text);
    let b = button(container(label).center_y(24)).padding([0, 10]).style(btn_style(
        [p.main_muted; 3],
        [p.main_text; 3],
        6.0,
    ));
    (b.on_press(msg).into(), w)
}

impl BeeperGuiApp {
    pub(crate) fn merge_suggest(&mut self, msg: Msg) -> Task<UiMessage> {
        match msg {
            Msg::Merge(rooms, keys) => {
                self.dismiss_suggestion(keys);
                self.open_merge(None, rooms)
            }
            Msg::Dismiss(keys) => {
                self.dismiss_suggestion(keys);
                Task::none()
            }
            Msg::DismissPromo => {
                if !self.store.read().settings.chat_info_merge_promo_dismissed {
                    self.store.write().update_settings(|s| s.chat_info_merge_promo_dismissed = true);
                }
                Task::none()
            }
            Msg::DismissNudge => {
                self.dismiss_nudge();
                Task::none()
            }
        }
    }

    /// Desktop's `addMergeSuggestionBannerDismissedKeys`: the new keys join the dismissed ones.
    fn dismiss_suggestion(&mut self, keys: Vec<String>) {
        let old = self.store.read().settings.merge_suggestion_dismissed_keys.clone();
        let new: Vec<String> = keys.into_iter().filter(|k| !old.contains(k)).collect();
        if !new.is_empty() {
            self.store.write().update_settings(|s| s.merge_suggestion_dismissed_keys.extend(new));
        }
    }

    /// Desktop's `dismissTargetSwitcherNudge`, from a click on the nudge or the switcher card that opens.
    pub(crate) fn dismiss_nudge(&mut self) {
        if !self.store.read().settings.target_switcher_nudge_dismissed {
            self.store.write().update_settings(|s| s.target_switcher_nudge_dismissed = true);
        }
    }

    /// The mount of Desktop's `MergeSuggestionBanner` and `TargetSwitcherNudge` with the chat that opens in the pane:
    /// each shows while the chat stays open when it can show then (`canShowSuggestionBanner`,
    /// `canShowTargetSwitcherNudge`), and counts the chat as seen.
    pub(crate) fn mount_merge_prompts(&mut self) {
        let st = self.store.read();
        let Some(idx) = self.pane.selected_chat_id.as_deref().and_then(|c| st.resolve_chat_idx(c)) else { return };
        let (id, s) = (st.chats[idx].id.clone(), &st.settings);
        let rooms = &s.merge_suggestion_seen_rooms;
        let banner =
            rooms.len() < BANNER_SHOW_LIMIT && !rooms.contains(&id) && self.banner_suggestion(&st, &id).is_some();
        // Desktop's `hasMergedChatSwitcher`: the `chat` switch targets, of 2 member chats or more, by the text composer.
        let switcher = st.members(idx).len() >= 2 && !st.chats[idx].exited();
        let rooms = &s.target_switcher_nudge_seen_rooms;
        let nudge =
            switcher && !s.target_switcher_nudge_dismissed && rooms.len() < NUDGE_SHOW_LIMIT && !rooms.contains(&id);
        drop(st);
        (self.pane.merge_banner, self.pane.nudge) = (banner, nudge);
        if banner || nudge {
            self.store.write().update_settings(|s| {
                if banner {
                    s.merge_suggestion_seen_rooms.push(id.clone());
                }
                if nudge {
                    s.target_switcher_nudge_seen_rooms.push(id);
                }
            });
        }
    }

    /// Desktop's `MergeSuggestionNetworkIcons`: the icon of each network of the chats, `size` px, each over 6 px of
    /// the one before in a cutout of the `ring` color, and past `max` icons a "+N" in the place of the last ones.
    fn suggestion_networks<'a>(
        &self,
        st: &BeeperStore,
        s: &Suggestion,
        size: f32,
        max: Option<usize>,
        ring: Color,
    ) -> El<'a> {
        let p = &self.pal;
        let mut brands: Vec<&'static str> = Vec::new();
        for c in s.rooms.iter().filter_map(|r| st.find_chat(r)) {
            let b = brand_key(&c.account_id);
            if !brands.contains(&b) {
                brands.push(b);
            }
        }
        let more = max.filter(|&m| brands.len() > m).map_or(0, |m| brands.len() - m + 1);
        brands.truncate(brands.len() - more);
        let mut slots: Vec<El<'a>> =
            brands.iter().map(|b| svg(crate::style::brand_svg(b)).width(size).height(size).into()).collect();
        if more > 0 {
            let plus = semi(format!("+{more}"), 10.0).line_height(Pixels(12.0)).color(p.muted);
            slots.push(container(plus).center(size).style(fill_box(p.prefs_group, 8.0)).into());
        }
        let step = size - ICON_OVERLAP;
        let n = slots.len();
        let mut layers: Vec<El<'a>> =
            vec![Space::new().width(size + step * n.saturating_sub(1) as f32).height(size).into()];
        for (i, slot) in slots.into_iter().enumerate() {
            let x = step * i as f32;
            if i > 0 {
                let cut = paint(&CUTOUT, size + 4.0, size, ring);
                layers.push(container(cut).padding(Padding::ZERO.left(x - 2.0)).into());
            }
            layers.push(container(slot).padding(Padding::ZERO.left(x)).into());
        }
        Stack::with_children(layers).into()
    }

    /// Desktop's `MergeSuggestionBanner` (`xmt`) under the header card of the chat: "%d chats on" over their networks,
    /// "Merge options" and Dismiss, on an elevated card whose tail points at the header. It needs the banner shown at
    /// the mount of the chat and a suggestion of the chat that was not dismissed.
    pub(crate) fn view_merge_banner<'a>(&'a self, st: &BeeperStore, chat: &Chat) -> Option<El<'a>> {
        let s = self.pane().merge_banner.then(|| self.banner_suggestion(st, &chat.id))??;
        let p = &self.pal;
        let n = s.rooms.len();
        let title = trn("%d chat on", "%d chats on", n, &[&n]);
        let icons_w = 20.0 + (20.0 - ICON_OVERLAP) * (s.rooms.len().min(BANNER_ICONS) - 1) as f32;
        let body_w = text_width(&title, 13.0, Weight::Semibold).max(icons_w);
        let msg = UiMessage::MergeSuggest;
        let (merge, merge_w) = small_button(p, tr("Merge options"), msg(Msg::Merge(s.rooms.clone(), s.keys.clone())));
        let w = (16.0 + body_w + 12.0 + merge_w + 12.0 + 28.0 + 12.0).clamp(310.0, 360.0);
        let body = column![
            semi(title, 13.0).line_height(Pixels(16.0)).wrapping(iced::widget::text::Wrapping::None).color(p.strong),
            self.suggestion_networks(st, &s, 20.0, Some(BANNER_ICONS), p.elevated),
        ]
        .spacing(6)
        .width(Fill);
        let close = button(container(ic(icon::CLOSE, 16.0).color(p.muted)).center(28))
            .padding(0)
            .style(btn_style([CLEAR; 3], [p.muted; 3], 8.0))
            .on_press(msg(Msg::Dismiss(s.keys.clone())));
        let line = row![body, merge, close].spacing(12).align_y(Center);
        let card = container(line)
            .padding(Padding { top: 16.0, right: 12.0, bottom: 16.0, left: 16.0 })
            .width(w)
            .style(raised(p.elevated, 16.0, None, (2.0, 6.0, 0x14)));
        // The card takes the pointer, so the header shows no members under it.
        let card = mouse_area(card).on_enter(UiMessage::HeaderHover(false));
        Some(column![paint(&TAIL_UP, 30.0, 11.0, p.elevated), card].align_x(Center).into())
    }

    /// Desktop's `BrandChatInfoPaneMergeChats` promo (`G7t`) in Chat Info until it is dismissed: the Plus pill and
    /// Dismiss, the networks of the chats, "View all your chats with %s as one conversation" and "Merge Chats".
    pub(crate) fn view_merge_promo<'a>(&'a self, st: &BeeperStore, chat: &Chat) -> Option<El<'a>> {
        let s = (!st.settings.chat_info_merge_promo_dismissed).then(|| self.suggestion_of(st, &chat.id))??;
        let p = &self.pal;
        let close = button(container(ic(icon::CLOSE, 16.0).color(p.muted)).center(34))
            .padding(0)
            .style(btn_style([CLEAR; 3], [p.muted; 3], 17.0))
            .on_press(UiMessage::MergeSuggest(Msg::DismissPromo));
        let top = row![bold_pill(), Space::new().width(Fill), close].align_y(Center).padding(Padding {
            top: 8.0,
            right: 8.0,
            bottom: 0.0,
            left: 16.0,
        });
        let headline = match s.title.is_empty() {
            true => tr("View all your chats with this person as one conversation").to_string(),
            false => trf("View all your chats with %s as one conversation", &[&s.title]),
        };
        let headline = semi(headline, 15.0).line_height(Pixels(20.0)).color(p.strong).align_x(Center).width(Fill);
        let open = UiMessage::Merged(merged::Msg::Open(None, s.rooms.clone()));
        let rest = column![
            self.suggestion_networks(st, &s, 20.0, None, p.chip),
            headline,
            crate::nickname::main_button(p, tr("Merge Chats"), true, Some(open), false),
        ]
        .spacing(16)
        .align_x(Center)
        .width(Fill)
        .padding(Padding { top: 0.0, right: 16.0, bottom: 20.0, left: 16.0 });
        let promo = container(column![top, rest].spacing(16)).width(Fill).style(fill_box(p.chip, 10.0));
        Some(container(promo).padding([8, 0]).into())
    }

    /// Desktop's compact `BrandChatInfoPaneMergeChats` (`K7t`) in Chat Info once the promo is dismissed: "Merged
    /// Chats" with the Plus pill, a card of "View all these chats together" over their networks that opens the merge
    /// editor, and "You can separate them anytime. Learn more".
    pub(crate) fn view_merge_card<'a>(&'a self, st: &BeeperStore, chat: &Chat) -> Option<El<'a>> {
        let s = st.settings.chat_info_merge_promo_dismissed.then(|| self.suggestion_of(st, &chat.id))??;
        let p = &self.pal;
        // The pill reaches 5 px over and under the 14 px line of the title.
        let head =
            row![txt(tr("Merged Chats"), 11.0).line_height(Pixels(14.0)).color(p.strong), Space::new().width(Fill)]
                .push(bold_pill())
                .align_y(Center)
                .padding(Padding { top: 9.0, right: 8.0, bottom: 5.0, left: 8.0 });
        let body = column![
            txt(tr("View all these chats together"), 12.0).line_height(Pixels(15.0)).color(p.strong),
            self.suggestion_networks(st, &s, 20.0, None, p.chip),
        ]
        .spacing(8)
        .width(Fill);
        let card = button(row![body, ic(icon::CHEVRON_RIGHT, 16.0).color(p.muted)].spacing(8).align_y(Center))
            .padding(10)
            .width(Fill)
            .style(btn_style([p.chip; 3], [p.strong; 3], 10.0))
            .on_press(UiMessage::Merged(merged::Msg::Open(None, s.rooms.clone())));
        let footer = rich_text![
            span::<UiMessage, _>(format!("{} ", tr("You can separate them anytime."))),
            span(tr("Learn more")).underline(true).link(UiMessage::OpenUrl(MERGE_HELP.into())),
        ]
        .on_link_click(|m: UiMessage| m)
        .size(12.0)
        .line_height(Pixels(15.0))
        .color(p.sub);
        let footer = container(footer).padding(Padding { top: 6.0, right: 8.0, bottom: 8.0, left: 8.0 });
        Some(column![head, card, footer].into())
    }

    /// Desktop's `TargetSwitcherNudge` (`SDt`) over the composer of a merged chat with its switcher: "Click the avatar
    /// to change chat." on an elevated bubble of radius 16 whose tail points down at the switcher avatar. It needs
    /// the nudge shown at the mount of the chat, and goes once dismissed.
    pub(crate) fn view_switcher_nudge<'a>(&'a self, st: &BeeperStore) -> Option<El<'a>> {
        let shown = self.pane().nudge && !st.settings.target_switcher_nudge_dismissed;
        self.merged_targets(st).filter(|t| shown && t.len() >= 2)?;
        let p = &self.pal;
        let (bg, fg) = (p.elevated, p.strong);
        let label = txt(tr("Click the avatar to change chat."), 13.0).line_height(Pixels(16.0)).color(fg);
        let bubble =
            button(label).padding([14, 16]).on_press(UiMessage::MergeSuggest(Msg::DismissNudge)).style(move |_, _| {
                button::Style {
                    background: Some(bg.into()),
                    text_color: fg,
                    border: border::rounded(16),
                    shadow: iced::Shadow {
                        color: Color::from_rgba8(0, 0, 0, 0.08),
                        offset: iced::Vector::new(0.0, 2.0),
                        blur_radius: 6.0,
                    },
                    ..Default::default()
                }
            });
        // The switcher avatar: 20 px in the 36 px line of the composer pill, 14 px into the pill after the attach
        // button, and 8 px from the bottom of the pane. The bubble is 16 px over it, its tail 32 px into it.
        let unveiled = hidden_actions(st).len() as f32;
        let plus = if self.pane().show_attach_menu { 36.0 + 6.0 + 2.0 + unveiled * 30.0 - 2.0 } else { 36.0 };
        let center = 8.0 + plus + 8.0 + 14.0 + 10.0;
        let left = (center - 32.0).max(8.0);
        let bubble = container(bubble).max_width((340.0f32).min(self.pane().size.width - 16.0));
        let tail = container(paint(&TAIL_DOWN, 30.0, 11.0, bg)).padding(Padding::ZERO.left(center - left - 15.0));
        let nudge = column![bubble, tail];
        Some(
            container(nudge)
                .align_bottom(Fill)
                .padding(Padding::ZERO.left(left).bottom(8.0 + 18.0 + 10.0 + 16.0 - 11.0))
                .into(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fuzz::demo_app;
    use beeper_core::models::{BLOCK_USER, USER_BLOCKED};
    use iced_test::simulator::simulator;

    const KAI: &str = "!ig_kai:beeper.com";
    const LENA: &str = "!sg_lena:beeper.com";
    const NUDGE: &str = "Click the avatar to change chat.";

    /// A demo app where Kai is Kai Nakamura, and the other person of Lena's chat has no name and Kai's email.
    fn same_person_app(name: &str) -> BeeperGuiApp {
        let app = demo_app(name);
        for (id, name, email) in [(KAI, Some("Kai Nakamura"), None), (LENA, None, Some(" KAI@Example.com"))] {
            let idx = app.store.read().chat_idx(id);
            app.store.write().update_chat(idx, |c| {
                let u = c.participants.items.iter_mut().find(|u| u.is_self != Some(true)).expect("the other person");
                u.full_name = name.map(String::from);
                if let Some(e) = email {
                    u.email = Some(e.into());
                }
            });
        }
        app
    }

    /// A merged chat of Kai on Instagram and Lena on Signal, open.
    fn merged_app(name: &str) -> (BeeperGuiApp, String) {
        let mut app = demo_app(name);
        let members = Some(vec![KAI.to_string(), LENA.to_string()]);
        let title = Some("Friends".to_string());
        let (chat, _) =
            app.store.write().create_or_start_chat("hungryserv", "single", title, vec![], None, members, None);
        let _ = app.update(UiMessage::SelectChat(chat.id.clone()));
        (app, chat.id)
    }

    fn clicked(app: &BeeperGuiApp, label: &str) -> Vec<UiMessage> {
        let mut sim = simulator(app.view_main());
        let _ = sim.click(label).unwrap_or_else(|_| panic!("{label}"));
        sim.into_messages().collect()
    }

    /// Desktop's `mergeSuggestions`: the direct chats whose other people share a `tel:`, `mailto:` or `name:` key,
    /// with the case and the spaces of an email folded, named after the first person with a name.
    #[test]
    fn merge_suggestions_find_the_same_person_like_desktop() {
        let app = same_person_app("suggest-keys");
        let st = app.store.read();
        let s = suggestion_of(&st, LENA).expect("Kai and Lena's chats look like one person");
        let mut rooms = s.rooms.clone();
        rooms.sort();
        assert_eq!(rooms, [KAI, LENA]);
        assert_eq!(s.title, "Kai Nakamura", "the full name of the first chat's person");
        assert_eq!(s.keys, ["mailto:kai@example.com", "name:kai nakamura"]);
        let groups = merge_suggestions(&st);
        assert!(groups.windows(2).all(|w| w[0].rooms.len() >= w[1].rooms.len()), "the biggest group comes first");
        drop(st);
        let lena = app.store.read().chat_idx(LENA);
        app.store.write().update_chat(lena, |c| c.block = BLOCK_USER | USER_BLOCKED);
        app.store.write().features.flags["features"]["*"]["block-users"] = true.into();
        assert!(app.suggestion_of(&app.store.read(), KAI).is_none(), "a blocked person is not suggested");
    }

    /// Desktop's `MergeSuggestionBanner`: "2 chats on" over their networks under the header, in the first 3 chats that
    /// show it and once in each. "Merge options" opens the merge editor with the chats and dismisses the suggestion,
    /// as Dismiss does; its keys stay dismissed in the local prefs.
    #[test]
    fn the_merge_suggestion_banner_shows_three_times_like_desktop() {
        let mut app = same_person_app("suggest-banner");
        let _ = app.update(UiMessage::SelectChat(KAI.into()));
        assert!(app.pane.merge_banner);
        assert_eq!(app.store.read().settings.merge_suggestion_seen_rooms, [KAI]);
        assert!(simulator(app.view_main()).find("2 chats on").is_ok());
        let picked = clicked(&app, "Merge options");
        let Some(UiMessage::MergeSuggest(Msg::Merge(rooms, keys))) = picked.into_iter().next() else {
            panic!("Merge options merges")
        };
        let _ = app.update(UiMessage::MergeSuggest(Msg::Merge(rooms.clone(), keys.clone())));
        let d = app.merge_window.as_ref().expect("the merge editor opens");
        assert_eq!(d.chats, rooms, "with the suggested chats");
        assert_eq!(app.store.read().settings.merge_suggestion_dismissed_keys, keys);
        assert!(simulator(app.view_main()).find("2 chats on").is_err(), "a dismissed suggestion goes");
        let _ = app.update(UiMessage::SelectChat(LENA.into()));
        assert!(!app.pane.merge_banner, "a dismissed suggestion shows no banner");

        let mut app = same_person_app("suggest-limit");
        app.store.write().update_settings(|s| s.merge_suggestion_seen_rooms = vec!["a".into(), "b".into()]);
        let _ = app.update(UiMessage::SelectChat(LENA.into()));
        assert!(app.pane.merge_banner, "the third chat shows it");
        let _ = app.update(UiMessage::SelectChat(KAI.into()));
        assert!(!app.pane.merge_banner, "the fourth does not");
        let _ = app.update(UiMessage::SelectChat(LENA.into()));
        assert!(!app.pane.merge_banner, "nor the third again");
        app.store.write().update_settings(|s| s.merge_suggestion_seen_rooms.clear());
        let _ = app.update(UiMessage::SelectChat(KAI.into()));
        let keys = app.suggestion_of(&app.store.read(), KAI).expect("a suggestion").keys;
        let _ = app.update(UiMessage::MergeSuggest(Msg::Dismiss(keys)));
        assert!(simulator(app.view_main()).find("2 chats on").is_err(), "Dismiss hides it");
    }

    /// Desktop's merge promo of Chat Info, and its compact card once the promo is dismissed. Both open the merge
    /// editor with the chats, also after the banner was dismissed.
    #[test]
    fn chat_info_offers_to_merge_like_desktop() {
        let mut app = same_person_app("suggest-info");
        let _ = app.update(UiMessage::SelectChat(KAI.into()));
        app.show_right_panel = true;
        let keys = app.suggestion_of(&app.store.read(), KAI).expect("a suggestion").keys;
        let _ = app.update(UiMessage::MergeSuggest(Msg::Dismiss(keys)));
        let opens = |m: &UiMessage| matches!(m, UiMessage::Merged(merged::Msg::Open(None, r)) if r.len() == 2);
        {
            let mut sim = simulator(app.view_info());
            assert!(sim.find("View all your chats with Kai Nakamura as one conversation").is_ok());
            assert!(sim.find("Plus").is_ok());
            let _ = sim.click("Merge Chats").expect("Merge Chats");
            assert!(sim.into_messages().any(|m| opens(&m)));
        }
        let _ = app.update(UiMessage::MergeSuggest(Msg::DismissPromo));
        assert!(app.store.read().settings.chat_info_merge_promo_dismissed);
        let mut sim = simulator(app.view_info());
        assert!(sim.find("View all your chats with Kai Nakamura as one conversation").is_err());
        assert!(sim.find("Merged Chats").is_ok());
        let _ = sim.click("View all these chats together").expect("the card");
        assert!(sim.into_messages().any(|m| opens(&m)));
    }

    /// Desktop's `TargetSwitcherNudge`: "Click the avatar to change chat." over the switcher of a merged chat, in the
    /// first 3 merged chats and once in each. A click on it or the switcher card that opens dismisses it for good.
    #[test]
    fn the_switcher_nudge_shows_three_times_like_desktop() {
        let (mut app, id) = merged_app("nudge-click");
        assert_eq!(app.store.read().settings.target_switcher_nudge_seen_rooms, [id]);
        let picked = clicked(&app, NUDGE);
        assert!(matches!(&picked[..], [UiMessage::MergeSuggest(Msg::DismissNudge)]), "{picked:?}");
        let _ = app.update(UiMessage::MergeSuggest(Msg::DismissNudge));
        assert!(simulator(app.view_main()).find(NUDGE).is_err());
        assert!(app.store.read().settings.target_switcher_nudge_dismissed);

        let (mut app, _) = merged_app("nudge-card");
        assert!(simulator(app.view_main()).find(NUDGE).is_ok());
        let _ = app.update(UiMessage::SwitchCard);
        assert!(app.store.read().settings.target_switcher_nudge_dismissed, "the switcher card dismisses it");
        assert!(simulator(app.view_main()).find(NUDGE).is_err());

        let (mut app, id) = merged_app("nudge-limit");
        let _ = app.update(UiMessage::SelectChat(KAI.into()));
        assert!(!app.pane.nudge, "another chat has no switcher");
        let _ = app.update(UiMessage::SelectChat(id.clone()));
        assert!(!app.pane.nudge, "a merged chat shows it once");
        let seen = vec!["a".into(), "b".into(), "c".into()];
        app.store.write().update_settings(|s| s.target_switcher_nudge_seen_rooms = seen);
        let _ = app.update(UiMessage::SelectChat(KAI.into()));
        let _ = app.update(UiMessage::SelectChat(id));
        assert!(!app.pane.nudge, "3 merged chats showed it");
    }

    /// Desktop's `draftSendRoomID` and `seedComposeTarget`: the draft of a merged chat keeps the target that the
    /// switcher pinned, which is pinned again when the chat opens. An unpinned target follows the activity.
    #[test]
    fn a_draft_keeps_its_compose_target_like_desktop() {
        let (mut app, id) = merged_app("draft-target");
        let _ = app.update(UiMessage::SwitchChat(KAI.into()));
        let _ = app.update(UiMessage::Composer(composer::Msg::Type("later".into())));
        let _ = app.update(UiMessage::SelectChat("!note_self:beeper.com".into()));
        let draft = app.store.read().chat(&id).draft.clone().expect("a draft");
        assert_eq!(draft.send_room_id.as_deref(), Some(KAI));
        let _ = app.update(UiMessage::SelectChat(id.clone()));
        assert_eq!(app.pane.compose_target, Some((KAI.to_string(), true)), "the draft pins its target");
        let st = app.store.read();
        assert_eq!(app.composer_placeholder(&st, st.chat(&id)), "Message Kai Nakamura on Instagram");
        drop(st);

        let (mut app, id) = merged_app("draft-unpinned");
        let _ = app.update(UiMessage::Composer(composer::Msg::Type("later".into())));
        let _ = app.update(UiMessage::SelectChat("!note_self:beeper.com".into()));
        assert_eq!(app.store.read().chat(&id).draft.clone().expect("a draft").send_room_id, None);
        let _ = app.update(UiMessage::SelectChat(id));
        assert_eq!(app.pane.compose_target, None);
    }
}
