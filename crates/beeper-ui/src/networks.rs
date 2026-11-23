//! Desktop's `AddNewAccounts`: the networks that take a new account, as a grid or as the rows of a search, the
//! accounts menu of a network, the footer of the accounts that remain, and the Add an Account window
//! (`AddAccountsBrowserWindow`).

use super::*;
use beeper_core::config::{cloud_bridge, NETWORK_SPECS};
use beeper_core::features::Flags;
use beeper_core::models::NetworkKind;
use iced::widget::{column, row, stack};

/// Desktop's Add an Account window size (`Ds` and `Us` of sentry-common).
pub const SIZE: Size = Size::new(400.0, 505.0);
/// Desktop's `IVt`: the account photos on the chip of a tile.
const FACES: usize = 4;
/// Desktop's `NetworkAccountsMenu` width.
const MENU_W: f32 = 220.0;

/// The Add an Account window and its search, and the open accounts menu of a network.
#[derive(Default)]
pub struct Networks {
    pub window: Option<window::Id>,
    pub query: String,
    /// Desktop's `L0e` state: the network of the open menu, and where it opens.
    pub menu: Option<(&'static str, Point)>,
}

#[derive(Clone)]
pub enum Msg {
    /// Desktop's `showAddAccountModal`.
    Open,
    Query(String),
    /// Desktop's `wj`: a network connects its Beeper Cloud bridge, or opens the upgrade.
    Connect(&'static str),
    Menu(Option<&'static str>),
    /// An account of the menu: Desktop's `M0e`, which opens its settings.
    Account(String),
}

/// Desktop's `addableNetworks` entry of a network on Linux, where beeper-rs adds accounts on the Beeper Cloud
/// bridges only.
pub struct Entry<'a> {
    pub network: &'static NetworkSpec,
    pub accounts: Vec<&'a BridgeAccountInfo>,
    /// `connectPlatform`: the bridge that takes a new account.
    pub connect: Option<&'static str>,
    /// `isUpgradable`: a paid plan allows another account.
    pub upgradable: bool,
}

impl Entry<'_> {
    /// Desktop's `D0e`: the network takes no account here, and its tile is gray.
    pub fn blocked(&self) -> bool {
        self.connect.is_none() && !self.upgradable
    }

    /// Desktop's `AVt` tooltip, which is also the label of its tile.
    fn tip(&self) -> String {
        let name = self.network.display_name;
        match (self.connect, self.upgradable, self.accounts.is_empty()) {
            (Some(_), _, false) => tr("Add another").into(),
            (Some(_), ..) => trf("Connect %s", &[&name]),
            (None, true, _) => trf("Upgrade to connect %s", &[&name]),
            (None, false, false) => trf("You can’t add another %s account", &[&name]),
            (None, false, true) => trf("Can't add %s", &[&name]),
        }
    }
}

/// The networks that you can add an account on here: Desktop's `addablePlatforms`, whose bridges the account's flags
/// turn on, and iMessage on a Mac.
pub(crate) fn addable(f: &Flags) -> impl Iterator<Item = &'static NetworkSpec> + '_ {
    NETWORK_SPECS[1..].iter().filter(|n| match n.kind {
        NetworkKind::IMessage => cfg!(target_os = "macos"),
        _ => f.network_on(n),
    })
}

/// Desktop's `addableNetworks`, in its order: a network connects when its bridge takes another account
/// (`canAddAccountToBridge`) and the plan allows one there and in total; else it is upgradable when the plan caps it.
pub fn entries(st: &BeeperStore) -> Vec<Entry<'_>> {
    let f = &st.features;
    let out = f.accounts_remaining() == 0;
    addable(f)
        .map(|n| {
            let accounts: Vec<_> =
                st.bridge_accounts.iter().filter(|a| account_network(&a.bridge_id).bridge_id == n.bridge_id).collect();
            let bridge = cloud_bridge(n).filter(|b| accounts.is_empty() || f.multi_account(b));
            let capped = bridge.is_some() && f.plan_capped(n);
            let open = bridge.filter(|_| !capped);
            let connect = open.filter(|_| !out);
            Entry { network: n, accounts, connect, upgradable: connect.is_none() && (capped || open.is_some()) }
        })
        .collect()
}

/// Desktop's `R0e`: the phone, the `@username` or the email of an account's user.
fn identifier(a: &BridgeAccountInfo) -> Option<String> {
    let p = a.remote_profile.as_ref()?;
    p.phone.as_deref().map(fmt_phone).or_else(|| p.username.as_ref().map(|u| format!("@{u}"))).or(p.email.clone())
}

/// The search field of the networks: a 16 px search icon, and Desktop's `rightIcons` "cancel" that clears it.
pub fn search_field<'a>(p: &Palette, query: &str, on: fn(String) -> UiMessage) -> El<'a> {
    row![
        ic(icon::SEARCH, 16.0).color(p.tertiary),
        text_input(tr("Search..."), query)
            .on_input(on)
            .size(rem(14.0))
            .padding([10, 0])
            .style(input_style(CLEAR, p.fg, p.tertiary, CLEAR, 0.0)),
    ]
    .push((!query.is_empty()).then(|| icon_btn(p, icon::CLOSE, 14.0, 20.0, p.tertiary, on(String::new()))))
    .spacing(10)
    .align_y(Center)
    .padding([0, 16])
    .into()
}

impl BeeperGuiApp {
    pub(crate) fn networks(&mut self, msg: Msg) -> Task<UiMessage> {
        self.networks.menu = None;
        match msg {
            Msg::Open => {
                if let Some(id) = self.networks.window {
                    return window::gain_focus(id);
                }
                let (id, open) = self.open_window(window::Settings {
                    resizable: false,
                    min_size: None,
                    position: window::Position::Centered,
                    ..Self::window_settings(SIZE)
                });
                (self.networks.window, self.networks.query) = (Some(id), String::new());
                return open.discard();
            }
            Msg::Query(q) => self.networks.query = q,
            Msg::Menu(network) => self.networks.menu = network.map(|n| (n, self.cursor.get())),
            // A demo account adds none.
            Msg::Connect(_) if self.store.read().config.demo => {}
            Msg::Connect(network) => {
                let st = self.store.read();
                let entry = entries(&st).into_iter().find(|e| e.network.bridge_id == network).expect("a network");
                let (connect, upgradable) = (entry.connect, entry.upgradable);
                drop(st);
                // Desktop's `openSubscriptionInfo` of the upgrade, and `openNewAccountInNewWindow`, which closes this
                // window.
                if upgradable {
                    return self.update(UiMessage::Settings(settings::Msg::Open(SettingsTab::BeeperPlus)));
                }
                let close = self.networks.window.map_or_else(Task::none, window::close);
                return match connect {
                    Some(_) => Task::batch([close, self.open_add_account(network, None)]),
                    None => Task::none(),
                };
            }
            Msg::Account(id) => {
                let close = self.networks.window.map_or_else(Task::none, window::close);
                return Task::batch([close, self.accounts(accounts::Msg::OpenSettings(id))]);
            }
        }
        Task::none()
    }

    /// Desktop's `DVt`: the grid of the networks, or the rows of the ones whose name has `query`.
    pub(crate) fn view_networks(&self, st: &BeeperStore, query: &str) -> El<'_> {
        let p = &self.pal;
        let q = query.trim().to_lowercase();
        let entries = entries(st);
        if !q.is_empty() {
            let hits = entries
                .iter()
                .filter(|e| e.network.display_name.to_lowercase().contains(&q) || e.network.bridge_id.contains(&q));
            let rows: Vec<El<'_>> = hits.map(|e| self.network_row(e)).collect();
            if rows.is_empty() {
                let empty = txt(tr("No networks found"), 12.0).color(p.sub);
                return container(empty).center_x(Fill).padding([24, 8]).into();
            }
            let rule = || container(Space::new()).width(Fill).height(1).style(fill_box(p.prefs_border, 0.0)).into();
            let rows = rows.into_iter().enumerate().flat_map(|(i, r)| [(i > 0).then(rule), Some(r)]).flatten();
            return Column::with_children(rows).width(Fill).into();
        }
        let tiles: Vec<El<'_>> = entries.iter().map(|e| self.network_tile(e)).collect();
        let mut rows = Vec::new();
        let mut tiles = tiles.into_iter().peekable();
        while tiles.peek().is_some() {
            let cells: Vec<El<'_>> = tiles.by_ref().take(4).map(|t| container(t).center_x(Fill).into()).collect();
            let pad = (cells.len()..4).map(|_| Space::new().width(Fill).into());
            rows.push(row(cells.into_iter().chain(pad)).into());
        }
        column(rows).spacing(24).padding(8).into()
    }

    /// Desktop's `AVt`: the logo and the name, gray when the network takes no account, with its tooltip, and the chip
    /// of its accounts, which opens their menu. A gray tile with accounts opens the menu too.
    fn network_tile(&self, e: &Entry<'_>) -> El<'_> {
        let p = &self.pal;
        let id = e.network.bridge_id;
        let logo = if e.blocked() { brand_gray(id) } else { brand_svg(id) };
        let tile = column![svg(logo).width(36).height(36), semi(e.network.display_name, 11.0).color(p.fg).center()]
            .spacing(10)
            .align_x(Center)
            .width(Fill);
        let press = match (e.blocked(), e.accounts.is_empty()) {
            (false, _) => Some(Msg::Connect(id)),
            (true, false) => Some(Msg::Menu(Some(id))),
            (true, true) => None,
        };
        let tile = button(tile)
            .padding(8)
            .width(75)
            .style(btn_style([CLEAR; 3], [p.fg; 3], 10.0))
            .on_press_maybe(press.map(UiMessage::Networks));
        let tip = container(txt(e.tip(), 13.0).color(p.strong))
            .padding([6, 12])
            .style(shadow_box(p.elevated, p.chip, 16.0, 10.0));
        let chip = (!e.accounts.is_empty()).then(|| {
            // Desktop's `networkTileAvatars`: each photo 4 px over the next one, the first on top.
            let n = e.accounts.len().min(FACES);
            let faces = e.accounts[..n].iter().enumerate().rev().map(|(i, a)| {
                let r = a.remote_profile.as_ref();
                let face = self.face(
                    r.and_then(|r| r.avatar.as_deref()),
                    r.and_then(|r| r.name.as_deref()).unwrap_or(""),
                    12.0,
                    0.0,
                );
                pin(face).x(i as f32 * 8.0).into()
            });
            let faces = Stack::with_children(faces).width(12.0 + (n - 1) as f32 * 8.0).height(12);
            let add = (!e.blocked()).then(|| ic(icon::PLUS, 10.0).color(p.fg));
            let fill = Color { a: if self.dark { 0.1 } else { 0.06 }, ..p.fg };
            let chip = row![faces].push(add).spacing(2).align_y(Center);
            button(chip)
                .padding(2)
                .style(btn_style([fill; 3], [p.fg; 3], 999.0))
                .on_press(UiMessage::Networks(Msg::Menu(Some(id))))
        });
        column![tooltip(tile, tip, tooltip::Position::Bottom)].push(chip).align_x(Center).into()
    }

    /// Desktop's `MVt` (an `AccountListItemPure`): the logo, the name and the active accounts, and Connect, which a
    /// network that takes no account disables. A network with accounts opens their menu.
    fn network_row(&self, e: &Entry<'_>) -> El<'_> {
        let p = &self.pal;
        let id = e.network.bridge_id;
        let n = e.accounts.len();
        let active = (n > 0).then(|| txt(trn("%d active account", "%d active accounts", n, &[&n]), 13.0).color(p.sub));
        let connect = button(semi(tr("Connect"), 12.0))
            .padding([4, 10])
            .style(btn_style([p.chip, p.hover, p.selected], [p.strong; 3], 999.0))
            .on_press_maybe((!e.blocked()).then_some(UiMessage::Networks(Msg::Connect(id))));
        let line = row![
            container(svg(brand_svg(id)).width(28).height(28)).center(46),
            column![semi(e.network.display_name, 14.0).color(p.strong)].push(active).width(Fill),
            connect,
        ]
        .spacing(8)
        .align_y(Center)
        .padding(Padding::new(4.0).left(8.0).right(16.0));
        let open = (n > 0).then_some(UiMessage::Networks(Msg::Menu(Some(id))));
        button(line).bare(p.strong).on_press_maybe(open).into()
    }

    /// Desktop's `NetworkAccountsMenu` (`y7t`) at the click that opened it: the accounts of the network, then "Add
    /// another account", which a network that takes no account disables.
    pub(crate) fn view_network_menu(&self) -> Option<El<'_>> {
        let (network, at) = self.networks.menu?;
        let p = &self.pal;
        let st = self.store.read();
        let entry = entries(&st).into_iter().find(|e| e.network.bridge_id == network)?;
        let accessory = |el: El<'static>| container(el).center(16);
        let rows = entry.accounts.iter().map(|a| {
            let r = a.remote_profile.as_ref();
            let name = r.and_then(|r| r.name.clone());
            let id = identifier(a);
            let face = self.face(r.and_then(|r| r.avatar.as_deref()), name.as_deref().unwrap_or(""), 12.0, 0.0);
            let shown = name.clone().or(id.clone()).unwrap_or_else(|| a.account_id.clone());
            let text = column![txt(shown, 13.0).wrapping(iced::widget::text::Wrapping::None)]
                .push(name.and(id).map(|i| txt(i, 12.0).color(p.sub)));
            let line = row![accessory(face), text].spacing(6);
            menu_line(p, line, Some(UiMessage::Networks(Msg::Account(a.account_id.clone()))))
        });
        let add = row![accessory(ic(icon::PLUS, 10.0).into()), txt(tr("Add another account"), 13.0)].spacing(6);
        let add = menu_line(p, add, (!entry.blocked()).then_some(UiMessage::Networks(Msg::Connect(network))));
        let items = Column::with_children(rows).push(menu_rule(p)).push(add);
        let menu = crate::menu_card(p, items, MENU_W);
        let close = UiMessage::Networks(Msg::Menu(None));
        let layer = container(mouse_area(menu).on_press(UiMessage::Noop))
            .padding(Padding::ZERO.left(at.x).top(at.y))
            .width(Fill)
            .height(Fill);
        Some(mouse_area(layer).on_press(close.clone()).on_right_press(close).into())
    }

    /// Desktop's `fVt`: the accounts that the plan still allows, and "Upgrade to %s for more" with a plan above
    /// this one. A plan without a limit shows nothing.
    pub(crate) fn view_accounts_footer(&self, st: &BeeperStore) -> Option<El<'_>> {
        let p = &self.pal;
        let f = &st.features;
        if f.unlimited_accounts() {
            return None;
        }
        let n = f.accounts_remaining();
        let text = match st.whoami.plan {
            Plan::Free => trn("%d free account remaining.", "%d free accounts remaining.", n, &[&n]),
            _ => trn("%d account remaining.", "%d accounts remaining.", n, &[&n]),
        };
        let upgrade = upgrade_plan(st.whoami.plan).map(|plan| {
            let open = UiMessage::Settings(settings::Msg::Open(SettingsTab::BeeperPlus));
            button(txt(trf("Upgrade to %s for more", &[&plan]), 12.0).color(p.accent)).bare(p.accent).on_press(open)
        });
        // `flex-wrap: wrap` and `justify-content: center`: the upgrade goes under the count when both do not fit.
        let foot = row![txt(text, 12.0).color(p.muted)].push(upgrade).spacing(4).align_y(Center).wrap().align_x(Center);
        Some(container(foot).center_x(Fill).padding(Padding::new(8.0).bottom(2.0)).into())
    }

    /// Desktop's `AddAccountsModal`: "Add an Account", the search and the networks on a card, and the footer.
    pub(crate) fn view_add_accounts(&self) -> El<'_> {
        let p = &self.pal;
        let st = self.store.read();
        let head = container(semi(tr("Add an Account"), 13.0).color(p.strong)).center_x(Fill).padding([8, 0]);
        let search = search_field(p, &self.networks.query, |q| UiMessage::Networks(Msg::Query(q)));
        let rule = container(Space::new()).width(Fill).height(1).style(fill_box(p.chip, 0.0));
        let list = scrollable(self.view_networks(&st, &self.networks.query)).height(Fill).style(scroll_style(p.scroll));
        let card = container(column![search, rule, list]).height(Fill).style(fill_box(p.card, 12.0));
        let page = column![head, card].push(self.view_accounts_footer(&st)).padding(8);
        drop(st);
        stack![container(page).width(Fill).height(Fill).style(fill_box(p.chip, 0.0))]
            .push(self.view_network_menu())
            .into()
    }
}

/// Desktop's `upgradePlan`: Beeper Plus without a subscription, Beeper Plus Plus with Beeper Plus.
pub(crate) fn upgrade_plan(plan: Plan) -> Option<&'static str> {
    match plan {
        Plan::Free => Some("Beeper Plus"),
        Plan::Plus => Some("Beeper Plus Plus"),
        Plan::PlusPlus => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced_test::simulator::simulator;
    use serde_json::json;

    /// A Beeper Cloud account of `bridge` whose user is `name` (`@ada`).
    fn account(st: &BeeperStore, bridge: &str, name: &str) -> BridgeAccountInfo {
        let profile = RemoteProfile { name: Some(name.into()), username: Some("ada".into()), ..Default::default() };
        BridgeAccountInfo {
            account_id: bridge.into(),
            bridge_id: bridge.into(),
            login_id: format!("{bridge}-login"),
            remote_name: None,
            remote_profile: Some(profile),
            ..st.bridge_accounts[0].clone()
        }
    }

    /// A free account with Ada on Signal, whose bridge takes more accounts, and on WhatsApp, whose bridge does not,
    /// and a plan that allows one account on Signal, Telegram and WhatsApp.
    fn planned(name: &str) -> BeeperGuiApp {
        let app = crate::fuzz::demo_app(name);
        let mut st = app.store.write();
        st.bridge_accounts = vec![account(&st, "signal", "Ada Lovelace"), account(&st, "whatsapp", "Ada")];
        st.features.flags["features"]["*"]["bridge:signal:multi-account"] = true.into();
        st.features.flags["features"]["*"]["bridge:local-bluesky"] = true.into();
        st.features.summary = json!({
            "current": { "num_accounts_against_limit": 2, "bridges": { "signal": 1, "whatsapp": 1 } },
            "max": { "num_accounts_against_limit": 10, "bridges": { "signal": 1, "telegram": 1, "whatsapp": 1 } },
        });
        drop(st);
        app
    }

    fn tip(app: &BeeperGuiApp, network: &str) -> (String, bool) {
        let st = app.store.read();
        let e = entries(&st).into_iter().find(|e| e.network.bridge_id == network).expect("an addable network");
        (e.tip(), e.blocked())
    }

    /// Desktop's `addableNetworks` and `AVt` tooltips: a network connects, or upgrades when the plan caps it on the
    /// network or in total, and is gray when its bridge takes no other account or beeper-rs adds none on it.
    #[test]
    fn tiles_follow_the_plan_like_desktop() {
        let app = planned("network-plan");
        let shown = |n| tip(&app, n);
        assert_eq!(shown("signal"), ("Upgrade to connect Signal".into(), false), "the plan caps Signal");
        assert_eq!(shown("telegram"), ("Connect Telegram".into(), false));
        assert_eq!(shown("discord"), ("Upgrade to connect Discord".into(), false), "a free plan without a limit");
        assert_eq!(shown("whatsapp"), ("You can’t add another WhatsApp account".into(), true));
        assert_eq!(shown("bluesky"), ("Can't add Bluesky".into(), true));
        app.store.write().features.summary["current"]["num_accounts_against_limit"] = json!(10);
        assert_eq!(tip(&app, "telegram"), ("Upgrade to connect Telegram".into(), false), "no accounts remain");
        app.store.write().features.summary["max"]["bridges"]["signal"] = json!(2);
        app.store.write().features.summary["current"]["num_accounts_against_limit"] = json!(2);
        assert_eq!(tip(&app, "signal"), ("Add another".into(), false));
    }

    /// Desktop's `addablePlatforms` in its order (`Pn`): the networks whose bridge flag is on, then the others.
    #[test]
    fn the_flags_pick_the_networks_like_desktop() {
        let mut f = Flags::free_plan();
        let names = |f: &Flags| addable(f).map(|n| n.display_name).collect::<Vec<_>>().join(", ");
        let imessage = if cfg!(target_os = "macos") { "iMessage, " } else { "" };
        let all = format!(
            "WhatsApp, X, Telegram, Slack, Discord, Facebook, Instagram, LinkedIn, Google Messages, Signal, Google Chat, \
             LINE, Google Voice, Tumblr, Viber, Snapchat, GroupMe, WeChat, Reddit, ChatGPT Dots, {imessage}Grok Bot, Muse"
        );
        assert_eq!(names(&f), all, "Bluesky has no flag on the free plan");
        f.flags["features"]["*"]["bridge:groupme"] = false.into();
        f.flags["features"]["appType:desktop"] = json!({ "bridge:slackgo": false, "bridge:local-slack": true });
        assert!(!names(&f).contains("GroupMe"), "a network without its bridge flag");
        assert!(names(&f).contains("Slack"), "a network on its on-device bridge");
        assert!(names(&Flags::default()).contains("Bluesky"), "every network before the flags come");
    }

    /// Desktop's tile presses: a network that takes an account connects, a gray one with accounts opens their menu
    /// (`NetworkAccountsMenu`), whose account opens its settings, and a gray one without accounts is disabled.
    #[test]
    fn tiles_connect_or_open_the_accounts_menu_like_desktop() {
        let mut app = planned("network-menu");
        let _ = app.update(UiMessage::Networks(Msg::Open));
        assert!(app.networks.window.is_some(), "Add an Account opens its own window");
        let press = |app: &BeeperGuiApp, label: &str| {
            let mut sim = simulator(app.view_add_accounts());
            let _ = sim.click(label).expect("a network");
            sim.into_messages().find_map(|m| match m {
                UiMessage::Networks(Msg::Connect(n)) => Some(format!("connect {n}")),
                UiMessage::Networks(Msg::Menu(Some(n))) => Some(format!("menu {n}")),
                _ => None,
            })
        };
        assert_eq!(press(&app, "Telegram").as_deref(), Some("connect telegram"));
        assert_eq!(press(&app, "WhatsApp").as_deref(), Some("menu whatsapp"));
        assert_eq!(press(&app, "Bluesky"), None, "a gray tile without accounts is disabled");
        let _ = app.update(UiMessage::Networks(Msg::Menu(Some("signal"))));
        let mut sim = simulator(app.view_add_accounts());
        for text in ["Ada Lovelace", "@ada", "Add another account"] {
            assert!(sim.find(text).is_ok(), "the menu shows {text}");
        }
        let _ = sim.click("Ada Lovelace").expect("the account");
        let open = sim.into_messages().find(|m| matches!(m, UiMessage::Networks(Msg::Account(_))));
        let _ = app.update(open.expect("the account opens"));
        assert_eq!(app.account_page.as_deref(), Some("signal"), "the account's settings");
        assert!(app.networks.menu.is_none());
        let _ = app.update(UiMessage::Networks(Msg::Menu(Some("signal"))));
        let id = app.networks.window.expect("the window");
        let _ = app.update(UiMessage::WindowKey(id, Key::Named(Named::Escape), Modifiers::empty()));
        assert!(app.networks.menu.is_none(), "Escape closes the menu first");
    }

    /// Desktop's `wj`: Connect opens the Add Account window of the bridge, and an upgradable network the upgrade.
    #[test]
    fn connect_adds_an_account_or_upgrades_like_desktop() {
        let mut app = planned("network-connect");
        app.store.write().config.demo = false;
        let _ = app.update(UiMessage::Networks(Msg::Connect("telegram")));
        assert_eq!(app.add_accounts.len(), 1, "Telegram's Add Account window opens");
        let _ = app.update(UiMessage::Networks(Msg::Connect("signal")));
        assert_eq!((app.add_accounts.len(), app.active_settings_tab), (1, SettingsTab::BeeperPlus));
    }

    /// Desktop's `DVt` with a search: the rows of the matching networks with their active accounts and Connect,
    /// which a gray network disables.
    #[test]
    fn a_search_lists_the_networks_like_desktop() {
        let mut app = planned("network-search");
        let _ = app.update(UiMessage::Networks(Msg::Query("sig".into())));
        let mut sim = simulator(app.view_add_accounts());
        for text in ["Signal", "1 active account", "Connect"] {
            assert!(sim.find(text).is_ok(), "the row shows {text}");
        }
        assert!(sim.find("WhatsApp").is_err(), "the search filters the networks");
        let _ = sim.click("Connect").expect("Connect");
        let connect = sim.into_messages().any(|m| matches!(m, UiMessage::Networks(Msg::Connect("signal"))));
        assert!(connect, "Connect connects the network");
        let _ = app.update(UiMessage::Networks(Msg::Query("whats".into())));
        let mut sim = simulator(app.view_add_accounts());
        let _ = sim.click("Connect").expect("Connect");
        assert!(sim.into_messages().all(|m| !matches!(m, UiMessage::Networks(Msg::Connect(_)))), "a gray network");
        let _ = app.update(UiMessage::Networks(Msg::Query("zzz".into())));
        assert!(simulator(app.view_add_accounts()).find("No networks found").is_ok());
    }

    /// Desktop's `fVt`: the free or paid accounts that remain from the summary, the upgrade to the next plan, and
    /// nothing without a limit.
    #[test]
    fn the_footer_follows_the_plan_like_desktop() {
        let app = planned("network-footer");
        let finds = |texts: &[&str]| {
            let mut sim = simulator(app.view_add_accounts());
            texts.iter().all(|t| sim.find(*t).is_ok())
        };
        assert!(finds(&["8 free accounts remaining.", "Upgrade to Beeper Plus for more"]));
        app.store.write().whoami.plan = Plan::Plus;
        assert!(finds(&["8 accounts remaining.", "Upgrade to Beeper Plus Plus for more"]));
        app.store.write().whoami.plan = Plan::PlusPlus;
        assert!(finds(&["8 accounts remaining."]) && !finds(&["Upgrade to Beeper Plus Plus for more"]));
        app.store.write().features.summary["max"]["num_accounts_against_limit"] = json!(-1);
        assert!(!finds(&["accounts remaining."]), "an unlimited plan shows no footer");
    }

    /// Desktop's footer wraps (`flex-wrap: wrap`): in the 330 px sign-in window the upgrade goes whole under the
    /// count, and the footer takes the room of both lines.
    #[test]
    fn the_footer_wraps_in_the_sign_in_window_like_desktop() {
        let mut app = planned("network-footer-wrap");
        app.login.screen = crate::login::Screen::Accounts;
        let size = Size::new(330.0, 620.0);
        let mut sim = iced_test::Simulator::with_size(Default::default(), size, app.view_login());
        let count = sim.find("8 free accounts remaining.").expect("the count").bounds();
        let upgrade = sim.find("Upgrade to Beeper Plus for more").expect("the upgrade").bounds();
        assert!(upgrade.y >= count.y + count.height, "the upgrade goes under the count: {count:?} {upgrade:?}");
        assert!(upgrade.height < 20.0, "on one line: {upgrade:?}");
        assert!(upgrade.y + upgrade.height <= size.height, "inside the window: {upgrade:?}");
    }
}
