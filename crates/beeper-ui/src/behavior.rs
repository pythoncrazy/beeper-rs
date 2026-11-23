//! Desktop's `BrandChatInfoPaneBehavior`: the Chat Info line that tells what hides or mutes a chat, its "Visibility and
//! Notifications" menu of the sources that take part, and the How It Works window. Labels have no snooze or active
//! hours here, so Desktop's `away` sources and label snoozes never take part.

use super::*;
use crate::i18n::trxf;
use iced::widget::column;

/// The menu key of the behavior line of a chat.
pub(crate) const BEHAVIOR_MENU: &str = "behavior:";
/// Desktop's How It Works window, `v3(341, 487)`.
pub(crate) const HOW_SIZE: Size = Size::new(341.0, 487.0);

/// The kinds of Desktop's behavior sources, in the order of its `OK`.
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    /// The chat's own snooze (`uce`).
    Snooze,
    /// The chat's own mute.
    Chat,
    Label,
    /// The chat account, for a chat without labels.
    Network,
}

#[derive(Clone)]
struct Source {
    kind: Kind,
    id: String,
    name: String,
}

impl Source {
    /// Desktop's `yM`.
    fn same(&self, other: Option<&Source>) -> bool {
        other.is_some_and(|o| o.kind == self.kind && o.id == self.id)
    }
}

/// The sections of Desktop's `jK`, in order. The `inactive` labels are the away and snoozed ones.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Section {
    Chat,
    Network,
    Active,
}

struct Contributor {
    section: Section,
    source: Source,
    hides: bool,
    mutes: bool,
    /// Desktop's `decidesVisibility || decidesNotifications`.
    decides: bool,
}

/// Desktop's `kP` over `DP`: whether the chat is hidden from the inbox and muted, the sources that decide it, and
/// each source that takes part.
pub(crate) struct Behavior {
    hidden: bool,
    muted: bool,
    visibility: Option<Source>,
    notification: Option<Source>,
    contributors: Vec<Contributor>,
    /// The end of the chat's own snooze.
    snoozed: Option<i64>,
}

/// Desktop's `matchesInboxVisibility`: "Only DMs" hides groups that are not group DMs.
fn shows(visibility: Option<&str>, c: &Chat) -> bool {
    match visibility {
        Some("none") => false,
        Some("dmOnly") => !c.inbox_group,
        _ => true,
    }
}

/// Desktop's `ru`: the nickname of an account, or its network.
pub(crate) fn account_name(st: &BeeperStore, account_id: &str) -> String {
    st.custom_name(account_id).unwrap_or(network_name(account_id)).to_string()
}

/// Desktop's `oKe`: the behavior of a chat while chat snoozes or label schedules are on (`snoozeOrSchedulesEnabled`),
/// unless it shows and notifies with nothing to tell.
pub(crate) fn behavior(st: &BeeperStore, c: &Chat) -> Option<Behavior> {
    let labels_on = st.feature("labels-v2").on;
    if !st.feature("snooze").on && !(st.whoami.internal && labels_on) {
        return None;
    }
    if let Some(at) = reminders::snoozed_until(st, c) {
        let s = Source { kind: Kind::Snooze, id: "chat".into(), name: String::new() };
        let entry = Contributor { section: Section::Chat, source: s.clone(), hides: true, mutes: true, decides: true };
        let (visibility, notification) = (Some(s.clone()), Some(s));
        return Some(Behavior {
            hidden: true,
            muted: true,
            visibility,
            notification,
            contributors: vec![entry],
            snoozed: Some(at),
        });
    }
    // Desktop's `labelContributions` of a chat with labels, each `normal`: it hides unless its "Show in Inbox" shows
    // the chat, and mutes when the label is muted.
    let labels: Vec<(Source, bool, bool)> = match labels_on {
        true => c
            .label_ids
            .iter()
            .map(|id| {
                let l = st.labels.iter().find(|l| l.id == *id);
                let name = l.map(|l| l.name.trim()).filter(|n| !n.is_empty());
                let name = name
                    .unwrap_or(trx("Unknown label", "chat behavior explanation, a label whose details are missing"));
                let source = Source { kind: Kind::Label, id: id.clone(), name: name.into() };
                (source, !shows(l.and_then(|l| l.inbox), c), l.is_some_and(|l| l.muted))
            })
            .collect(),
        false => Vec::new(),
    };
    let chat = c.flag(ChatFlag::Muted).then(|| Source { kind: Kind::Chat, id: "chat".into(), name: String::new() });
    let network = labels.is_empty().then(|| {
        let name = account_name(st, &c.account_id);
        Source { kind: Kind::Network, id: c.account_id.clone(), name }
    });
    let net_mutes = network.clone().filter(|_| chat.is_none() && !st.settings.account(&c.account_id).notifications);
    let net_hides = network.clone().filter(|_| c.is_hidden_from_inbox == Some(true));
    let hidden = match labels.is_empty() {
        true => net_hides.is_some(),
        false => labels.iter().any(|l| l.1),
    };
    let muted = chat.is_some() || (!labels.is_empty() && labels.iter().all(|l| l.2)) || net_mutes.is_some();
    // Desktop's `FK`: the first deciding label by name.
    let first = |pick: &dyn Fn(&(Source, bool, bool)) -> bool| {
        let picked = labels.iter().filter(|l| pick(l)).map(|l| &l.0);
        picked.min_by(|a, b| locale_cmp(&a.name, &b.name).then_with(|| a.id.cmp(&b.id))).cloned()
    };
    let visibility = net_hides.clone().or_else(|| first(&|l| l.1 == hidden));
    let notification = chat.clone().or(net_mutes.clone()).or_else(|| first(&|l| l.2 == muted));
    if visibility.is_none() && notification.is_none() {
        return None;
    }
    let entry = |section, source: Source, hides, mutes| Contributor {
        section,
        decides: source.same(visibility.as_ref()) || source.same(notification.as_ref()),
        source,
        hides,
        mutes,
    };
    let network = network.filter(|_| net_mutes.is_some() || net_hides.is_some());
    let mut contributors: Vec<Contributor> = (chat.map(|s| entry(Section::Chat, s, false, true)).into_iter())
        .chain(network.map(|s| entry(Section::Network, s, net_hides.is_some(), net_mutes.is_some())))
        .chain(labels.into_iter().map(|(s, hides, mutes)| entry(Section::Active, s, hides, mutes)))
        .collect();
    contributors.sort_by(|a, b| {
        (a.section.cmp(&b.section))
            .then_with(|| b.decides.cmp(&a.decides))
            .then_with(|| locale_cmp(&a.source.name, &b.source.name))
    });
    Some(Behavior { hidden, muted, visibility, notification, contributors, snoozed: None })
}

/// Desktop's `Gp`: the time today, or the day and the time.
fn until(at: i64, h24: bool) -> String {
    match day_of(at) == today() {
        true => clock(at, h24),
        false => scheduled::send_day(at, h24),
    }
}

impl Behavior {
    /// Desktop's `dKe`: the line of a hidden, muted or snoozed chat. No source here has an end but the chat's snooze.
    pub(crate) fn line(&self, h24: bool) -> Option<String> {
        if let Some(at) = self.snoozed {
            return Some(trxf("Snoozed until %s", "chat info line", &[&until(at, h24)]));
        }
        let (hidden, muted) = (self.hidden, self.muted);
        if !hidden && !muted {
            return None;
        }
        let deciding: Vec<&Source> = [hidden.then_some(&self.visibility), muted.then_some(&self.notification)]
            .into_iter()
            .flatten()
            .flatten()
            .collect();
        let one = match deciding[..] {
            [s] => Some(s),
            [a, b] if a.same(Some(b)) => Some(a),
            _ => None,
        };
        Some(match (one, hidden, muted) {
            (Some(s), ..) if s.kind == Kind::Chat => trx("Muted by you", "chat info line").into(),
            (Some(s), true, true) => trxf("Hidden and muted by %s", "chat info line", &[&s.name]),
            (Some(s), true, false) => trxf("Hidden by %s", "chat info line", &[&s.name]),
            (Some(s), ..) => trxf("Muted by %s", "chat info line", &[&s.name]),
            (None, true, true) => trx("Hidden and muted", "chat info line").into(),
            (None, true, false) => trx("Hidden", "chat info line").into(),
            (None, ..) => trx("Muted", "chat info line").into(),
        })
    }

    /// Desktop's `iKe`: the icon of the line.
    pub(crate) fn icon(&self) -> &'static svg::Handle {
        let kinds = [&self.visibility, &self.notification];
        match kinds.iter().any(|s| s.as_ref().is_some_and(|s| s.kind == Kind::Snooze)) {
            true => &MOON_STARS,
            false => &TUNE,
        }
    }
}

/// Desktop's `hce`: what a source does to the chat, below its name in the menu.
fn sublabel(c: &Contributor, st: &BeeperStore, snoozed: Option<i64>, h24: bool) -> String {
    // Desktop's `pKe`: a label whose "Show in Inbox" is "Only DMs".
    let dm_only =
        || c.source.kind == Kind::Label && st.labels.iter().any(|l| l.id == c.source.id && l.inbox == Some("dmOnly"));
    match (c.source.kind, c.hides, c.mutes) {
        (Kind::Snooze, ..) => {
            trxf("Snoozes until %s", "chat info menu row", &[&until(snoozed.expect("a snoozed chat"), h24)])
        }
        (_, true, false) if dm_only() => trx("Shows only DMs", "chat info menu row").into(),
        (_, true, true) => trx("Hides and mutes this chat", "chat info menu row").into(),
        (_, true, false) => trx("Hides this chat", "chat info menu row").into(),
        (_, false, true) => trx("Mutes this chat", "chat info menu row").into(),
        (_, false, false) => trx("Shows and notifies", "chat info menu row").into(),
    }
}

impl BeeperGuiApp {
    /// Desktop's `P0e`: the behavior line of Chat Info, which opens the Visibility and Notifications menu.
    pub(crate) fn view_behavior_line<'a>(&self, st: &BeeperStore, chat: &Chat) -> Option<El<'a>> {
        let p = &self.pal;
        let b = behavior(st, chat)?;
        let line = b.line(st.settings.pref("timeFormat") == 1)?;
        let dim = p.sub;
        let text =
            txt(line, 12.0).line_height(Pixels(rem(15.0))).color(p.strong).wrapping(iced::widget::text::Wrapping::None);
        let label =
            row![tinted(b.icon(), 14.0, dim), text, tinted(&CHEVRON_FORWARD, 10.0, dim)].spacing(5).align_y(Center);
        let open = UiMessage::ChatList(chat_list::Msg::OpenMenu(format!("{BEHAVIOR_MENU}{}", chat.id)));
        let btn = button(label).padding(0).style(btn_style([CLEAR; 3], [p.strong; 3], 0.0)).on_press(open);
        Some(container(btn).center_x(Fill).clip(true).padding(Padding::ZERO.top(4.0).bottom(8.0)).into())
    }

    /// Desktop's `b7t` menu of the behavior line: the heading, each source with what it does, and How It Works. Each
    /// row comes with its height, and the menu with its width.
    pub(crate) fn behavior_items(&self, st: &BeeperStore, cid: &str) -> (Vec<(El<'_>, f32)>, f32) {
        let p = &self.pal;
        let c = st.chat(cid);
        let h24 = st.settings.pref("timeFormat") == 1;
        let close = |msg| UiMessage::Commands(vec![UiMessage::ChatList(chat_list::Msg::CloseMenu), msg]);
        let heading = tr("Visibility and Notifications").to_string();
        let mut width = text_width(&heading, 11.0, Weight::Medium) + 26.0;
        let mut rows = vec![(menu_heading(p, heading), menu_pitch())];
        let Some(b) = behavior(st, c) else { return (rows, width) };
        for entry in &b.contributors {
            let s = &entry.source;
            let (name, icon): (String, El) = match s.kind {
                Kind::Snooze => {
                    (trx("Chat Snooze", "chat behavior explanation row").into(), tinted(&MOON_STARS, 16.0, p.text))
                }
                Kind::Chat => {
                    (trx("Chat Settings", "chat behavior explanation row").into(), tinted(&TUNE, 16.0, p.text))
                }
                Kind::Network => (s.name.clone(), svg(brand_svg(brand_key(&s.id))).width(16).height(16).into()),
                Kind::Label => {
                    let label = st.labels.iter().find(|l| l.id == s.id).cloned().unwrap_or_default();
                    let glyph = if label_fill(label.color).is_some() { 10.0 } else { 13.0 };
                    (s.name.clone(), label_tile(&label, 13.0, glyph, 13.0 * 0.35, p.text))
                }
            };
            let sub = sublabel(entry, st, b.snoozed, h24);
            width = width.max(
                16.0 + 5.0
                    + text_width(&name, 13.0, Weight::Normal).max(crate::style::fixed_width(
                        &sub,
                        11.0,
                        Weight::Normal,
                    ))
                    + 26.0,
            );
            // Desktop's `CO` opens the Label Settings of a label, and `M0e` the settings of an account.
            let msg = match s.kind {
                Kind::Label => UiMessage::Labels(labels::Msg::OpenSettings(s.id.clone())),
                Kind::Network => UiMessage::Accounts(accounts::Msg::OpenSettings(s.id.clone())),
                Kind::Snooze | Kind::Chat => UiMessage::ChatList(chat_list::Msg::CloseMenu),
            };
            let stack = column![
                txt(name, 13.0).line_height(Pixels(rem(MENU_LINE))),
                crate::style::fixed(sub, 11.0, Weight::Normal).color(p.sub)
            ];
            let line = row![container(icon).center_x(16), stack].spacing(14);
            rows.push((menu_line(p, line, Some(close(msg))), menu_pitch() + 11.0 * 1.4));
        }
        let how = tr("How It Works");
        width = width.max(14.0 + 14.0 + text_width(how, 13.0, Weight::Normal) + 26.0);
        let info = container(tinted(&INFO_ICON, 14.0, p.text)).center_x(16);
        let how = menu_item(
            p,
            Some(row![info].padding(Padding::ZERO.right(5.0)).into()),
            how.into(),
            "",
            Some(close(UiMessage::Info(info::Msg::HowItWorks(true)))),
        );
        rows.extend([(menu_rule(p), 11.0), (how, menu_pitch())]);
        (rows, width.clamp(160.0, 360.0))
    }

    /// Desktop's How It Works window (`nrt`): the rules of the visibility and the notifications of a chat.
    pub(crate) fn view_how_it_works(&self) -> El<'_> {
        let p = &self.pal;
        let rules = [
            (&ACCOUNT_CIRCLE, tr("You Come First"), tr("If you mute or snooze a chat, labels can’t change it.")),
            (
                &SELL,
                tr("Labels Set the Rules"),
                tr("If any label hides a chat, it’s hidden. If any label notifies, you’re notified."),
            ),
            (
                &EVENT_BUSY,
                tr("Away and Snoozed Labels"),
                tr("Outside active hours or while snoozed, a label hides or mutes its chats, unless another label applies right now."),
            ),
            (
                &ALTERNATE_EMAIL,
                tr("Mentions"),
                tr("Mentions notify in muted chats. Outside active hours, only if you allow them. Never while snoozed."),
            ),
            (&FORUM, tr("Chats Without a Label"), tr("They follow their chat app account’s settings.")),
        ];
        let rows = rules.into_iter().map(|(icon, title, body)| -> El<'_> {
            let icon = container(tinted(icon, 18.0, p.strong)).center(28);
            let text = column![
                med(title, 13.0).line_height(Pixels(rem(16.0))).color(p.strong),
                txt(body, 13.0).line_height(Pixels(rem(16.0))).color(p.sub)
            ]
            .spacing(2)
            .padding(Padding::ZERO.left(4.0).top(5.0));
            row![icon, text].into()
        });
        let head = column![
            semi(tr("How It Works"), 15.0).line_height(Pixels(rem(20.0))).color(p.strong),
            txt(tr("Visibility and Notifications"), 13.0).line_height(Pixels(rem(16.0))).color(p.sub)
        ];
        let body = column![head, Column::with_children(rows).spacing(16).padding(Padding::ZERO.top(16.0))].padding(16);
        let done = button(med(tr("Done"), 13.0).color(Color::WHITE))
            .padding([6, 14])
            .style(btn_style([p.accent; 3], [Color::WHITE; 3], 999.0))
            .on_press(UiMessage::Info(info::Msg::HowItWorks(false)));
        let footer = container(done).align_right(Fill).align_bottom(Fill).padding(12);
        let card = stack![scrollable(body).direction(bar()).style(scroll_style(p.scroll)).height(Fill), footer];
        let card = container(card).height(Fill).clip(true).style(fill_box(p.card, 12.0));
        let head = container(semi(tr("How It Works"), 13.0).color(p.strong)).center_x(Fill).padding([8, 0]);
        container(column![head, card]).padding(8).style(fill_box(p.window, 0.0)).into()
    }
}
