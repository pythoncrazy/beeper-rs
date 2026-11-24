//! Desktop's space bar (rail): its spaces, their order and drags, the floating rail, and the width of the sidebar.

use super::*;
use crate::prelude::*;
use iced::widget::column;

#[derive(Clone)]
pub enum Msg {
    Select(Option<String>),
    Float(bool),
    SetOrder(Vec<SpaceOrder>),
    ResizeStart,
    ResizeMoved(Point),
    ResizeEnd,
    /// A double click on the sidebar edge gives the sidebar its first width, like Desktop.
    ResetWidth,
    /// The cursor is over a rail space, at this point in it.
    Hover(String, Point),
    Leave(String),
    Press,
    DragMoved(Point),
    DragEnd,
}

impl BeeperGuiApp {
    pub(crate) fn rail(&mut self, msg: Msg) -> Task<UiMessage> {
        match msg {
            Msg::Select(acc) => {
                self.list.chat_menu = None;
                self.list.active_tab = InboxTab::AllChats;
                self.list.selected_account_filter = acc;
                self.list.space_view_account = None;
                self.leave_draft();
            }
            Msg::SetOrder(order) => {
                self.patch_settings(|s| s.spaces_order = order);
                if let Some(d) = self.spaces_window.as_mut() {
                    let st = self.store.read();
                    d.order = rail_spaces(&st).into_iter().chain([SPACES_DIVIDER.into()]).chain(hidden_spaces(&st)).collect();
                }
            }
            Msg::ResizeStart => self.dragging = true,
            Msg::ResetWidth => self.sidebar_width = SIDEBAR_WIDTH,
            Msg::Float(on) => self.rail_float = on,
            Msg::ResizeMoved(pt) => {
                let width = pt.x - 68.0 - 2.0;
                if width >= 172.0 {
                    self.sidebar_width = width.min(732.0);
                }
                self.set_sidebar_hidden(width < 172.0);
            }
            Msg::Hover(id, at) => {
                if let Some(order) = self.rail_drag.as_mut().filter(|d| d.id != id).and_then(|d| {
                    let order = d.order.as_mut()?;
                    let from = order.iter().position(|o| *o == d.id).expect("the dragged space");
                    Some((order, from))
                }) {
                    let (order, from) = order;
                    let to = order.iter().position(|o| *o == id).expect("a rail space");
                    let item = order.remove(from);
                    order.insert(to, item);
                }
                self.rail_hover = Some((id, at));
            }
            Msg::Leave(id) => {
                if self.rail_hover.as_ref().is_some_and(|h| h.0 == id) {
                    self.rail_hover = None;
                }
            }
            Msg::Press => {
                let Some((id, grab)) = self.rail_hover.clone() else { return Task::none() };
                let last = self.rail_press.replace((id.clone(), std::time::Instant::now()));
                let double = last.is_some_and(|(s, at)| s == id && at.elapsed() < std::time::Duration::from_millis(500));
                self.rail_drag = Some(RailDrag { id: id.clone(), grab, from: None, at: Point::ORIGIN, order: None });
                // Desktop's `onDoubleClick` of a rail space, besides Create Label and a label being made.
                if double && ![LABEL_NUDGE, LABEL_DRAFT].contains(&id.as_str()) {
                    self.rail_press = None;
                    return self.update(UiMessage::Window(windows::Msg::OpenSpace(id)));
                }
            }
            Msg::DragMoved(at) => {
                let Some(d) = self.rail_drag.as_mut() else { return Task::none() };
                d.at = at;
                if d.order.is_none() && at.distance(*d.from.get_or_insert(at)) > 5.0 {
                    let st = self.store.read();
                    let draft = self.label_window.as_ref().filter(|w| w.label.is_none());
                    d.order = Some(draft.map_or_else(|| rail_spaces(&st), |w| draft_rail_spaces(&st, &w.name)));
                    self.list.chat_menu = None;
                }
            }
            Msg::DragEnd => {
                let Some(order) = self.rail_drag.take().and_then(|d| d.order) else { return Task::none() };
                let old = self.store.read().settings.spaces_order.clone();
                let order: Vec<String> = order.into_iter().map(|id| if id == LABEL_DRAFT { LABEL_NUDGE.into() } else { id }).collect();
                let shown = order
                    .iter()
                    .map(|id| SpaceOrder { id: id.clone(), visible: old.iter().find(|o| o.id == *id).is_none_or(|o| o.visible) });
                let rest = old.iter().filter(|o| !order.contains(&o.id)).cloned();
                let order = shown.chain(rest).collect();
                self.patch_settings(|s| s.spaces_order = order);
            }
            Msg::ResizeEnd => {
                self.dragging = false;
                let path = self.store.read().config.data_dir.join("sidebar-width");
                std::fs::write(path, self.sidebar_width.to_string()).expect("write the sidebar width");
            }
        }
        self.settle()
    }
}

/// Desktop's custom spaces by their space IDs, in the default order.
pub(crate) const RAIL_VIEWS: [(&str, Option<InboxTab>); 8] = [
    ("inbox", Some(InboxTab::AllChats)),
    ("archive", Some(InboxTab::Archived)),
    (LABEL_NUDGE, None),
    ("reminders", Some(InboxTab::Reminders)),
    ("scheduled", Some(InboxTab::SendLater)),
    ("requests", Some(InboxTab::Requests)),
    ("low_priority", Some(InboxTab::LowPriority)),
    ("snoozed", Some(InboxTab::Snoozed)),
];

/// Desktop's `orderedSidebarSpaces`: the spaces of the `spaces_order` setting in its order, without the hidden
/// ones, then the other spaces in the default order.
pub(crate) fn rail_spaces(st: &BeeperStore) -> Vec<String> {
    ordered_spaces(all_spaces(st), &st.settings.spaces_order)
}

/// Desktop's `setDraftLabel`: the draft label named `name` is a label space, and it takes the order slot of the nudge.
pub(crate) fn draft_rail_spaces(st: &BeeperStore, name: &str) -> Vec<String> {
    let mut all: Vec<String> = all_spaces(st).into_iter().filter(|s| s != LABEL_NUDGE).collect();
    let key = name.to_lowercase();
    let after = |s: &String| rail_label(st, s).is_some_and(|l| l.name.to_lowercase() > key);
    all.insert(all.iter().position(after).unwrap_or(all.len()), LABEL_DRAFT.into());
    let order = st.settings.spaces_order.iter().map(|o| match o.id == LABEL_NUDGE {
        true => SpaceOrder { id: LABEL_DRAFT.into(), ..o.clone() },
        false => o.clone(),
    });
    ordered_spaces(all, &order.collect::<Vec<_>>())
}

/// The `chat_menu` key of the sidebar overflow menu.
pub(crate) const RAIL_MORE: &str = "rail:more";

/// The `chat_menu` key of Desktop's space bar background menu (`rfe` without a space).
pub(crate) const RAIL_MENU: &str = "rail:menu";

/// Where `account_menu` shows: on a sidebar space, in the Edit Spaces gear menu, or on the sidebar header.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum SpaceMenu {
    Rail,
    Gear,
    Header,
    /// The rows of the Accounts settings, like Desktop's `PL` with `hideSettings` and `hideAddRemoveAccounts`.
    Prefs,
    /// The actions of an account page (`AccountActions`), Desktop's `rj` with only `hideSettings`.
    Page,
    /// The Space Options submenu of the space sheet (`sfe`), which keeps `chat_sub` at `Some(1)` while it is open.
    Options,
}

/// The account of an account or network space.
pub(crate) fn rail_owner(st: &BeeperStore, id: &str) -> String {
    network_space(st, id).map_or(id, |n| n.account_id.as_str()).to_string()
}

/// The accounts: Beeper's, then those of the bridges.
pub(crate) fn account_ids(st: &BeeperStore) -> impl Iterator<Item = &str> {
    std::iter::once("hungryserv").chain(st.accounts.iter().map(|a| a.account_id.as_str()))
}

/// The account, network and label spaces in the default order.
pub(crate) fn rail_accounts(st: &BeeperStore) -> Vec<String> {
    let mut accounts: Vec<&str> = account_ids(st).collect();
    // The first chat of each account, from one pass over the chats that ends once each account has one.
    let mut first: HashMap<&str, usize> = accounts.iter().map(|&a| (a, usize::MAX)).collect();
    let mut left = first.len();
    for (i, c) in st.chats.iter().enumerate() {
        if left == 0 {
            break;
        }
        if let Some(f) = first.get_mut(c.account_id.as_str()).filter(|f| **f == usize::MAX) {
            (*f, left) = (i, left - 1);
        }
    }
    accounts.sort_by_key(|&a| (a != "hungryserv", first[a]));
    let mut accounts: Vec<String> = accounts.into_iter().map(String::from).collect();
    let spaces = st.network_spaces.iter().filter(|n| accounts.contains(&n.account_id) && space_shown(st, n));
    accounts.extend(spaces.map(|n| n.id.clone()).collect::<Vec<_>>());
    if st.feature("labels-v2").on {
        let mut labels: Vec<&Label> = st.labels.iter().collect();
        labels.sort_by_cached_key(|l| l.name.to_lowercase());
        accounts.extend(labels.into_iter().map(|l| format!("label:{}", l.id)));
    }
    accounts
}

/// Finds the label of a rail entry.
pub(crate) fn rail_label<'a>(st: &'a BeeperStore, id: &str) -> Option<&'a Label> {
    let id = id.strip_prefix("label:")?;
    let id = st.sent.get(id).map_or(id, String::as_str);
    st.labels.iter().find(|l| l.id == id)
}

/// Desktop's `allSpaces`: WhatsApp Communities and Telegram Topics are spaces only when their setting is on.
pub(crate) fn space_shown(st: &BeeperStore, n: &NetworkSpace) -> bool {
    match account_network(&n.account_id).kind {
        NetworkKind::WhatsApp => st.settings.on("whatsappCommunities"),
        NetworkKind::Telegram => st.settings.on("telegramTopics"),
        _ => true,
    }
}

/// Finds the network space of a rail entry. Other rail entries are accounts.
pub(crate) fn network_space<'a>(st: &'a BeeperStore, id: &str) -> Option<&'a NetworkSpace> {
    st.network_spaces.iter().find(|n| n.id == id && space_shown(st, n))
}

/// Desktop's space filter: an account shows the chats that are not in one of its shown network spaces.
pub(crate) fn in_rail_entry(st: &BeeperStore, id: &str, c: &Chat) -> bool {
    if let Some(l) = rail_label(st, id) {
        return c.label_ids.contains(&l.id);
    }
    match network_space(st, id) {
        Some(n) => c.account_id == n.account_id && c.network_id.as_ref() == Some(&n.id),
        None => c.account_id == id && c.network_id.as_ref().is_none_or(|n| network_space(st, n).is_none()),
    }
}

impl BeeperGuiApp {
    /// The chats whose unread messages the space bar counts, as indices of `chats`, found once for each store
    /// generation: unread, neither archived nor a request, in the chat list, and notifying or with a mention.
    pub(crate) fn counted_chats(&self, st: &BeeperStore) -> Rc<[usize]> {
        let find = || -> Rc<[usize]> {
            let counted = |c: &Chat| {
                c.flags & CHAT_UNREAD != 0
                    && c.flags & (CHAT_ARCHIVED | CHAT_REQUEST) == 0
                    && c.listed()
                    && (c.unread_mentions > 0 || !st.notifications_muted(c))
            };
            (0..st.chats.len()).filter(|&i| counted(&st.chats[i])).collect()
        };
        let Some(memo) = self.memo() else { return find() };
        memo.counted.borrow_mut().get_or_insert_with(find).clone()
    }

    /// Desktop's account context menu (`gYe`) of a sidebar account or network space.
    pub(crate) fn view_account_menu(&self, entry: &str) -> El<'_> {
        let (menu, sub) = self.account_menu(entry, SpaceMenu::Rail);
        row![menu].push(sub).into()
    }

    /// Desktop's `kS`: the items of a label space menu, after an empty `lead` of that width.
    pub(crate) fn label_items<'a>(&self, l: &Label, lead: f32) -> Vec<El<'a>> {
        let p = &self.pal;
        let blank = || Some(Space::new().width(lead).into());
        let mute = trx(if l.muted { "Unmute" } else { "Mute" }, "label context menu action");
        let heading = (!l.name.is_empty()).then(|| menu_heading(p, l.name.clone()));
        let items = [
            menu_item(p, blank(), mute.into(), "", Some(UiMessage::Labels(labels::Msg::Mute(l.id.clone(), !l.muted)))),
            menu_item(p, blank(), tr("Label Settings").into(), "", Some(UiMessage::Labels(labels::Msg::OpenSettings(l.id.clone())))),
        ];
        heading.into_iter().chain(items).collect()
    }

    /// Desktop's space sheet (`zft`): the shown spaces with a check on the selected one, then the hidden spaces. The
    /// Space Options of an account space or the items of a label space follow, then the space actions. Returns the
    /// sheet, its width and its height.
    pub(crate) fn view_space_sheet(&self) -> (El<'_>, f32, f32) {
        let p = &self.pal;
        let st = self.store.read();
        let requests = st.chats.iter().any(|c| st.new_request(c));
        let blank = || Some(Space::new().width(22).into());
        let space = |id: &String| -> El<'_> {
            let view = RAIL_VIEWS.iter().find(|v| v.0 == id);
            let label = rail_label(&st, id);
            let owner = rail_owner(&st, id);
            let on = match view {
                Some(v) => self.list().selected_account_filter.is_none() && v.1 == Some(self.list().active_tab),
                None => self.list().selected_account_filter.as_ref() == Some(id),
            };
            let glyph: El = match (view, label) {
                (Some(v), _) if v.0 == "snoozed" => container(tinted(&MOON_STARS, 12.0, p.strong)).center(14).into(),
                (Some(v), _) => {
                    let (g, size) = match v.0 {
                        "inbox" => (icon::INBOX, 13.0),
                        "archive" => (icon::ARCHIVE, 12.0),
                        "reminders" => (icon::SCHEDULE, 12.0),
                        "scheduled" => (icon::SCHEDULE_SEND, 12.0),
                        "requests" => (icon::EMAIL, 12.0),
                        "low_priority" => (icon::LOW_PRIORITY_SPACE, 12.0),
                        _ => (icon::MAPS_UGC, 12.0),
                    };
                    let dot = (v.0 == "requests" && requests).then(|| {
                        let dot = container(Space::new()).width(6).height(6);
                        container(dot.style(bordered_box(p.accent, p.text, 1.0, 3.0))).align_right(16)
                    });
                    stack![container(ic(g, size)).center(14)].push(dot).into()
                }
                (None, Some(l)) => label_icon(l, 14.0, p.strong),
                (None, None) => match network_space(&st, id).and_then(|n| n.avatar.as_deref().map(|a| (a, &n.name))) {
                    Some((url, name)) => self.face(Some(url), name, 14.0, 0.0),
                    None => svg(brand_svg(brand_key(&owner))).width(14).height(14).into(),
                },
            };
            let muted = match (view, label) {
                (Some(_), _) => false,
                (None, Some(l)) => l.muted,
                (None, None) => !st.settings.account(&owner).notifications,
            };
            let status = space_error(&st, id);
            let badge = status.map(|s| container(semi(tr(s), 10.0)).padding([2, 6]).style(fill_box(p.chip, 9.0)));
            let line = row![
                container(on.then(|| ic(icon::CHECK, 14.0))).width(22),
                glyph,
                Space::new().width(6),
                txt(space_name(&st, id), 13.0).line_height(Pixels(rem(MENU_LINE))),
                Space::new().width(4)
            ]
            .push(muted.then(|| ic(icon::NOTIFICATIONS_OFF, 12.0)))
            .push(Space::new().width(Fill))
            .push(badge);
            menu_line(p, line, Some(space_msg(id)))
        };
        // Desktop's sheet is as wide as its longest space name, which stays on one line: the check, icon and gaps
        // (22 + 14 + 10), the row padding (26), and room for the mute icon and the unread badge (44).
        let (shown_ids, hidden_ids) = (rail_spaces(&st), hidden_spaces(&st));
        let widest = shown_ids.iter().chain(&hidden_ids).map(|id| text_width(&space_name(&st, id), 13.0, Weight::Normal));
        let width = (widest.fold(0.0, f32::max) + 22.0 + 14.0 + 10.0 + 26.0 + 44.0).max(233.0);
        let shown: Vec<El> = shown_ids.iter().map(space).collect();
        let hidden: Vec<El> = hidden_ids.iter().map(space).collect();
        let rules = 1 + usize::from(!shown.is_empty() && !hidden.is_empty());
        let mut top: Vec<El> = shown;
        if rules == 2 {
            top.push(menu_rule(p));
        }
        top.extend(hidden);
        top.push(menu_rule(p));
        let mut h = (top.len() - rules) as f32 * menu_pitch() + rules as f32 * MENU_RULE;
        let entry = self.list().selected_account_filter.as_deref();
        let options = match entry.map(|e| (e, rail_label(&st, e))) {
            Some((_, Some(l))) => {
                let items = self.label_items(l, 22.0);
                h += items.len() as f32 * menu_pitch() + MENU_RULE;
                top.extend(items.into_iter().chain([menu_rule(p)]));
                None
            }
            Some((e, None)) => Some(e),
            None => None,
        };
        let open = UiMessage::CommandBar(commands::Msg::ChatSub(Some(1)));
        let item = menu_item(p, blank(), tr("Space Options").into(), "›", Some(open.clone()));
        let options_item = options.map(|_| mouse_area(item).on_enter(open));
        let options_top = h;
        let labels = st.feature("labels-v2").show;
        let rest =
            column![menu_item(p, blank(), tr("Select Chats").into(), "", Some(UiMessage::ChatList(chat_list::Msg::ToggleSelectMode)))]
                // Desktop's `nfe`: Select Chats, then Merge Chats.
                .push(menu_item(p, blank(), tr("Merge Chats").into(), "", Some(open_merge())))
                .push(labels.then(|| {
                    menu_item(p, blank(), tr("New Label").into(), "", Some(UiMessage::Labels(labels::Msg::OpenCreate(Vec::new()))))
                }))
                .push(menu_rule(p))
                .push(menu_item(p, blank(), tr("Edit Spaces...").into(), "", Some(UiMessage::OpenEditSpaces)))
                .push(menu_item(
                    p,
                    blank(),
                    tr("Add/Remove Accounts...").into(),
                    "",
                    Some(UiMessage::Settings(settings::Msg::Open(SettingsTab::Accounts))),
                ));
        h += 8.0 + f32::from(u8::from(options.is_some()) + 4 + u8::from(labels)) * menu_pitch() + MENU_RULE;
        let top = mouse_area(Column::with_children(top)).on_enter(UiMessage::CommandBar(commands::Msg::ChatSub(None)));
        let rest = mouse_area(rest).on_enter(UiMessage::CommandBar(commands::Msg::ChatSub(None)));
        let sheet = column![top].push(options_item).push(rest);
        let sheet = crate::menu_card(p, sheet, width);
        drop(st);
        let sub = options.filter(|_| self.chat_sub.is_some()).map(|e| {
            let (menu, sub) = self.account_menu(e, SpaceMenu::Options);
            column![Space::new().height(options_top), row![menu].push(sub)]
        });
        let w = width + sub.as_ref().map_or(0.0, |_| 233.0 + if self.chat_sub == Some(0) { ACCOUNT_SUB_W } else { 0.0 });
        (row![sheet].push(sub).into(), w, h)
    }

    pub(crate) fn view_rail(&self) -> El<'_> {
        let p = &self.pal;
        let drag = self.rail_drag.as_ref().filter(|d| d.order.is_some());
        // Desktop's buttons of the space bar take the focus without showing it (`button { outline: 0 }`), and only the
        // selected space has a fill: hovering or pressing one shows none.
        let window = self.main_window;
        let rail_btn = |key: String, glyph: El<'static>, on: bool, msg: UiMessage| -> El<'static> {
            let b = button(container(glyph).center(36))
                .padding(0)
                .style(btn_style([if on { p.selected } else { CLEAR }; 3], [p.strong; 3], 14.0))
                .on_press_maybe(drag.is_none().then(|| msg.clone()));
            self.target(window, key, b, Keys::press(msg), Look::Hidden)
        };
        let quiet = self.list().chat_menu.is_some() || drag.is_some();
        let tip = |btn, label, keys| self.tip_caps(btn, label, keys, tooltip::Position::Right);
        let st = self.store.read();
        // Desktop's `${SWITCH_FIRST_9_ACCOUNTS.key}+${n}` of the first nine spaces.
        let modifier = st.settings.hotkey_keys("Switch First 9 Spaces Modifier");
        let hotkey = |n: usize| if n < 9 { hotkey_caps(&format!("{modifier}+{}", n + 1)) } else { vec![] };
        let numbers = st.settings.on("showUnreadCount");
        // A dark rich theme's `AccountBadge` dot is white with the text in `--color-from`.
        let (dot_bg, dot_fg) = p.badge.map_or((p.accent, Color::WHITE), |from| (Color::WHITE, from));
        let badge = move |btn: El<'static>, n: usize, number: bool| -> El<'static> {
            let dot = match number && numbers {
                // `.unreadWithNumber`: 4 px sides, at least 15 px wide.
                true => container(semi(unread_label(n), 10.0).color(dot_fg))
                    .center_x((crate::style::text_width(&unread_label(n), 10.0, Weight::Semibold) + 8.0).max(15.0))
                    .center_y(14)
                    .style(fill_box(dot_bg, 8.0)),
                false => container(Space::new()).width(8).height(8).style(fill_box(dot_bg, 4.0)),
            };
            let corner = (n > 0).then(|| container(dot).align_right(36).padding(Padding::ZERO.top(2.0).right(2.0)));
            stack![btn].push(corner).into()
        };
        let counted = self.counted_chats(&st);
        let tally = |key: &str, f: &dyn Fn(&Chat) -> bool| self.count(key, || counted.iter().filter(|&&i| f(&st.chats[i])).count());
        let tab = |id: &'static str, key: usize, glyph: El<'static>, t: InboxTab, n: usize, number: bool| -> El<'static> {
            let on = self.list().active_tab == t && self.list().selected_account_filter.is_none();
            let btn = badge(rail_btn(format!("{id}-button"), glyph, on, UiMessage::ChatList(chat_list::Msg::SelectTab(t))), n, number);
            let view = RAIL_VIEWS.iter().find(|v| v.1 == Some(t)).expect("a sidebar view").0;
            let btn: El = mouse_area(btn).on_right_press(UiMessage::ChatList(chat_list::Msg::OpenMenu(format!("account:{view}")))).into();
            let btn = if quiet { btn } else { tip(btn, t.label(), hotkey(key)).into() };
            container(btn).id(id).into()
        };
        let requests = self.count("requests", || st.chats.iter().filter(|c| st.new_request(c)).count());
        let g = |c: char| -> El<'static> { ic(c, 24.0).color(p.strong).into() };
        let ring_color = p.fg;
        let ring =
            svg(DASHED_RING.clone()).width(32).height(32).opacity(0.15_f32).style(move |_, _| svg::Style { color: Some(ring_color) });
        let add_space = container(stack![ring, container(ic(icon::MAPS_UGC, 24.0).color(p.strong)).center(32)]).center(36);
        let account = |i: usize, acc: String| -> El<'_> {
            let on = self.list().selected_account_filter.as_ref() == Some(&acc);
            let space = network_space(&st, &acc);
            let owner = space.map_or(acc.as_str(), |n| &n.account_id);
            let label = rail_label(&st, &acc);
            // Desktop's `JO`: a network space with its own picture shows it, else the brand logo.
            let logo: El<'_> = match (label, space.and_then(|n| n.avatar.as_deref())) {
                (Some(l), _) => label_icon(l, 28.0, p.strong),
                (None, Some(url)) => self.face(Some(url), &space.expect("a space").name, 28.0, 0.0),
                (None, None) => svg(brand_svg(brand_key(owner))).width(28).height(28).into(),
            };
            let n = tally(&format!("rail:{acc}"), &|c| {
                c.flags & CHAT_HIDDEN != 0 && c.flags & CHAT_LOW_PRIORITY == 0 && in_rail_entry(&st, &acc, c)
            });
            // Desktop's `iR`: the space name, with the account's nickname.
            let label = space_name(&st, &acc);
            let error = space_error(&st, &acc);
            let label = if let Some(s) = error { format!("{label} ({s})") } else { label };
            let select = UiMessage::Rail(rail::Msg::Select((!on).then_some(acc.clone())));
            let btn = rail_btn(format!("rail-{acc}-button"), logo, on, select);
            let btn = match error {
                Some(_) => {
                    let corner = container(svg(ERROR_CIRCLE.clone()).width(14).height(14));
                    stack![btn, corner.align_right(36).padding(Padding::ZERO.top(2.0).right(2.0))].into()
                }
                None => badge(btn, n, true),
            };
            let btn: El = mouse_area(btn).on_right_press(UiMessage::ChatList(chat_list::Msg::OpenMenu(format!("account:{acc}")))).into();
            if quiet {
                btn
            } else {
                self.tip_caps(btn, label, hotkey(i), tooltip::Position::Right).into()
            }
        };
        let labels = st.feature("labels-v2");
        let add_label = button(add_space)
            .padding(0)
            .style(btn_style([CLEAR; 3], [p.strong; 3], 14.0))
            .on_press_maybe(labels.on.then(|| UiMessage::Labels(labels::Msg::OpenCreate(Vec::new()))));
        let mut add_label = Some(add_label);
        let draft = self.label_window.as_ref().filter(|d| d.label.is_none());
        let draft_btn = |i: usize, d: &LabelDraft| -> El<'static> {
            let (l, fg) = (Label { name: d.name.clone(), color: d.color, icon: d.icon.clone(), ..Label::default() }, p.strong);
            let ring = svg(DRAFT_RING.clone()).width(30).height(30).style(move |_, _| svg::Style { color: Some(fg) });
            let logo = container(stack![ring, container(label_icon(&l, 28.0, fg)).center(30)]).into();
            let open = UiMessage::Labels(labels::Msg::OpenCreate(Vec::new()));
            let btn: El = mouse_area(rail_btn("rail-draft-button".into(), logo, false, open))
                .on_right_press(UiMessage::Labels(labels::Msg::OpenCreate(Vec::new())))
                .into();
            if quiet {
                btn
            } else {
                self.tip_caps(btn, l.name, hotkey(i), tooltip::Position::Right).into()
            }
        };
        let ids = match drag.and_then(|d| d.order.clone()) {
            Some(order) => order,
            None => draft.map_or_else(|| rail_spaces(&st), |d| draft_rail_spaces(&st, &d.name)),
        };
        // A space that the keyboard moves shows in the place it drops at.
        let items: std::sync::Arc<[String]> = ids.clone().into();
        let ids = self.key_sort_of(window, sortable::List::Rail).and_then(|s| s.moved(&ids)).unwrap_or(ids);
        let mut lifted = None;
        let spaces = ids.into_iter().enumerate().map(|(i, id)| {
            let el = match id.as_str() {
                "inbox" => tab(
                    "rail-inbox",
                    i,
                    g(icon::INBOX),
                    InboxTab::AllChats,
                    tally("rail:inbox", &|c| c.flags & (CHAT_HIDDEN | CHAT_LOW_PRIORITY) == 0),
                    true,
                ),
                "archive" => tab("rail-archive", i, g(icon::ARCHIVE), InboxTab::Archived, 0, false),
                LABEL_NUDGE => tip(add_label.take().expect("one nudge").into(), tr("Create Label"), vec![]).into(),
                LABEL_DRAFT => draft_btn(i, draft.expect("an open Create Label")),
                "reminders" => tab("rail-reminders", i, g(icon::SCHEDULE), InboxTab::Reminders, 0, false),
                "scheduled" => tab("rail-send-later", i, g(icon::SCHEDULE_SEND), InboxTab::SendLater, 0, false),
                "requests" => tab("rail-requests", i, g(icon::EMAIL), InboxTab::Requests, requests, false),
                "snoozed" => {
                    let moon = tinted(&MOON_STARS, 24.0, p.strong);
                    tab("rail-snoozed", i, moon, InboxTab::Snoozed, 0, false)
                }
                "low_priority" => tab(
                    "rail-low-priority",
                    i,
                    g(icon::LOW_PRIORITY_SPACE),
                    InboxTab::LowPriority,
                    tally("rail:low", &|c| c.flags & CHAT_LOW_PRIORITY != 0),
                    true,
                ),
                _ => account(i, id.clone()),
            };
            let el = match drag {
                Some(d) if d.id == id => {
                    lifted = Some(el);
                    Space::new().width(36).height(36).into()
                }
                _ => el,
            };
            // Desktop's spaces are `dnd-kit` sortables around their buttons: Tab shows Chromium's ring on the space,
            // then reaches its button. Enter and Space on the space pick it up for the arrows (see `sortable`).
            let pick = Self::pick(window, sortable::List::Rail, &id, &items, 1);
            let el = self.target(window, format!("space-{id}"), el, Keys::press(pick), Look::Auto);
            let hover = id.clone();
            mouse_area(el)
                .on_move(move |at| UiMessage::Rail(rail::Msg::Hover(hover.clone(), at)))
                .on_exit(UiMessage::Rail(rail::Msg::Leave(id)))
                .into()
        });
        let top: Vec<El> = spaces.collect();
        let lifted = lifted.zip(drag).map(|(el, d)| {
            let (x, y) = ((d.at.x - d.grab.x).clamp(0.0, 32.0), (d.at.y - d.grab.y).max(0.0));
            container(el).padding(Padding::ZERO.left(x).top(y))
        });
        let top = Column::with_children(top)
            .push(badge(
                rail_btn("rail-more".into(), g(icon::MORE_HORIZ), false, UiMessage::ChatList(chat_list::Msg::OpenMenu(RAIL_MORE.into()))),
                if hidden_spaces(&st).iter().any(|s| s == "requests") { requests } else { 0 },
                false,
            ))
            .spacing(8)
            .align_x(Center)
            .width(68)
            .padding(Padding::ZERO.top(14.0));
        let me = st.me();
        let media = UiMessage::Media(media_view::Msg::Open);
        let settings = UiMessage::Settings(settings::Msg::Open(SettingsTab::General));
        let me_btn = button(self.face(me.img_url.as_deref(), me.full_name.as_deref().expect("self name"), 36.0, 0.0))
            .bare(p.text)
            .on_press(settings.clone());
        let me_btn = self.target(window, "rail-settings", me_btn, Keys::press(settings), Look::Hidden);
        let footer = column![
            tip(rail_btn("rail-media".into(), g(icon::PHOTO_LIBRARY), false, media), tr("Media"), vec![]),
            // Desktop's "Add an account" opens the Add an Account window (`showAddAccountModal`).
            tip(
                rail_btn("rail-add".into(), g(icon::LIBRARY_ADD), false, UiMessage::Networks(crate::networks::Msg::Open)),
                tr("Add an account"),
                vec![]
            ),
            tip(me_btn, tr("Settings"), hotkey_caps(st.settings.hotkey_keys("Settings"))),
        ]
        .spacing(8)
        .align_x(Center)
        .padding(Padding::ZERO.bottom(16.0));
        let scroll = scrollable(top).direction(Direction::Vertical(Scrollbar::hidden())).height(Fill);
        let rail = container(column![scroll, footer].align_x(Center)).width(68).height(Fill);
        let rail =
            mouse_area(rail.style(fill_box(p.hover, 0.0))).on_right_press(UiMessage::ChatList(chat_list::Msg::OpenMenu(RAIL_MENU.into())));
        stack![rail].push(lifted).into()
    }
}

pub(crate) const LABEL_NUDGE: &str = "create-label-nudge";

/// Desktop's `allSpaces` in the default order.
pub(crate) fn all_spaces(st: &BeeperStore) -> Vec<String> {
    let nudge = st.feature("labels-v2").show && st.labels.is_empty() && !st.settings.ever_labeled;
    let views = RAIL_VIEWS.iter().filter(|v| (nudge || v.0 != LABEL_NUDGE) && v.0 != "snoozed");
    // Desktop's Snoozed space follows the accounts while snoozing is on.
    let snoozed = st.feature("snooze").on.then(|| "snoozed".to_string());
    views.map(|v| v.0.to_string()).chain(rail_accounts(st)).chain(snoozed).collect()
}

/// A press on a rail space, which Desktop's `Drop` sorts after the cursor moves 5px.
pub struct RailDrag {
    pub id: String,
    /// Where the press was in the space, where the cursor first moved in the window, and where it is.
    pub grab: Point,
    pub from: Option<Point>,
    pub at: Point,
    /// The rail order while the space moves, or none before the drag starts.
    pub order: Option<Vec<String>>,
}

/// The Edit Spaces window: the shown spaces, then `SPACES_DIVIDER`, then the hidden spaces.
pub struct SpacesDraft {
    pub id: window::Id,
    pub order: Vec<String>,
    pub search: String,
    /// The space that the drag handle moves, and the order before the drag.
    pub drag: Option<(String, Vec<String>)>,
    /// The space of the open gear menu, and where it opened.
    pub menu: Option<(String, Point)>,
    /// Whether the list scrolled down, so the search row casts a shadow.
    pub scrolled: bool,
}

/// The size of Desktop's Edit Spaces window.
pub(crate) const SPACES_SIZE: Size = Size::new(360.0, 560.0);

/// Desktop's Merge Chats item (`new D3("inbox_context_menu")`): the editor of a new merged chat.
fn open_merge() -> UiMessage {
    UiMessage::Merged(merged::Msg::Open(None, Vec::new()))
}

/// Desktop's `HIDDEN_DIVIDER_ID`.
pub(crate) const SPACES_DIVIDER: &str = "__hidden_divider__";

#[derive(Debug, Clone)]
pub enum SpacesEdit {
    Search(String),
    Drag(String),
    /// The dragged space moves to this space, or to the end for an empty ID.
    Over(String),
    Drop,
    NewLabel,
    /// The gear menu of a space, or no menu.
    Menu(Option<String>),
    /// A gear menu item.
    Act(Box<UiMessage>),
    Scrolled(bool),
}

impl BeeperGuiApp {
    /// Desktop's `rfe`: the menu of a custom space of the space bar, whose `open` item opens the space in a window,
    /// and of the space bar itself, which has none.
    pub(crate) fn rail_menu<'a>(&self, st: &BeeperStore, open: Option<El<'a>>) -> El<'a> {
        let p = &self.pal;
        let keys = |name| prefs::hotkey_label(st.settings.hotkey_keys(name));
        let bar = if st.settings.on("showSidebar") { "Hide Space Bar" } else { "Show Space Bar" };
        let accounts = UiMessage::Settings(settings::Msg::Open(SettingsTab::Accounts));
        let general = UiMessage::Settings(settings::Msg::Open(SettingsTab::General));
        let col = column![open]
            .push(menu_item(p, None, tr("Add/Remove Accounts…").into(), keys("Settings"), Some(accounts)))
            .push(menu_item(p, None, tr("Edit Spaces…").into(), "", Some(UiMessage::OpenEditSpaces)))
            .push(menu_item(p, None, tr(bar).into(), keys("Toggle Space Bar"), Some(UiMessage::RunHotkey("Toggle Space Bar"))))
            .push(menu_item(p, None, tr("Settings").into(), keys("Settings"), Some(general)));
        crate::menu_card(p, col, 233).into()
    }

    /// The menu of a sidebar space and its Show in Inbox submenu. The gear menu of the Edit Spaces window
    /// (`mYe` and `gYe`) has fewer items, and the sidebar header menu (`Ede` with `Ide` or `Ade`) has Select Chats.
    /// A label space has the label items (`kS`) instead.
    pub(crate) fn account_menu(&self, entry: &str, kind: SpaceMenu) -> (El<'_>, Option<El<'_>>) {
        let p = &self.pal;
        let st = self.store.read();
        let select = (kind == SpaceMenu::Header).then(|| {
            let item = menu_item(p, None, tr("Select Chats").into(), "", Some(UiMessage::ChatList(chat_list::Msg::ToggleSelectMode)));
            // Desktop's `nfe`: Select Chats, then Merge Chats.
            let merge = menu_item(p, None, tr("Merge Chats").into(), "", Some(open_merge()));
            mouse_area(column![item, merge]).on_enter(UiMessage::CommandBar(commands::Msg::ChatSub(None)))
        });
        if kind == SpaceMenu::Header && RAIL_VIEWS.iter().any(|v| v.0 == entry) {
            let menu = crate::menu_card(p, column![select], 233);
            return (menu.into(), None);
        }
        // Desktop's `nj`: the space menus of the space bar open the space in a window.
        let open = (kind == SpaceMenu::Rail).then(|| {
            let msg = UiMessage::Window(windows::Msg::OpenSpace(entry.into()));
            menu_item(p, None, tr("Open in New Window").into(), "", Some(msg))
        });
        if RAIL_VIEWS.iter().any(|v| v.0 == entry) {
            return (self.rail_menu(&st, open), None);
        }
        if let Some(l) = rail_label(&st, entry) {
            let item = |label: &str, msg| menu_item(p, None, label.into(), "", Some(msg));
            let tail = match kind {
                SpaceMenu::Rail => vec![
                    menu_rule(p),
                    open.expect("a space bar menu"),
                    item(tr("Edit Spaces…"), UiMessage::OpenEditSpaces),
                    item(tr("Add/Remove Accounts…"), UiMessage::Settings(settings::Msg::Open(SettingsTab::Accounts))),
                ],
                SpaceMenu::Header => vec![menu_rule(p), select.expect("the header menu has Select Chats").into()],
                SpaceMenu::Gear => vec![],
                SpaceMenu::Prefs | SpaceMenu::Page | SpaceMenu::Options => {
                    unreachable!("only account spaces have these menus")
                }
            };
            let col = Column::with_children(self.label_items(l, 0.0).into_iter().chain(tail));
            let menu = crate::menu_card(p, col, 233);
            return (menu.into(), None);
        }
        let space = network_space(&st, entry);
        let account = space.map_or(entry, |n| n.account_id.as_str());
        let bridge = match account {
            "hungryserv" => "hungryserv",
            _ => &st.bridge_accounts.iter().find(|b| b.account_id == account).expect("a rail account").bridge_id,
        };
        let (key, now) = beeper_core::matrix::inbox_rule(&st.inbox_rules, account, bridge, space.map(|n| n.id.as_str()));
        let props = st.settings.account(account);
        let muted = !props.notifications;
        let write = st.matrix.is_some();
        let options = INBOX_OPTIONS.iter().filter(|o| space.is_none() || o.0 != "dmOnly");
        let sub = (self.chat_sub == Some(0)).then(|| {
            let items = options.map(|&(o, label)| {
                let lead = container((o == now).then(|| ic(icon::CHECK, 14.0))).width(22).into();
                let msg = write.then(|| UiMessage::Accounts(accounts::Msg::SetInboxVisibility(key.clone(), o)));
                menu_item(p, Some(lead), inbox_label(label).into(), "", msg)
            });
            crate::menu_card(p, Column::with_children(items), ACCOUNT_SUB_W)
        });
        let inbox = menu_item(p, None, tr("Show in Inbox").into(), "›", Some(UiMessage::CommandBar(commands::Msg::ChatSub(Some(0)))));
        let mute = beeper_core::models::AccountProps { notifications: muted, ..props };
        let gear = kind == SpaceMenu::Gear;
        let label = match (gear, muted) {
            (true, _) => trx(if muted { "Unmute" } else { "Mute" }, "account context menu action"),
            (false, true) => tr("Unmute Space"),
            (false, false) => tr("Mute Space"),
        };
        let mute = menu_item(p, None, label.into(), "", Some(UiMessage::Accounts(accounts::Msg::SetProps(account.into(), mute))));
        let accounts = UiMessage::Settings(settings::Msg::Open(SettingsTab::Accounts));
        let settings = UiMessage::Accounts(accounts::Msg::OpenSettings(account.into()));
        // Desktop's `Zpe` after Edit Spaces… in `rj`.
        let nickname = nickname::can_nickname(&st, account).then(|| {
            let open = UiMessage::Nickname(nickname::Msg::Open(account.into()));
            menu_item(p, None, tr("Edit Account Nickname…").into(), "", Some(open))
        });
        let rest = match kind {
            SpaceMenu::Gear => column![mute],
            SpaceMenu::Header | SpaceMenu::Options => {
                column![mute, menu_rule(p), menu_item(p, None, tr("Account Settings...").into(), "", Some(settings))]
            }
            SpaceMenu::Prefs => {
                let remove = match account {
                    "hungryserv" => UiMessage::ChatList(chat_list::Msg::CloseMenu),
                    _ => UiMessage::Accounts(accounts::Msg::AskRemove(account.into())),
                };
                column![mute, menu_rule(p), menu_item(p, None, tr("Edit Spaces…").into(), "", Some(UiMessage::OpenEditSpaces)),]
                    .push(nickname)
                    .push(menu_item(p, None, tr("Remove Account…").into(), "", Some(remove)))
            }
            SpaceMenu::Page => {
                column![mute, menu_rule(p), menu_item(p, None, tr("Edit Spaces…").into(), "", Some(UiMessage::OpenEditSpaces)),]
                    .push(nickname)
                    .push(menu_item(p, None, tr("Add/Remove Accounts…").into(), "", Some(accounts)))
            }
            SpaceMenu::Rail => column![mute]
                .push(open)
                .push(menu_rule(p))
                .push(menu_item(p, None, tr("Edit Spaces…").into(), "", Some(UiMessage::OpenEditSpaces)))
                .push(nickname)
                .push(menu_item(p, None, tr("Add/Remove Accounts…").into(), "", Some(accounts)))
                .push(menu_rule(p))
                .push(menu_item(p, None, tr("Account Settings…").into(), "", Some(settings))),
        };
        // Desktop's Reconnect item (`App:89486`) of an account whose bridge asks for a new login.
        let again = reconnect(&st, account).filter(|_| kind != SpaceMenu::Gear);
        let again = again.map(|m| column![menu_rule(p), menu_item(p, None, tr("Reconnect").into(), "", Some(m))]);
        let col = column![
            select,
            mouse_area(inbox).on_enter(UiMessage::CommandBar(commands::Msg::ChatSub(Some(0)))),
            mouse_area(rest.push(again)).on_enter(UiMessage::CommandBar(commands::Msg::ChatSub((kind == SpaceMenu::Options).then_some(1))))
        ];
        let menu = crate::menu_card(p, col, 233);
        (menu.into(), sub.map(Into::into))
    }

    pub(crate) fn spaces_edit(&mut self, e: SpacesEdit) -> Task<UiMessage> {
        let d = self.spaces_window.as_mut().expect("the Edit Spaces window is open");
        match e {
            SpacesEdit::Search(q) => d.search = q,
            SpacesEdit::Scrolled(down) => d.scrolled = down,
            SpacesEdit::Drag(id) => d.drag = Some((id, d.order.clone())),
            SpacesEdit::Over(to) => {
                let Some((id, _)) = &d.drag else { return Task::none() };
                let from = d.order.iter().position(|o| o == id).expect("a dragged space");
                let to = d.order.iter().position(|o| *o == to).unwrap_or(d.order.len() - 1);
                let item = d.order.remove(from);
                d.order.insert(to, item);
            }
            SpacesEdit::Drop => {
                let Some((_, before)) = d.drag.take() else { return Task::none() };
                if d.order[0] == SPACES_DIVIDER && d.order.len() > 1 {
                    let inbox = d.order.iter().position(|o| o == "inbox").unwrap_or(1);
                    let first = d.order.remove(inbox);
                    d.order.insert(0, first);
                }
                if d.order == before {
                    return Task::none();
                }
                let split = d.order.iter().position(|o| o == SPACES_DIVIDER).expect("the divider");
                let order = d.order.iter().enumerate().filter(|(i, _)| *i != split);
                let order = order.map(|(i, id)| SpaceOrder { id: id.clone(), visible: i < split }).collect();
                let old = self.store.read().settings.spaces_order.clone();
                self.patch_settings(|s| s.spaces_order = order);
                self.act(tr("Reorder sidebar").into(), UiMessage::Rail(rail::Msg::SetOrder(old)), None);
            }
            SpacesEdit::Menu(id) => (d.menu, self.chat_sub) = (id.map(|id| (id, self.cursor.get())), None),
            SpacesEdit::Act(m) => {
                if !matches!(*m, UiMessage::CommandBar(commands::Msg::ChatSub(_))) {
                    d.menu = None;
                }
                return self.update(*m);
            }
            SpacesEdit::NewLabel => {
                let id = d.id;
                let close = self.close_window(id);
                return Task::batch([close, self.update(UiMessage::Labels(labels::Msg::OpenCreate(Vec::new())))]);
            }
        }
        Task::none()
    }

    pub(crate) fn view_edit_spaces<'a>(&'a self, d: &'a SpacesDraft) -> El<'a> {
        let p = &self.pal;
        let st = self.store.read();
        let edit = UiMessage::Spaces;
        let search = text_input(tr("Search Spaces"), &d.search)
            .id("spaces-search")
            .on_input(move |q| edit(SpacesEdit::Search(q)))
            .size(rem(13.0))
            .line_height(lh(16.0))
            .padding([14, 0])
            .style(input_style(CLEAR, p.strong, p.sub, p.accent, 0.0));
        // Desktop's controls of the sheet: plain `<button>`s whose focus does not show, and the `dnd-kit` handles,
        // which show Chromium's ring, and whose Enter and Space pick the space up for the arrows (see `sortable`).
        let window = Some(d.id);
        let plain = |key: String, b: El<'a>, msg: UiMessage| self.target(window, key, b, Keys::press(msg), Look::Hidden);
        let clear = (!d.search.trim().is_empty()).then(|| {
            let empty = edit(SpacesEdit::Search(String::new()));
            plain("clear".into(), icon_btn(p, icon::CLOSE, 16.0, 28.0, p.sub, empty.clone()), empty)
        });
        let search = row![ic(icon::SEARCH, 16.0).color(p.sub), search]
            .push(clear)
            .spacing(10)
            .align_y(Center)
            .padding(Padding::ZERO.left(16.0).right(8.0));
        let q = d.search.trim().to_lowercase();
        // The shown and the hidden spaces that the search finds.
        let found = |order: &[String]| -> (Vec<String>, Vec<String>) {
            let split = order.iter().position(|o| o == SPACES_DIVIDER).expect("the divider");
            let hit = |ids: &[String]| -> Vec<String> {
                let hit = |id: &&String| q.is_empty() || space_name(&st, id).to_lowercase().contains(&q);
                ids.iter().filter(hit).cloned().collect()
            };
            (hit(&order[..split]), hit(&order[split + 1..]))
        };
        let (shown, hidden) = found(&d.order);
        let none_hidden = hidden.is_empty();
        let items: std::sync::Arc<[String]> = shown.iter().chain([&SPACES_DIVIDER.to_string()]).chain(&hidden).cloned().collect();
        // A space that the keyboard moves shows in the place it drops at, lifted like Desktop's drag overlay.
        let sort = self.key_sort_of(window, sortable::List::Spaces);
        let (shown, hidden) = sort.and_then(|s| s.moved(&d.order)).map_or((shown, hidden), |o| found(&o));
        let line = p.fg.scale_alpha(0.15);
        let dragged = d.drag.as_ref().map(|(id, _)| id.as_str()).or(sort.map(|s| s.id.as_str()));
        let group = |ids: Vec<String>| -> Vec<El<'a>> {
            let last = ids.len().saturating_sub(1);
            ids.into_iter()
                .enumerate()
                .map(|(i, id)| {
                    let id = &id;
                    let (top, bottom) = (if i == 0 { 12.0 } else { 0.0 }, if i == last { 12.0 } else { 0.0 });
                    let muted = match rail_label(&st, id) {
                        Some(l) => l.muted,
                        None => !RAIL_VIEWS.iter().any(|v| v.0 == id) && !st.settings.account(&rail_owner(&st, id)).notifications,
                    };
                    let gear = !RAIL_VIEWS.iter().any(|v| v.0 == id) && id != LABEL_NUDGE;
                    let room = if gear { 190.0 } else { 216.0 };
                    let name = row![ell(&space_name(&st, id), room, 13.0, Weight::Semibold).color(p.strong)]
                        .push(muted.then(|| ic(icon::NOTIFICATIONS_OFF, 12.0).color(p.strong)))
                        .spacing(2)
                        .align_y(Center);
                    let sub = space_subtitle(&st, id).map(|t| txt(t, 11.0).color(p.sub));
                    let handle = mouse_area(container(tinted(&DRAG_HANDLE, 20.0, p.sub)).center(28))
                        .interaction(iced::mouse::Interaction::Grab)
                        .on_press(edit(SpacesEdit::Drag(id.clone())));
                    let pick = Self::pick(window, sortable::List::Spaces, id, &items, 1);
                    let handle = self.target(window, format!("handle-{id}"), handle, Keys::press(pick), Look::Auto);
                    let lift = dragged == Some(id.as_str());
                    let body = row![container(self.space_icon(&st, id)).center(20), column![name].push(sub).width(Fill),]
                        .push(gear.then(|| {
                            let menu = edit(SpacesEdit::Menu(Some(id.clone())));
                            let gear = mouse_area(container(ic(icon::MORE_HORIZ, 16.0).color(p.sub)).center_x(16))
                                .interaction(mouse::Interaction::Pointer)
                                .on_press(menu.clone());
                            plain(format!("gear-{id}"), gear.into(), menu)
                        }))
                        .push(handle)
                        .spacing(10)
                        .align_y(Center)
                        .padding([6, 12]);
                    let fill = p.chip;
                    let body = container(body).style(move |_: &Theme| container::Style {
                        background: Some(fill.into()),
                        border: iced::Border {
                            radius: border::Radius { top_left: top, top_right: top, bottom_right: bottom, bottom_left: bottom },
                            ..iced::Border::default()
                        },
                        shadow: if lift {
                            Shadow { color: Color::BLACK.scale_alpha(0.15), offset: Vector::new(0.0, 4.0), blur_radius: 12.0 }
                        } else {
                            Shadow::default()
                        },
                        ..Default::default()
                    });
                    let rule = (i != last).then(|| container(Space::new()).width(Fill).height(1).style(fill_box(line, 0.0)));
                    mouse_area(column![body].push(rule)).on_enter(edit(SpacesEdit::Over(id.clone()))).into()
                })
                .collect()
        };
        let header = |label: &'static str| container(txt(label, 12.0).color(p.sub)).padding([8, 0]);
        let empty = none_hidden.then(|| {
            let fill = if dragged.is_some() { CLEAR } else { p.chip };
            let msg = container(txt(tr("No Hidden Spaces"), 13.0).color(p.sub)).center_x(Fill).center_y(56).style(move |_: &Theme| {
                container::Style {
                    background: Some(fill.into()),
                    border: iced::Border { color: p.chip, width: if fill == CLEAR { 1.0 } else { 0.0 }, radius: 12.0.into() },
                    ..Default::default()
                }
            });
            mouse_area(msg).on_enter(edit(SpacesEdit::Over(String::new())))
        });
        let list = column![header(trx("Pinned", "edit spaces section header"))]
            .extend(group(shown))
            .push(mouse_area(header(tr("Hidden")).width(Fill)).on_enter(edit(SpacesEdit::Over(SPACES_DIVIDER.into()))))
            .push(empty)
            .extend(group(hidden))
            .padding(pad(0.0, 13.0, 30.0, 13.0));
        let list = scrollable(list)
            .direction(Direction::Vertical(Scrollbar::hidden()))
            .height(Fill)
            .on_scroll(move |v| edit(SpacesEdit::Scrolled(v.absolute_offset().y > 0.0)));
        let shade = d.scrolled.then(|| container(Space::new()).width(Fill).height(2).style(fill_box(p.strong.scale_alpha(0.1), 0.0)));
        let card = container(column![search, stack![list].push(shade)]).height(Fill).clip(true).style(fill_box(p.card, 12.0));
        let new = st.feature("labels-v2").show.then(|| {
            let link =
                button(txt(tr("New Label"), 13.0).color(p.accent)).bare(p.accent).padding([4, 8]).on_press(edit(SpacesEdit::NewLabel));
            // Desktop's `Button` with `rounded`.
            let keys = Keys::press(edit(SpacesEdit::NewLabel));
            let link = self.target(window, "new-label", link, keys, Look::button(p, 9999.0));
            container(link).center_x(Fill).padding(pad(8.0, 0.0, 4.0, 0.0))
        });
        let head = container(semi(tr("Edit Spaces"), 13.0).color(p.strong)).center_x(Fill).padding([8, 0]);
        let window = container(column![head, card].push(new)).padding(8).style(fill_box(p.window, 0.0));
        let window = mouse_area(window).on_release(edit(SpacesEdit::Drop));
        let Some((id, at)) = &d.menu else {
            drop(st);
            return window.into();
        };
        let h = 8.0 + menu_pitch() * rail_label(&st, id).map_or(2.0, label_rows);
        drop(st);
        // The window is resizable, so the menu stays in the window's size.
        let menus = iced::widget::responsive(move |size| {
            let (menu, sub) = self.account_menu(id, SpaceMenu::Gear);
            let x = if at.x + 233.0 > size.width { at.x - 233.0 } else { at.x }.max(4.0);
            let y = at.y.min(size.height - h - 8.0);
            let sub_x = (x + 233.0).min(size.width - ACCOUNT_SUB_W - 4.0);
            let act = |m| UiMessage::Spaces(SpacesEdit::Act(Box::new(m)));
            let place = |m: El<'a>, x: f32| -> El<'a> {
                let m = mouse_area(m.map(act)).on_press(UiMessage::Noop);
                container(m).padding(Padding::ZERO.left(x).top(y)).into()
            };
            let close = crate::backdrop(container(Space::new()).width(Fill).height(Fill))
                .on_press(edit(SpacesEdit::Menu(None)))
                .on_right_press(edit(SpacesEdit::Menu(None)));
            stack![close, place(menu, x)].push(sub.map(|s| place(s, sub_x))).into()
        });
        stack![window, menus].into()
    }
}
