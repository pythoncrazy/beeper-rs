//! Desktop's `LoginBrowserWindow` with the `BridgeV2AuthComponent` of a Beeper Cloud bridge: it adds an account of
//! a network, or logs an account in again. The bridge describes each step (see `beeper_core::bridge_login`).
use crate::bridge_texts::{bridge_error, bridge_text};
use crate::i18n::{tr, trf, trx};
use crate::phone::{self, KeyOutcome, Phone, PhoneMsg};
use crate::prelude::*;
use beeper_core::bridge_login::{self, Field, Flow, Step};
use beeper_core::browser_login::{self, BrowserLogin, Outcome};
use beeper_core::matrix::Provision;
use iced::widget::column;
use iced::widget::operation::{self, AbsoluteOffset};
use iced::window;

/// Desktop's `LoginBrowserWindow` size (`Ms` and `Bs` of sentry-common).
pub const SIZE: Size = Size::new(400.0, 560.0);
const QR_SIZE: f32 = 126.0;
const LEARN_MORE: &str = "https://help.beeper.com/using-on-device-chat-network-connections-in-beeper";
/// Desktop's `preferredLoginMode` of the platforms that have one.
const PREFERRED: [(&str, &str); 2] = [("telegram", "qr"), ("discordgo", "machine")];
/// Desktop's `LoginMethodMenu`: the flows that the menu offers for some bridge types, and the labels of some flows.
const MENU_FLOWS: [(&str, &[&str]); 4] = [
    ("facebookgo", &["messenger-lite", "messenger"]),
    ("slackgo", &["email", "token"]),
    ("discordgo", &["token", "qr"]),
    ("telegram", &["phone", "qr"]),
];
const FLOW_LABELS: [(&str, &str); 9] = [
    ("facebookgo/messenger", "Login with Browser"),
    ("twitter/cookies", "Login with Browser"),
    ("slackgo/token", "Login with Browser"),
    ("discordgo/token", "Login with Browser"),
    ("gvoice/cookies", "Login with Browser"),
    ("linkedin/cookies", "Login with Browser"),
    ("facebookgo/messenger-lite", "Login with Password"),
    ("signal/qr", "QR Code"),
    ("whatsapp/qr", "QR Code"),
];
/// Desktop's `ConnectionPicker` limitations of the Beeper Cloud connection of a network.
const CLOUD_LIMITS: [(&str, &str); 11] = [
    ("whatsapp", "Online status"),
    ("facebook", "Online status and polls"),
    ("instagram", "Disappearing messages, online status, and polls"),
    ("telegram", "Online status"),
    ("googlechat", "Start a new chat, disappearing messages, online status, and polls"),
    ("twitter", "Disappearing messages"),
    ("slack", "Online status"),
    ("discord", "Start a new chat, online status, and polls"),
    ("linkedin", "Online status"),
    ("line", "Edit messages, typing indicators, and polls"),
    ("tumblr", "Start a new chat and online status"),
];
/// The step whose instructions Desktop shows with a Done button when the login completes (`Cpe`).
const DONE_STEP: &str = "fi.mau.viber.login.complete";
/// The text when no browser can open a cookie login. Desktop opens its own window, so it has no such text.
const NO_BROWSER: &str = "To log in to %s, install Chrome, Chromium, Brave, Edge or Vivaldi.";

/// A 24px stroke icon like the ones of Desktop's icon set.
fn stroke(paths: &str) -> svg::Handle {
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">{paths}</svg>"#
    );
    svg::Handle::from_memory(svg.into_bytes())
}

static COPY: LazyLock<svg::Handle> = LazyLock::new(|| {
    stroke(r#"<path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/><rect x="9" y="9" width="13" height="13" rx="2"/>"#)
});
static EYE: LazyLock<svg::Handle> =
    LazyLock::new(|| stroke(r#"<path d="M2 12s3.6-7 10-7 10 7 10 7-3.6 7-10 7S2 12 2 12z"/><circle cx="12" cy="12" r="3"/>"#));
static EYE_OFF: LazyLock<svg::Handle> = LazyLock::new(|| {
    stroke(concat!(
        r#"<path d="M9.9 4.2A10 10 0 0 1 12 4c6.4 0 10 8 10 8a17 17 0 0 1-2.2 3.2M6.6 6.6A17 17 0 0 0 2 12s3.6 8 10 8"#,
        r#"a9.7 9.7 0 0 0 5.4-1.6"/><path d="M2 2l20 20"/><path d="M14.1 14.1a3 3 0 1 1-4.2-4.2"/>"#
    ))
});
static CLEAR_FIELD: LazyLock<svg::Handle> = LazyLock::new(|| stroke(r#"<path d="M18 6 6 18M6 6l12 12"/>"#));
/// Desktop's `ExternalLinkIcon` of the "Open %s Login" button.
static EXTERNAL: LazyLock<svg::Handle> = LazyLock::new(|| {
    let path =
        r#"<path d="M1 7L7 1M7 1V5.5M7 1H2.5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/>"#;
    let svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 8 8" fill="none">{path}</svg>"#);
    svg::Handle::from_memory(svg.into_bytes())
});
/// Desktop's `og`: a domain name.
static DOMAIN: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?i)^(?:[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z0-9][a-z0-9-]{0,61}[a-z0-9]$")
        .expect("the domain pattern compiles")
});

/// An Add Account window.
pub struct AddAccount {
    pub id: window::Id,
    network: &'static NetworkSpec,
    bridge: &'static str,
    /// The login that the window logs in again, or empty for a new account.
    login: String,
    /// Desktop's `userDisplayText` of the account that the window logs in again.
    who: Option<String>,
    flows: Vec<Flow>,
    flow: String,
    process: Option<String>,
    step: Option<Step>,
    /// Desktop's `loginState.error`: the step failed, and the window offers Start Over.
    error: Option<String>,
    /// Desktop's `prefsViewStore.errorMessage`: a sent form failed, and the window shows only the error.
    failed: Option<String>,
    busy: bool,
    menu: bool,
    values: BTreeMap<String, String>,
    /// The fields that show their validation error: the changed ones, or all of them after a submit.
    touched: BTreeSet<String>,
    /// The state of each phone field.
    phones: BTreeMap<String, Phone>,
    shown: BTreeSet<String>,
    qr: Option<svg::Handle>,
    emoji: Option<image::Handle>,
    /// Counts the login processes, so that the replies of an earlier process are dropped.
    run: u32,
    /// The device tab of Desktop's login steps: iPhone (`false`) or Android (`true`).
    android: bool,
    /// Desktop's `PlatformRemoteStatus`: the message of the platform's entry of `/platform-status`.
    status: Option<String>,
    /// Desktop's `remoteState`: the status hides the login (`disabled`), or hides it until Continue (`confirm`).
    remote: Remote,
    /// The browser window of a `cookies` step.
    browser: Browser,
}

/// The browser window of a `cookies` step.
#[derive(Clone, PartialEq)]
enum Browser {
    Closed,
    Open,
    /// No browser could open, for this reason.
    Failed(String),
}

/// Desktop's `remoteState` of the Add Account window.
#[derive(Clone, Copy, Default, PartialEq)]
enum Remote {
    #[default]
    Open,
    Disabled,
    Confirm,
}

#[derive(Clone)]
pub enum AddMsg {
    Flows(Result<Vec<Flow>, String>),
    Started(u32, Result<(String, Step), String>),
    Next(u32, Result<Step, String>),
    Emoji(u32, Result<Vec<u8>, String>),
    Field(String, String),
    Reveal(String),
    Submit,
    Back,
    StartOver,
    Flow(String),
    Menu,
    Copy(String),
    /// The iPhone (`false`) or Android (`true`) tab of the login steps.
    Device(bool),
    Close,
    /// The reply of Desktop's `/platform-status`.
    Status(Result<serde_json::Value, String>),
    /// Desktop's "Continue →" of a platform status that asks to confirm.
    Continue,
    Phone(String, PhoneMsg),
    /// A key or typed text in the window, for an open dropdown of a phone field.
    Key(iced::keyboard::Key),
    Text(String),
    /// Desktop's "Open %s Login" of a `cookies` step.
    OpenBrowser,
    /// The fields that the browser window read when it closed.
    Browser(u32, Result<Outcome, String>),
}

/// Desktop's `di`: the validation error of a field value.
fn invalid(f: &Field, v: &str) -> Option<String> {
    if v.trim().is_empty() {
        return Some(tr("This field is required").into());
    }
    let bad = match f.kind.as_str() {
        "phone_number" => (!phone::is_valid(v)).then_some("Please enter a valid phone number"),
        "domain" => (!DOMAIN.is_match(v)).then_some("Please enter a valid domain (e.g., example.com)"),
        "email" => {
            (!v.split_once('@').is_some_and(|(a, b)| !a.is_empty() && b.contains('.'))).then_some("Please enter a valid email address")
        }
        "url" => (!(v.starts_with("https://") || v.starts_with("http://"))).then_some("Please enter a valid URL"),
        "select" => return None,
        _ => None,
    };
    if let Some(e) = bad {
        return Some(tr(e).into());
    }
    let n = v.chars().count();
    let (min, max) = (f.min_length, f.max_length);
    let chars = |one, many, k: usize| crate::i18n::trn(one, many, k, &[&k]);
    match () {
        _ if min > 0 && min == max && n != min => {
            return Some(chars("Must be exactly %d character", "Must be exactly %d characters", min));
        }
        _ if min > 0 && n < min => return Some(chars("Must be at least %d character", "Must be at least %d characters", min)),
        _ if max > 0 && n > max => return Some(chars("Must be at most %d character", "Must be at most %d characters", max)),
        _ => {}
    }
    // Desktop ignores a pattern that does not compile.
    let pattern = regex::Regex::new(&f.pattern).ok().filter(|_| !f.pattern.is_empty());
    pattern.filter(|p| !p.is_match(v)).map(|_| tr("Invalid format").into())
}

/// Desktop's `AuthButton` with its `ExternalLinkIcon`: a 250 px blue pill whose filter brightens it on hover and
/// press.
fn auth_button<'a>(label: String, msg: UiMessage) -> El<'a> {
    let blue = crate::hex(0x0d4ffb);
    let lit = |k: f32| Color { r: (blue.r * k).min(1.0), g: (blue.g * k).min(1.0), b: (blue.b * k).min(1.0), a: 1.0 };
    let icon = svg(EXTERNAL.clone()).width(10).height(10).style(|_, _| svg::Style { color: Some(Color::WHITE) });
    // `gap: 1ch` of 14 px text.
    let label = row![med(label, 14.0), icon].spacing(7).align_y(Center);
    let button = button(container(label).center_x(Fill))
        .width(Fill)
        .padding([10, 12])
        .style(btn_style([blue, lit(1.025), lit(1.085)], [Color::WHITE; 3], 999.0))
        .on_press(msg);
    container(button).max_width(250).into()
}

/// The icons in Desktop's WeChat and Viber steps: its `add` and `scan` glyphs, and its `Dots` and `QrCode` icons.
static STEP_ICONS: LazyLock<[svg::Handle; 4]> = LazyLock::new(|| {
    [
        ("0 0 72 72", "M34.2 34.2V21h3.6v13.2H51v3.6H37.8V51h-3.6V37.8H21v-3.6zM36 66C19.431 66 6 52.569 6 36S19.431 6 36 6s30 13.431 30 30-13.431 30-30 30m0-3.6c14.58 0 26.4-11.82 26.4-26.4S50.58 9.6 36 9.6 9.6 21.42 9.6 36 21.42 62.4 36 62.4"),
        ("0 0 96 96", "M71.47 79h-4.495c-4.326 0-9.105-1.616-14.298-14.036H40.99c-1.27 0-1.990-1.677-1.990-2.946v-2.866c0-1.262.725-2.212 1.990-2.212H69.2c2.366 0 3.780-2.127 3.780-4.828 0-.425-.062-.83-.15-1.223h.15v1.207c.095-.886.07-1.470-.371-3.008-.632-2.204 1.785-8.563 6.284-9.070 1.125-.128 2.493.423 2.300 1.979C79.760 53.490 84 55.655 84 67.990 84 73.724 79.570 79 71.470 79M28.810 39c-.2.037-4.864 2.190-4.864 4.888 0 .424.062.83.150 1.222h-.150v-1.205c-.094.885-.070 1.470.370 3.009.627 2.201-1.774 8.560-6.243 9.068-1.118.127-2.476-.423-2.284-1.978C17.210 42.509 13 40.344 13 28.009 13 22.276 17.401 17 25.447 17h4.464c4.299 0 9.234 2.531 14.392 14.951h11.423c1.260 0 2.274.744 2.274 2.012v2.750a2.280 2.280 0 0 1-2.274 2.286z"),
        ("0 0 181 38", "M19 38c10.493 0 19-8.507 19-19S29.493 0 19 0 0 8.507 0 19s8.507 19 19 19M90.25 38c10.493 0 19-8.507 19-19s-8.507-19-19-19-19 8.507-19 19 8.507 19 19 19M161.5 38c10.493 0 19-8.507 19-19s-8.507-19-19-19-19 8.507-19 19 8.507 19 19 19"),
        ("0 0 20 20", "M3.333 9.167V2.5H10v6.667zM5 7.5h3.333V4.167H5zm-1.667 10v-6.667H10V17.5zM5 15.833h3.333V12.5H5zm6.667-6.666V2.5h6.666v6.667zM13.333 7.5h3.334V4.167h-3.334zm3.334 10v-1.667h1.666V17.5zm-5-5v-1.667h1.666V12.5zm1.666 1.667V12.5H15v1.667zm-1.666 1.666v-1.666h1.666v1.666zm1.666 1.667v-1.667H15V17.5zM15 15.833v-1.666h1.667v1.666zm0-3.333v-1.667h1.667V12.5zm1.667 1.667V12.5h1.666v1.667z"),
    ]
    .map(|(view, d)| {
        let svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="{view}"><path fill-rule="evenodd" d="{d}"/></svg>"#);
        svg::Handle::from_memory(svg.into_bytes())
    })
});

/// A line of Desktop's `.login-steps`: 13 px text on an 18 px line.
fn step_line<'a>(p: &Palette, t: String) -> iced::widget::Text<'a> {
    txt(t, 13.0).line_height(iced::Pixels(rem(18.0))).color(p.strong)
}

/// A step whose text has Desktop's `zs` tags, each in place of its icon.
fn inline<'a>(p: &Palette, text: &str, icons: Vec<(&str, El<'a>)>) -> El<'a> {
    let mut parts = Vec::new();
    let mut rest = text;
    for (tag, icon) in icons {
        let (before, after) = rest.split_once(tag).unwrap_or((rest, ""));
        parts.push(Some(before.trim()).filter(|t| !t.is_empty()).map(|t| step_line(p, t.into()).into()));
        parts.push(Some(icon));
        rest = after;
    }
    parts.push(Some(rest.trim()).filter(|t| !t.is_empty()).map(|t| step_line(p, t.into()).into()));
    row(parts.into_iter().flatten()).spacing(4).align_y(Center).wrap().into()
}

/// The QR code of a `display_and_wait` step as an SVG, like Desktop's `QRCodeSVG` with level `M`.
fn qr_svg(data: &str) -> svg::Handle {
    let code = qrcodegen::QrCode::encode_text(data, qrcodegen::QrCodeEcc::Medium).expect("the QR data fits in a QR code");
    crate::style::qr_svg(&code, 0)
}

impl AddAccount {
    /// The flows of Desktop's "Login options" menu: the other flows, for some bridges only some of them.
    fn other_flows(&self) -> impl Iterator<Item = &Flow> {
        let allowed = MENU_FLOWS.iter().find(|m| m.0 == self.bridge).map(|m| m.1);
        self.flows.iter().filter(move |f| f.id != self.flow && allowed.is_none_or(|a| a.contains(&f.id.as_str())))
    }
}

impl BeeperGuiApp {
    /// The bridge login that the Add Account window `i` acts as.
    fn add_provision(&self, i: usize) -> Provision {
        let (st, a) = (self.store.read(), &self.add_accounts[i]);
        Provision {
            homeserver: st.beeper_state.homeserver.clone(),
            access_token: st.beeper_state.access_token.clone(),
            user_id: st.beeper_state.user_id.clone(),
            bridge_id: a.bridge.into(),
            login_id: a.login.clone(),
        }
    }

    /// Desktop's `openNewAccountInNewWindow`, or `reauthenticate` for the account `login` of the network.
    pub fn open_add_account(&mut self, network: &str, login: Option<String>) -> Task<UiMessage> {
        let network = find_network(network).expect("a known network");
        let bridge = cloud_bridge(network).expect("a network with a Beeper Cloud bridge");
        let who = login.as_ref().and_then(|l| {
            let st = self.store.read();
            // Desktop's `userDisplayText` in "Reconnecting %s": the nickname, or the user.
            let a = st.bridge_accounts.iter().find(|a| a.bridge_id == bridge && a.login_id == *l)?;
            st.custom_name(&a.account_id).map(str::to_string).or_else(|| a.display_text())
        });
        let (id, open) = self.open_window(window::Settings {
            resizable: false,
            min_size: None,
            position: window::Position::Centered,
            ..Self::window_settings(SIZE)
        });
        self.add_accounts.push(beeper_core::with_defaults!(AddAccount {
            id,
            network,
            bridge,
            login: login.unwrap_or_default(),
            who,
            busy: true,
            remote: Remote::Open,
            browser: Browser::Closed,
        }, default: flows, flow, process, step, error, failed, menu, values, touched, phones, shown, qr, emoji, run,
            android, status));
        let p = self.add_provision(self.add_accounts.len() - 1);
        let status = beeper_core::cloud::platform_status(self.cloud.app_api.clone(), p.access_token.clone());
        let status = Task::perform(status, move |r| UiMessage::AddAccount(id, AddMsg::Status(r)));
        let flows = bridge_login::flows(p, self.cloud.api_url.clone());
        let flows = Task::perform(flows, move |r| UiMessage::AddAccount(id, AddMsg::Flows(r)));
        Task::batch([open.discard(), flows, status])
    }

    /// Desktop's `disposeIfNotLoggedIn`: a closed window cancels its login process.
    pub fn add_account_closed(&mut self, id: window::Id) -> Task<UiMessage> {
        let i = self.add_accounts.iter().position(|a| a.id == id).expect("an Add Account window");
        let p = self.add_provision(i);
        match self.add_accounts.remove(i).process {
            Some(process) => Task::perform(bridge_login::cancel(p, process), |_| UiMessage::Noop),
            None => Task::none(),
        }
    }

    /// Desktop's `startLoginProcess`: cancels the running process and starts the flow.
    fn start_flow(&mut self, i: usize, flow: String) -> Task<UiMessage> {
        let p = self.add_provision(i);
        let cancel =
            self.add_accounts[i].process.take().map(|process| Task::perform(bridge_login::cancel(p.clone(), process), |_| UiMessage::Noop));
        let a = &mut self.add_accounts[i];
        a.run += 1;
        (a.flow, a.step, a.error, a.failed, a.busy, a.menu, a.qr, a.emoji, a.browser) =
            (flow.clone(), None, None, None, true, false, None, None, Browser::Closed);
        let (id, run) = (a.id, a.run);
        let start = Task::perform(bridge_login::start(p, flow), move |r| UiMessage::AddAccount(id, AddMsg::Started(run, r)));
        Task::batch(cancel.into_iter().chain([start]))
    }

    /// Desktop's `setLoginProcessStep`: shows the step, and waits for a `display_and_wait` step to end.
    fn show_step(&mut self, i: usize, step: Step) -> Task<UiMessage> {
        let p = self.add_provision(i);
        let a = &mut self.add_accounts[i];
        let (id, run) = (a.id, a.run);
        a.busy = false;
        if step.kind == "complete" {
            let reconnect = !a.login.is_empty();
            let text = if reconnect { "Reconnected to %s" } else { "Connected to %s" };
            let toast = Toast::new(trf(text, &[&a.network.display_name]));
            let show = step.step_id == DONE_STEP && !step.instructions.is_empty();
            (a.process, a.qr) = (None, None);
            a.step = Some(step);
            self.show(toast);
            let refresh = self.refresh_cloud_accounts();
            return if show { refresh } else { Task::batch([refresh, self.close_window(id)]) };
        }
        a.values = step.user_input.iter().flat_map(|u| &u.fields).map(|f| (f.id.clone(), f.default_value.clone())).collect();
        // Desktop keys the form by its field IDs, so the next form with the same fields keeps its phone fields: their
        // country stays, and the reset of the form's values empties their number to the dial code.
        let ids = |s: &Step| s.user_input.iter().flat_map(|u| &u.fields).map(|f| f.id.clone()).collect::<Vec<_>>();
        let same = a.step.as_ref().is_some_and(|old| old.user_input.is_some() && ids(old) == ids(&step));
        let old = std::mem::take(&mut a.phones);
        for f in step.user_input.iter().flat_map(|u| &u.fields).filter(|f| f.kind == "phone_number") {
            let kept = old.get(&f.id).filter(|_| same).map(|p| p.country);
            let (field, value) = Phone::new(&f.default_value, kept.unwrap_or_else(phone::default_country));
            let value = if kept.is_some() { field.phone().to_string() } else { value };
            a.phones.insert(f.id.clone(), field);
            a.values.insert(f.id.clone(), value);
        }
        (a.touched, a.shown) = (BTreeSet::new(), BTreeSet::new());
        let display = step.display_and_wait.clone();
        a.qr = display.as_ref().filter(|d| d.kind == "qr").map(|d| qr_svg(&d.data));
        a.emoji = None;
        let process = a.process.clone().expect("a running login process");
        a.step = Some(step.clone());
        // Desktop's `BrowserCookieLogin` clicks its one "Open %s Login" button at once.
        if step.kind == "cookies" {
            return self.open_browser(i);
        }
        let Some(d) = display else { return Task::none() };
        let wait = Task::perform(bridge_login::submit(p, process, step, serde_json::Value::Null), move |r| {
            UiMessage::AddAccount(id, AddMsg::Next(run, r))
        });
        let emoji = (d.kind == "emoji").then(|| {
            Task::perform(async move { beeper_core::cloud::http_bytes(&d.image_url).await }, move |r| {
                UiMessage::AddAccount(id, AddMsg::Emoji(run, r))
            })
        });
        Task::batch([wait].into_iter().chain(emoji))
    }

    /// Desktop's `launchBrowserWindow` for the `cookies` step: the browser window of the login page, which reads
    /// the fields of the step.
    fn open_browser(&mut self, i: usize) -> Task<UiMessage> {
        let find = self.find_browser;
        let a = &mut self.add_accounts[i];
        let cookies = a.step.as_ref().and_then(|s| s.cookies.as_ref()).expect("a cookies step");
        let login = BrowserLogin::new(cookies, a.network.bridge_id, &a.flow);
        let Some(browser) = find() else {
            a.browser = Browser::Failed(NO_BROWSER.replace("%s", a.network.display_name));
            return Task::none();
        };
        a.browser = Browser::Open;
        let (id, run) = (a.id, a.run);
        Task::perform(browser_login::log_in(browser, login), move |r| UiMessage::AddAccount(id, AddMsg::Browser(run, r)))
    }

    pub fn update_add_account(&mut self, id: window::Id, msg: AddMsg) -> Task<UiMessage> {
        // A reply can come after its window closed.
        let Some(i) = self.add_accounts.iter().position(|a| a.id == id) else { return Task::none() };
        let p = self.add_provision(i);
        let run = self.add_accounts[i].run;
        let next = move |r| UiMessage::AddAccount(id, AddMsg::Next(run, r));
        if let AddMsg::Flows(Ok(flows)) = msg {
            let bridge = self.add_accounts[i].bridge;
            let flows = self.store.read().features.login_flows(bridge, flows);
            let preferred = PREFERRED.iter().find(|p| p.0 == bridge);
            let preferred = preferred.and_then(|p| flows.iter().find(|f| f.id.contains(p.1)));
            let Some(flow) = preferred.or(flows.first()).map(|f| f.id.clone()) else {
                let a = &mut self.add_accounts[i];
                (a.failed, a.busy) = (Some(tr("Something went wrong").into()), false);
                return Task::none();
            };
            self.add_accounts[i].flows = flows;
            return self.start_flow(i, flow);
        }
        let a = &mut self.add_accounts[i];
        match msg {
            AddMsg::Flows(Ok(_)) => unreachable!("handled above"),
            AddMsg::Started(r, _) | AddMsg::Next(r, _) | AddMsg::Emoji(r, _) | AddMsg::Browser(r, _) if r != run => {}
            AddMsg::Flows(Err(e)) | AddMsg::Started(_, Err(e)) => (a.failed, a.busy) = (Some(e), false),
            AddMsg::Started(_, Ok((process, step))) => {
                a.process = Some(process);
                return self.show_step(i, step);
            }
            // Desktop's `handleLoginError`: a sent form that fails shows only the error, and a failed wait offers
            // Start Over.
            AddMsg::Next(_, Err(e)) => match a.busy && a.step.as_ref().is_some_and(|s| s.display_and_wait.is_none()) {
                true => (a.failed, a.busy) = (Some(e), false),
                false => (a.error, a.busy) = (Some(e), false),
            },
            AddMsg::Next(_, Ok(step)) => return self.show_step(i, step),
            AddMsg::Emoji(_, r) => a.emoji = r.ok().map(image::Handle::from_bytes),
            // A phone field takes a new value in its country, like Desktop's `PhoneInput` with a new `value`.
            AddMsg::Field(f, v) => {
                let v = match a.phones.get_mut(&f) {
                    Some(field) => field.set(&v),
                    None => v,
                };
                a.touched.insert(f.clone());
                a.values.insert(f, v);
            }
            AddMsg::Phone(f, m) => return self.update_phone(i, f, m),
            AddMsg::Key(key) => {
                let Some(f) = a.phones.iter().find(|p| p.1.open).map(|p| p.0.clone()) else { return Task::none() };
                return self.dropdown_key(i, f, |p| p.key(&key));
            }
            AddMsg::Text(t) => {
                let Some(f) = a.phones.iter().find(|p| p.1.open).map(|p| p.0.clone()) else { return Task::none() };
                let now = std::time::Instant::now();
                return self.dropdown_key(i, f, |p| p.typed_letters(&t, now));
            }
            AddMsg::Reveal(f) => {
                if !a.shown.insert(f.clone()) {
                    a.shown.remove(&f);
                }
            }
            AddMsg::Submit => {
                let step = a.step.clone().expect("a form step");
                let fields = &step.user_input.as_ref().expect("a form step").fields;
                a.touched.extend(fields.iter().map(|f| f.id.clone()));
                if a.busy || fields.iter().any(|f| invalid(f, &a.values[&f.id]).is_some()) {
                    return Task::none();
                }
                a.busy = true;
                let values = serde_json::to_value(&a.values).expect("the field values are JSON");
                let process = a.process.clone().expect("a running login process");
                return Task::perform(bridge_login::submit(p, process, step, values), next);
            }
            AddMsg::Back => {
                (a.busy, a.error) = (true, None);
                let step = a.step.as_ref().expect("a step to leave").step_id.clone();
                let process = a.process.clone().expect("a running login process");
                return Task::perform(bridge_login::cancel_step(p, process, step), next);
            }
            AddMsg::StartOver => {
                let flow = a.flow.clone();
                return self.start_flow(i, flow);
            }
            AddMsg::Flow(f) => return self.start_flow(i, f),
            AddMsg::Menu => a.menu ^= true,
            AddMsg::Device(android) => a.android = android,
            AddMsg::Status(Err(e)) => eprintln!("platform status failed: {e}"),
            // Desktop's platform name of a cloud bridge is `bridge-` and the bridge.
            AddMsg::Status(Ok(v)) => {
                let s = &v[format!("bridge-{}", a.bridge)];
                if s["disabled"] == true {
                    a.remote = Remote::Disabled;
                } else if s["confirm"] == true {
                    a.remote = Remote::Confirm;
                }
                a.status = s["message"].as_str().map(str::to_string);
            }
            AddMsg::Continue => a.remote = Remote::Open,
            AddMsg::OpenBrowser => return self.open_browser(i),
            AddMsg::Browser(_, Err(e)) => a.browser = Browser::Failed(e),
            // Desktop's `didFinish`: a window that the user closed first offers the button again.
            AddMsg::Browser(_, Ok(o)) if !o.finished => a.browser = Browser::Closed,
            // Desktop's `handleSubmitCookies` throws without fields, which its `login` shows as an unknown error.
            AddMsg::Browser(_, Ok(o)) if o.fields.is_empty() => {
                (a.browser, a.failed) = (Browser::Closed, Some("An unknown error occurred".into()));
            }
            AddMsg::Browser(_, Ok(o)) => {
                (a.browser, a.busy) = (Browser::Closed, true);
                let step = a.step.clone().expect("a cookies step");
                let process = a.process.clone().expect("a running login process");
                return Task::perform(bridge_login::submit(p, process, step, o.fields.into()), next);
            }
            AddMsg::Copy(code) => return iced::clipboard::write(code),
            AddMsg::Close => return self.close_window(id),
        }
        Task::none()
    }

    /// Whether the window has the open dropdown of a phone field, which takes the window's keys.
    pub fn add_account_dropdown(&self, id: window::Id) -> bool {
        self.add_accounts.iter().any(|a| a.id == id && a.phones.values().any(|p| p.open))
    }

    /// A change of the phone field `f` of the window `i`. Typing moves the cursor to Desktop's place in the
    /// formatted number, a picked country focuses the input, and an opened dropdown shows at its last scroll.
    fn update_phone(&mut self, i: usize, f: String, m: PhoneMsg) -> Task<UiMessage> {
        let a = &mut self.add_accounts[i];
        let (input, list) = (phone::input_id(a.id, &f), phone::list_id(a.id, &f));
        let Some(field) = a.phones.get_mut(&f) else { return Task::none() };
        let (value, task) = match m {
            PhoneMsg::Input(v) => {
                let (value, at) = field.typed(&v);
                (value, operation::move_cursor_to(input, at))
            }
            PhoneMsg::Paste(v) => {
                let (value, at) = field.pasted(&v);
                (value, operation::move_cursor_to(input, at))
            }
            // Desktop shows no error of the focused field that the picked country empties, until a blur or a submit.
            PhoneMsg::Pick(c) => {
                a.values.insert(f.clone(), field.pick(c));
                return operation::focus(input);
            }
            PhoneMsg::Toggle => {
                return field.toggle().map_or_else(Task::none, |y| operation::scroll_to(list, AbsoluteOffset { x: 0.0, y }));
            }
            PhoneMsg::Blur => {
                field.close();
                return Task::none();
            }
            PhoneMsg::Scrolled(y) => {
                field.scrolled(y);
                return Task::none();
            }
        };
        a.touched.insert(f.clone());
        a.values.insert(f, value);
        task
    }

    /// A key of the window for the open dropdown of the phone field `f`.
    fn dropdown_key(&mut self, i: usize, f: String, key: impl FnOnce(&mut Phone) -> KeyOutcome) -> Task<UiMessage> {
        let a = &mut self.add_accounts[i];
        let list = phone::list_id(a.id, &f);
        let Some(field) = a.phones.get_mut(&f) else { return Task::none() };
        match key(field) {
            KeyOutcome::Moved(Some(y)) => operation::scroll_to(list, AbsoluteOffset { x: 0.0, y }),
            KeyOutcome::Picked(c) => self.update_phone(i, f, PhoneMsg::Pick(c)),
            KeyOutcome::Moved(None) | KeyOutcome::Closed | KeyOutcome::Ignored => Task::none(),
        }
    }

    pub fn add_account_title(&self, id: window::Id) -> Option<String> {
        let a = self.add_accounts.iter().find(|a| a.id == id)?;
        let text = if a.login.is_empty() { "Add %s" } else { "Reconnect %s" };
        Some(text.replace("%s", a.network.display_name))
    }

    /// Desktop's numbered login steps in place of the bridge's instructions: its `auths` components (`A`, `f`, `T`,
    /// `b`) of the WhatsApp, Telegram, Signal and Google Messages logins, and its `BridgeV2Instructions` of the WeChat
    /// and Viber steps, with Desktop's icons in the text and a note under the steps.
    fn login_steps<'a>(&'a self, a: &'a AddAccount, s: &'a Step) -> Option<El<'a>> {
        let p = &self.pal;
        let line = |t: &str| -> El<'a> { step_line(p, t.to_string()).into() };
        let icon = |i: usize, height: f32| -> El<'a> {
            let color = Some(p.strong);
            svg(STEP_ICONS[i].clone()).width(18).height(height).style(move |_, _| svg::Style { color }).into()
        };
        let (items, note): (Vec<El<'a>>, Option<&'static str>) = match s.step_id.as_str() {
            "fi.mau.WeChat.login.qr" => {
                let tap = tr("Tap <add/> in the top-right corner, then <scan/> Scan");
                let tap = inline(p, tap, vec![("<add/>", icon(0, 18.0)), ("<scan/>", icon(1, 18.0))]);
                let note = "If WeChat warns that another device is already logged in, continuing will sign out that device. On your phone, select “Import recent chat history from this phone”, then tap “Log In”.";
                (vec![line(tr("Open WeChat on your phone")), tap, line(tr("Scan this QR code"))], Some(note))
            }
            "fi.mau.viber.login.qr" => {
                let more = inline(p, tr("Open Viber and tap <icon/> More at the bottom"), vec![("<icon/>", icon(2, 12.0))]);
                let qr = inline(p, tr("Tap <icon/> QR code"), vec![("<icon/>", icon(3, 18.0))]);
                (vec![more, qr, line(tr("Scan the QR code shown here"))], None)
            }
            DONE_STEP => {
                // Desktop's `<b>` is a `strong`.
                let text = tr("Tap <b>Sync</b> in the <b>Sync history</b> prompt to import older messages");
                let bold = crate::style::fw(iced::font::Weight::Bold);
                let spans: Vec<_> = text
                    .split("<b>")
                    .enumerate()
                    .flat_map(|(i, part)| {
                        let (strong, rest) = if i == 0 { ("", part) } else { part.split_once("</b>").unwrap_or((part, "")) };
                        [span::<UiMessage, _>(strong).font(bold), span(rest)]
                    })
                    .collect();
                let sync = rich_text(spans).size(rem(13.0)).line_height(iced::Pixels(rem(18.0))).color(p.strong);
                let note = "You can send and receive new messages without syncing your history.";
                (vec![line(tr("Open Viber on your phone")), sync.into()], Some(note))
            }
            _ => (self.device_steps(a, s)?, None),
        };
        // `.login-steps`: 288 px of rows 12 px apart, each with its number in a 20 px tinted circle.
        let rows = items.into_iter().enumerate().map(|(i, item)| -> El<'a> {
            let n = container(semi((i + 1).to_string(), 14.0).color(p.strong)).center(20).style(fill_box(p.strong.scale_alpha(0.06), 10.0));
            row![n, container(item).width(Fill)].spacing(8).into()
        });
        let steps = container(Column::with_children(rows).spacing(12).width(288)).center_x(Fill);
        let note = note.map(|n| txt(tr(n), 14.0).color(p.strong).center().width(Fill));
        Some(column![steps].push(note).spacing(14).into())
    }

    /// Desktop's Telegram help under a wrong two-step verification password: 12 px text over its `.list`, a 320 px
    /// column of steps 16 px apart, each with its number in a 24 px gray circle.
    fn password_help<'a>(&self, a: &AddAccount, s: &Step) -> Option<El<'a>> {
        if (a.bridge, s.step_id.as_str()) != ("telegram", "fi.mau.telegram.login.password.incorrect") {
            return None;
        }
        let fg = if self.dark { crate::hex(0xdfdfdf) } else { crate::hex(0x171717) };
        let circle = if self.dark { crate::hex(0x313131) } else { crate::hex(0xe8e8e8) };
        let steps = [
            "Open the Telegram app.",
            "Go to Settings → Privacy and Security → Two-Step Verification → Forgot Password.",
            "Follow the prompts to reset your password, then return to Beeper.",
        ];
        let rows = steps.iter().enumerate().map(|(i, t)| -> El<'a> {
            let n = container(med((i + 1).to_string(), 14.0).color(fg)).center(24).style(fill_box(circle, 12.0));
            let text = txt(tr(t), 13.0).line_height(iced::Pixels(rem(18.2))).color(fg);
            row![n, container(text).width(Fill)].spacing(12).align_y(Center).into()
        });
        let list = container(Column::with_children(rows).spacing(16).width(320).padding([8, 0])).center_x(Fill);
        let lead = txt(tr("If you've forgotten your password, reset it in Telegram:"), 12.0).line_height(iced::Pixels(rem(16.8))).color(fg);
        Some(column![lead, list].spacing(4).padding([0, 16]).into())
    }

    /// Desktop's `auths` steps of the bridge, which start with the device tabs.
    fn device_steps<'a>(&'a self, a: &'a AddAccount, s: &'a Step) -> Option<Vec<El<'a>>> {
        let p = &self.pal;
        let line = |t: String| step_line(p, t);
        let go = move |m: AddMsg| UiMessage::AddAccount(a.id, m);
        let hint = |t: &'static str| txt(tr(t), 12.0).line_height(iced::Pixels(rem(16.0))).color(p.sub);
        // `.TabList-module__segmented`: 10 px semibold tabs in a 2 px tinted pill, the active one on the card color.
        let tab = |label: &'static str, android: bool| -> El<'a> {
            let on = a.android == android;
            let bg = if on { p.card } else { CLEAR };
            button(container(semi(label, 10.0).color(p.strong)).center_y(14).padding([0, 4]))
                .padding(0)
                .style(btn_style([bg; 3], [p.strong; 3], 4.0))
                .on_press(go(AddMsg::Device(android)))
                .into()
        };
        let tabs = container(row![tab("iPhone", false), tab("Android", true)]).padding(2).style(fill_box(p.strong.scale_alpha(0.06), 6.0));
        // "Open %s on your <device/>", with the tabs in place of `<device/>`.
        let open = |app: &str| -> El<'a> {
            let text = tr(match app {
                "WhatsApp" => "Open WhatsApp on your <device/>",
                "Telegram" => "Open Telegram on your <device/>",
                _ => "Open Signal on your <device/>",
            });
            let (before, after) = text.split_once("<device/>").unwrap_or((text, ""));
            row![line(before.trim_end().to_string()), tabs, line(after.trim_start().to_string())].spacing(4).align_y(Center).into()
        };
        let qr = |app: &'static str| -> El<'a> { column![line(tr("Scan the QR code").into()), hint(app)].spacing(2).into() };
        let (first, [two, three], rest): (El<'a>, [&'static str; 2], Vec<El<'a>>) = match (a.bridge, a.android) {
            ("whatsapp", android) => {
                let code = s.step_id == "fi.mau.whatsapp.login.code" || s.display_and_wait.as_ref().is_some_and(|d| d.kind == "code");
                let rest = match code {
                    true => vec![
                        line(tr("Tap \"Link with phone number instead\"").into()).into(),
                        line(tr("Enter the code below").into()).into(),
                    ],
                    false => vec![qr("Grant WhatsApp camera access if prompted")],
                };
                let steps = match android {
                    false => ["Tap \"Settings\" in the bottom right", "Tap \"Linked Devices\" → \"Link device\""],
                    true => ["Tap the menu (⋮) in the top right", "Tap \"Linked devices\" → \"Link a device\""],
                };
                (open("WhatsApp"), steps, rest)
            }
            ("telegram", android) if s.step_id == "fi.mau.telegram.login.show_qr" => {
                let steps = match android {
                    false => ["Tap \"Settings\" in the bottom right → \"Devices\"", "Tap \"Link Desktop Device\""],
                    true => ["Tap the top left (☰) → \"Settings\"", "Tap \"Devices\" → \"Link Desktop Device\""],
                };
                (open("Telegram"), steps, vec![qr("Grant Telegram camera access if prompted")])
            }
            ("signal", android) => {
                let steps = match android {
                    false => ["Tap your profile in the top left → \"Settings\"", "Tap \"Linked Devices\" → \"Link a New Device\""],
                    true => ["Tap your profile in the top left", "Tap \"Linked Devices\" → \"Link a new Device\""],
                };
                (open("Signal"), steps, vec![qr("Grant Signal camera access if prompted")])
            }
            ("gmessages", _) => (
                line(tr("Open Messages by Google on your Android phone").into()).into(),
                ["Open the side menu", "Click \"Device pairing\""],
                vec![line(bridge_text("instructions", &s.step_id, &s.instructions)).into()],
            ),
            _ => return None,
        };
        Some([first, line(tr(two).into()).into(), line(tr(three).into()).into()].into_iter().chain(rest).collect())
    }

    pub fn view_add_account<'a>(&'a self, a: &'a AddAccount) -> El<'a> {
        let p = &self.pal;
        let go = move |m: AddMsg| UiMessage::AddAccount(a.id, m);
        let name = a.network.display_name;
        let primary = |label: String, msg: Option<AddMsg>| -> El<'a> {
            let fg = if msg.is_some() { Color::WHITE } else { Color::WHITE.scale_alpha(0.6) };
            let bg = if msg.is_some() { p.accent } else { p.accent.scale_alpha(0.4) };
            button(container(semi(label, 13.0)).center_x(Fill).center_y(32))
                .width(Fill)
                .padding([0, 14])
                .style(btn_style([bg; 3], [fg; 3], 16.0))
                .on_press_maybe(msg.map(go))
                .into()
        };
        let para = |s: String| -> El<'a> { txt(s, 14.0).color(p.strong).center().width(Fill).into() };
        // Desktop's `Za(error, "error")`: the bridge's error in the app language.
        let error_line = |e: &str| -> El<'a> {
            row![svg(ERROR_CIRCLE.clone()).width(16).height(16), txt(bridge_error(e), 13.0).color(p.danger)]
                .spacing(4)
                .align_y(Center)
                .into()
        };
        // Desktop's `ErrorBox` with the Start Over or Reload QR Code button.
        let error_box = |label: &'static str| -> Option<El<'a>> {
            a.error.as_deref().map(|e| {
                let msg = (!a.busy).then_some(AddMsg::StartOver);
                column![
                    txt(tr("Something went wrong and we couldn't add your account."), 13.0).color(p.strong),
                    error_line(e),
                    primary(tr(label).into(), msg)
                ]
                .spacing(12)
                .align_x(Center)
                .into()
            })
        };
        let back = |step: &Step| -> Option<El<'a>> {
            step.can_cancel().then(|| {
                button(med(tr("Back"), 13.0))
                    .style(btn_style([CLEAR; 3], [p.accent, p.accent, p.sub], 6.0))
                    .on_press_maybe((!a.busy).then_some(go(AddMsg::Back)))
                    .into()
            })
        };
        // Desktop's `auths` steps take the place of the bridge's instructions when it has them.
        let instructions = |s: &'a Step| {
            (!s.instructions.is_empty()).then(|| {
                let text = bridge_text("instructions", &s.step_id, &s.instructions);
                self.login_steps(a, s).unwrap_or_else(|| para(text))
            })
        };
        let loading = || para(tr("Loading...").into());
        let body: El<'a> = match (&a.failed, &a.step) {
            (Some(_), _) => para(tr("Something went wrong and we couldn't add your account.").into()),
            (None, None) => error_box("Start Over").unwrap_or_else(loading),
            (None, Some(s)) => match s.kind.as_str() {
                "display_and_wait" => {
                    let d = s.display_and_wait.as_ref().expect("a display step");
                    let shown: Option<El<'a>> = match d.kind.as_str() {
                        "emoji" => a.emoji.clone().map(|h| image(h).width(100).height(100).into()),
                        "code" => Some({
                            let code = semi(d.data.as_str(), 22.0).color(p.strong);
                            let copy = button(svg(COPY.clone()).width(16).height(16))
                                .padding(6)
                                .style(btn_style([CLEAR, p.hover, p.hover], [p.strong; 3], 6.0))
                                .on_press(go(AddMsg::Copy(d.data.clone())));
                            row![code, copy].spacing(8).align_y(Center).into()
                        }),
                        "qr" => None,
                        _ => Some(ic(icon::HISTORY, 32.0).color(p.strong.scale_alpha(0.5)).into()),
                    };
                    let shown = shown.filter(|_| a.error.is_none());
                    let label = if d.kind == "qr" || d.kind == "emoji" { "Reload QR Code" } else { "Start Over" };
                    let col = column![].push(instructions(s)).push(shown).push(error_box(label)).push(back(s));
                    col.spacing(16).align_x(Center).into()
                }
                "user_input" => column![self.view_add_form(a, s)].push(self.password_help(a, s)).spacing(16).into(),
                // Desktop's `BrowserLoginOption`, whose text shows the state of the browser window.
                "cookies" => {
                    let hidden = s.cookies.as_ref().is_some_and(|c| c.hidden);
                    let window = trx("window", "browser login UI container");
                    match &a.browser {
                        Browser::Open if !hidden && !a.busy => para(trf("Continue login in the browser %s", &[&window])),
                        Browser::Open | Browser::Closed if hidden || a.busy => para(tr("Authenticating...").into()),
                        browser => {
                            let text = trf("Click the button below to open the browser and begin authentication with %s.", &[&name]);
                            let error = match browser {
                                Browser::Failed(e) => Some(error_line(e)),
                                _ => None,
                            };
                            let open = auth_button(trf("Open %s Login", &[&name]), go(AddMsg::OpenBrowser));
                            column![para(text), open].push(error).spacing(16).align_x(Center).into()
                        }
                    }
                }
                _ => {
                    let done = primary(tr("Done").into(), Some(AddMsg::Close));
                    column![].push(instructions(s)).push(done).spacing(16).into()
                }
            },
        };
        let complete = a.step.as_ref().is_some_and(|s| s.kind == "complete");
        let header = match (complete, &a.who) {
            (true, _) => trf("Connected to %s", &[&name]),
            (false, Some(who)) => trf("Reconnecting %s", &[who]),
            (false, None) => trf("Connect to %s", &[&name]),
        };
        let mark: El<'a> = match a.qr.clone().filter(|_| a.error.is_none()) {
            Some(qr) => {
                let plate = container(svg(qr).width(QR_SIZE).height(QR_SIZE)).padding(8).style(fill_box(Color::WHITE, 12.0));
                let logo =
                    container(svg(brand_svg(a.network.bridge_id)).width(20).height(20)).padding(2).style(fill_box(Color::WHITE, 6.0));
                stack![plate, container(logo).center(QR_SIZE + 16.0)].into()
            }
            None => svg(brand_svg(a.network.bridge_id)).width(52).height(52).into(),
        };
        let body = match a.remote {
            Remote::Open => Some(body),
            Remote::Disabled => None,
            Remote::Confirm => Some(primary(format!("{} →", tr("Continue")), Some(AddMsg::Continue))),
        };
        let failed = a.failed.as_deref().map(error_line);
        let main = column![mark, semi(header, 17.0).color(p.strong).center()]
            .push(a.status.clone().map(para))
            .push(body)
            .push(failed)
            .spacing(24)
            .align_x(Center)
            .padding([24, 32]);
        let limits = CLOUD_LIMITS
            .iter()
            .find(|l| l.0 == a.network.bridge_id)
            .map(|l| row![txt(tr("Not supported:"), 12.0).color(p.strong), txt(tr(l.1), 12.0).color(p.sub).width(Fill)].spacing(4));
        let about = rich_text![
            span(tr("Your login information and chats will be synced across devices and data is stored encrypted on Beeper servers."))
                .color(p.sub),
            span(" "),
            span(tr("Learn more")).color(p.accent).link(UiMessage::OpenUrl(LEARN_MORE.into())),
        ]
        .on_link_click(|m: UiMessage| m)
        .size(rem(12.0))
        .width(Fill);
        let fine = (a.login.is_empty() && !complete).then(|| column![about].push(limits).spacing(6).padding([16, 24]));
        let card = column![container(main).center_x(Fill).height(Fill)].push(fine);
        let card = container(scrollable(card).height(Fill)).height(Fill).style(fill_box(p.prefs_group, 14.0));
        let options: Vec<&Flow> = a.other_flows().collect();
        let chevron = (!options.is_empty()).then(|| {
            button(container(ic(icon::EXPAND_MORE, 20.0).color(p.muted)).center(28))
                .padding(0)
                .style(btn_style([CLEAR, p.hover, p.hover], [p.muted; 3], 6.0))
                .on_press(go(AddMsg::Menu))
        });
        let title = stack![
            container(semi(name, 13.0).color(p.strong)).center_x(Fill).center_y(30),
            container(row![].push(chevron)).align_right(Fill).center_y(30).padding(Padding::ZERO.right(4)),
        ];
        let menu = (a.menu && !options.is_empty()).then(|| {
            let items = options.into_iter().map(|f| {
                let key = format!("{}/{}", a.bridge, f.id);
                let label = FLOW_LABELS.iter().find(|l| l.0 == key).map(|l| tr(l.1).to_string());
                let label = label.unwrap_or_else(|| match f.name.is_empty() {
                    true => f.id.clone(),
                    false => bridge_text("flowName", &key, &f.name),
                });
                menu_item(p, None, label, "", (!a.busy).then(|| go(AddMsg::Flow(f.id.clone()))))
            });
            let list = crate::menu_card(p, Column::with_children(items), 200);
            crate::backdrop(container(list).align_right(Fill).height(Fill).padding(Padding::ZERO.top(30).right(8)))
                .on_press(go(AddMsg::Menu))
        });
        let window = column![title, card].padding(Padding::new(8.0).top(0.0));
        container(stack![window].push(menu)).style(fill_box(p.window, 0.0)).into()
    }

    /// Desktop's `login-form login-user-input`: one row for each field of the step, Continue and Back.
    fn view_add_form<'a>(&'a self, a: &'a AddAccount, s: &'a Step) -> El<'a> {
        let p = &self.pal;
        let go = move |m: AddMsg| UiMessage::AddAccount(a.id, m);
        let fields = &s.user_input.as_ref().expect("a form step").fields;
        let rows = fields.iter().map(|f| -> El<'a> {
            let value = a.values[&f.id].as_str();
            // Desktop's `rs(ci(stepId, field.id), …)`: the field's texts in the app language.
            let key = format!("{}/{}", s.step_id, f.id);
            let name = bridge_text("fieldLabel", &key, &f.name);
            let description = bridge_text("fieldDescription", &key, &f.description);
            let secret = matches!(f.kind.as_str(), "password" | "token");
            let shown = a.shown.contains(&f.id);
            let id = f.id.clone();
            let control: El<'a> = match f.kind.as_str() {
                "select" if !f.options.is_empty() => {
                    pick_list(f.options.as_slice(), Some(value.to_string()).filter(|v| !v.is_empty()), move |v| {
                        go(AddMsg::Field(id.clone(), v))
                    })
                    .placeholder(trf("Select %s...", &[&name]))
                    .width(Fill)
                    .into()
                }
                "phone_number" if let Some(field) = a.phones.get(&f.id) => {
                    let ids = (phone::input_id(a.id, &f.id), phone::list_id(a.id, &f.id));
                    let phone = move |m| go(AddMsg::Phone(id.clone(), m));
                    self.view_phone(field, ids, tr("Enter phone number"), phone, go(AddMsg::Submit))
                }
                _ => {
                    let hint = if secret { "••••••••".into() } else { name.clone() };
                    let input = text_input(&hint, value)
                        .on_input(move |v| go(AddMsg::Field(id.clone(), v)))
                        .on_submit(go(AddMsg::Submit))
                        .secure(secret && !shown)
                        .size(rem(14.0))
                        .padding([8, 10])
                        .style(input_style(p.input, p.strong, p.tertiary, p.accent, 8.0));
                    let accessory = |handle: svg::Handle, msg: AddMsg| {
                        button(svg(handle).width(16).height(16))
                            .padding(4)
                            .style(btn_style([CLEAR, p.hover, p.hover], [p.strong; 3], 6.0))
                            .on_press(go(msg))
                    };
                    let eye = secret.then(|| accessory(if shown { EYE_OFF.clone() } else { EYE.clone() }, AddMsg::Reveal(f.id.clone())));
                    let clear =
                        (!secret && !value.is_empty()).then(|| accessory(CLEAR_FIELD.clone(), AddMsg::Field(f.id.clone(), String::new())));
                    row![input].push(eye).push(clear).spacing(4).align_y(Center).into()
                }
            };
            let error = a.touched.contains(&f.id).then(|| invalid(f, value)).flatten();
            column![semi(name, 13.0).color(p.strong), control]
                .push(error.map(|e| txt(e, 12.0).color(p.text_danger)))
                .push((!description.is_empty()).then(|| txt(description, 12.0).color(p.strong.scale_alpha(0.7))))
                .spacing(4)
                .into()
        });
        let ready = !a.busy && fields.iter().all(|f| invalid(f, &a.values[&f.id]).is_none());
        let fg = if ready { Color::WHITE } else { Color::WHITE.scale_alpha(0.6) };
        let bg = if ready { p.accent } else { p.accent.scale_alpha(0.4) };
        let submit = button(container(semi(tr("Continue"), 13.0)).center_x(Fill).center_y(28))
            .width(Fill)
            .style(btn_style([bg; 3], [fg; 3], 14.0))
            .on_press_maybe(ready.then(|| go(AddMsg::Submit)));
        let back = s.can_cancel().then(|| {
            button(med(tr("Back"), 13.0))
                .style(btn_style([CLEAR; 3], [p.accent, p.accent, p.sub], 6.0))
                .on_press_maybe((!a.busy).then(|| go(AddMsg::Back)))
        });
        let intro = (!s.instructions.is_empty())
            .then(|| txt(bridge_text("instructions", &s.step_id, &s.instructions), 14.0).color(p.strong).center().width(Fill));
        let start_over = a.error.as_deref().map(|e| {
            column![
                txt(tr("Something went wrong and we couldn't add your account."), 13.0).color(p.strong),
                txt(bridge_error(e), 13.0).color(p.danger),
                button(med(tr("Start Over"), 13.0)).on_press_maybe((!a.busy).then(|| go(AddMsg::StartOver))),
            ]
            .spacing(8)
            .align_x(Center)
        });
        column![].push(intro).extend(rows).push(submit).push(back).push(start_over).spacing(16).align_x(Center).width(Fill).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fuzz::clicks;
    use crate::fuzz::task_messages as run;

    #[test]
    fn fields_validate_like_desktop() {
        let field = |kind: &str| Field { kind: kind.into(), ..Field::default() };
        assert_eq!(invalid(&field("username"), " ").as_deref(), Some("This field is required"));
        assert_eq!(invalid(&field("phone_number"), "+12015550123").as_deref(), None);
        let bad = Some("Please enter a valid phone number");
        assert_eq!(invalid(&field("phone_number"), "+15550100").as_deref(), bad, "libphonenumber wants 10 digits");
        assert_eq!(invalid(&field("phone_number"), "+1").as_deref(), bad);
        assert_eq!(invalid(&field("email"), "a@b").as_deref(), Some("Please enter a valid email address"));
        assert_eq!(invalid(&field("domain"), "bsky.social").as_deref(), None);
        assert_eq!(invalid(&field("url"), "x.com").as_deref(), Some("Please enter a valid URL"));
        let code = Field { min_length: 5, max_length: 5, ..field("2fa_code") };
        assert_eq!(invalid(&code, "123").as_deref(), Some("Must be exactly 5 characters"));
        let pin = Field { pattern: "^[0-9]+$".into(), ..field("2fa_code") };
        assert_eq!(invalid(&pin, "12a").as_deref(), Some("Invalid format"));
        assert_eq!(invalid(&Field { pattern: "(".into(), ..field("text") }, "x"), None, "a bad pattern is ignored");
    }

    /// Desktop's `auths` steps: WhatsApp's numbered steps with the iPhone and Android tabs in place of the bridge's
    /// instructions, the pairing-code variant, and Google Messages with the bridge's instructions as the last step.
    #[test]
    fn login_steps_like_desktop() {
        use iced_test::simulator::simulator;
        let mut app = crate::fuzz::demo_app("login-steps");
        let _ = app.open_add_account("whatsapp", None);
        let step = |id: &str, kind: &str| Step {
            kind: "display_and_wait".into(),
            step_id: id.into(),
            instructions: "Scan the QR code on your phone".into(),
            display_and_wait: Some(beeper_core::bridge_login::Display { kind: kind.into(), ..Default::default() }),
            ..Step::default()
        };
        app.add_accounts[0].step = Some(step("fi.mau.whatsapp.login.qr", "qr"));
        let finds = |app: &BeeperGuiApp, texts: &[&str]| {
            let mut sim = simulator(app.view_add_account(&app.add_accounts[0]));
            texts.iter().all(|t| sim.find(*t).is_ok())
        };
        assert!(finds(&app, &["Open WhatsApp on your", "iPhone", "Android", "Tap \"Settings\" in the bottom right"]));
        assert!(finds(&app, &["Scan the QR code", "Grant WhatsApp camera access if prompted"]));
        let id = app.add_accounts[0].id;
        let _ = app.update(UiMessage::AddAccount(id, AddMsg::Device(true)));
        assert!(finds(&app, &["Tap the menu (⋮) in the top right", "Tap \"Linked devices\" → \"Link a device\""]));
        app.add_accounts[0].step = Some(step("fi.mau.whatsapp.login.code", "code"));
        assert!(finds(&app, &["Tap \"Link with phone number instead\"", "Enter the code below"]));
        app.add_accounts[0].bridge = "gmessages";
        app.add_accounts[0].step = Some(step("fi.mau.gmessages.qr", "qr"));
        assert!(finds(&app, &["Open the side menu", "Click \"Device pairing\"", "Scan the QR code on your phone"]));
    }

    /// Desktop's `BridgeV2Instructions` of the WeChat and Viber QR steps, with its icons in the text, and of the
    /// Viber step after the login, whose Done closes the window.
    #[test]
    fn wechat_and_viber_steps_like_desktop() {
        use iced_test::simulator::simulator;
        let mut app = crate::fuzz::demo_app("wechat-viber-steps");
        let _ = app.open_add_account("wechat", None);
        let step = |kind: &str, id: &str| Step {
            kind: kind.into(),
            step_id: id.into(),
            instructions: "The bridge's own steps".into(),
            display_and_wait: (kind == "display_and_wait")
                .then(|| beeper_core::bridge_login::Display { kind: "qr".into(), ..Default::default() }),
            ..Step::default()
        };
        app.add_accounts[0].step = Some(step("display_and_wait", "fi.mau.WeChat.login.qr"));
        let shows = |app: &BeeperGuiApp, texts: &[&str]| {
            let mut sim = simulator(app.view_add_account(&app.add_accounts[0]));
            texts.iter().all(|t| sim.find(*t).is_ok()) && sim.find("The bridge's own steps").is_err()
        };
        assert!(shows(&app, &["Open WeChat on your phone", "Tap", "in the top-right corner, then", "Scan", "Scan this QR code"]));
        assert!(shows(&app, &["If WeChat warns that another device is already logged in, continuing will sign out that device. On your phone, select “Import recent chat history from this phone”, then tap “Log In”."]));
        app.add_accounts[0].step = Some(step("display_and_wait", "fi.mau.viber.login.qr"));
        assert!(shows(&app, &["Open Viber and tap", "More at the bottom", "QR code", "Scan the QR code shown here"]));
        app.add_accounts[0].step = Some(step("complete", DONE_STEP));
        assert!(shows(&app, &["Open Viber on your phone", "You can send and receive new messages without syncing your history."]));
        let mut sim = simulator(app.view_add_account(&app.add_accounts[0]));
        let _ = sim.click("Done").expect("the Done button");
        let id = app.add_accounts[0].id;
        assert!(matches!(sim.into_messages().next(), Some(UiMessage::AddAccount(w, AddMsg::Close)) if w == id));
    }

    #[test]
    fn every_addable_network_has_a_cloud_bridge_but_bluesky() {
        let none: Vec<_> = crate::networks::addable(&beeper_core::features::Flags::default())
            .filter(|n| cloud_bridge(n).is_none())
            .map(|n| n.bridge_id)
            .collect();
        // macOS also adds iMessage, which runs on the Mac and not in Beeper Cloud.
        let local: &[&str] = if cfg!(target_os = "macos") { &["imessage", "bluesky"] } else { &["bluesky"] };
        assert_eq!(none, local);
        assert_eq!(cloud_bridge(find_network("slackgo").unwrap()), Some("slackgo"));
    }

    /// A signed-in app whose homeserver, Beeper API and app API are the stub bridges of `beeper_core::bridge_stub`.
    async fn stub_app(name: &str) -> (BeeperGuiApp, beeper_core::bridge_stub::Bridges) {
        use beeper_core::models::{MatrixSession, Profile};
        let (url, bridges) = beeper_core::bridge_stub::homeserver().await;
        let dir = beeper_core::testing::temp_dir(&format!("add-account-{name}"));
        let config = beeper_core::config::CentralConfig::new(dir, "127.0.0.1".into(), 23631, None);
        let store = beeper_core::store::StoreLock::shared(beeper_core::store::BeeperStore::from_config(config));
        let session = MatrixSession { user_id: "@me:beeper.com".into(), access_token: "t".into(), device_id: "D".into(), jwt: None };
        store.write().sign_in(session, url.clone(), &Profile::default(), Vec::new());
        let mut app = BeeperGuiApp::boot(store).0;
        let cloud = beeper_core::cloud::Cloud::new(&url, &url);
        app.cloud = std::sync::Arc::new(beeper_core::cloud::Cloud { app_api: url, ..cloud });
        (app, bridges)
    }

    /// Sends `msg`, and returns the messages of its task.
    async fn send(app: &mut BeeperGuiApp, msg: UiMessage) -> Vec<UiMessage> {
        run(app.update(msg)).await
    }

    /// Sends each message of a task until no task outputs more, like the iced runtime.
    async fn settle(app: &mut BeeperGuiApp, msg: UiMessage) {
        let mut queue = vec![msg];
        while let Some(m) = queue.pop() {
            queue.extend(send(app, m).await);
        }
    }

    fn shows(app: &BeeperGuiApp, id: window::Id, text: &str) -> bool {
        iced_test::simulator(app.view(id)).find(text).is_ok()
    }

    /// Clicks the text in the window, and returns the message of the click.
    fn click(app: &BeeperGuiApp, id: window::Id, text: &str) -> UiMessage {
        let mut sim = iced_test::simulator(app.view(id));
        sim.click(text).unwrap_or_else(|e| panic!("click {text}: {e:?}"));
        sim.into_messages().next().unwrap_or_else(|| panic!("{text} has no message"))
    }

    /// The one login message of the Add Account window among the messages of a task, without the platform status.
    fn one(msgs: Vec<UiMessage>) -> UiMessage {
        let login = |m: &UiMessage| matches!(m, UiMessage::AddAccount(_, m) if !matches!(m, AddMsg::Status(_)));
        let mut ours: Vec<_> = msgs.into_iter().filter(login).collect();
        assert_eq!(ours.len(), 1, "one message of the window");
        ours.remove(0)
    }

    /// The Telegram flows of the stub: the QR code of the preferred flow, the phone flow of "Login options", a
    /// wrong phone number, a wrong code and Start Over, Desktop's help under a wrong password, and the cancel of the
    /// login when the window closes.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_new_telegram_account_goes_through_the_bridge_steps() {
        let (mut app, bridges) = stub_app("telegram").await;
        let flows = one(send(&mut app, UiMessage::OpenAddAccount("telegram".into(), None)).await);
        let id = app.add_accounts[0].id;
        assert_eq!(app.title(id), "Add Telegram");
        assert!(shows(&app, id, "Connect to Telegram") && shows(&app, id, "Loading..."));
        let started = one(send(&mut app, flows).await);
        let wait = one(send(&mut app, started).await);
        assert!(app.add_accounts[0].qr.is_some(), "Desktop prefers the QR flow of Telegram");
        // Desktop's Telegram steps take the place of the bridge's instructions.
        assert!(shows(&app, id, "Open Telegram on your") && shows(&app, id, "Tap \"Link Desktop Device\""));
        assert!(!shows(&app, id, "Scan the QR code on your phone to log in"));
        assert!(shows(&app, id, "Not supported:"), "the fine print has the Beeper Cloud limitations");
        assert!(!shows(&app, id, "Phone Number"), "the Login options menu is closed");
        let _ = send(&mut app, UiMessage::AddAccount(id, AddMsg::Menu)).await;
        let pick = click(&app, id, "Phone Number");
        assert!(matches!(pick, UiMessage::AddAccount(_, AddMsg::Flow(ref f)) if f == "phone"));
        let started = one(send(&mut app, pick).await);
        assert!(send(&mut app, wait).await.is_empty(), "the reply of the QR process is dropped");
        assert!(send(&mut app, started).await.is_empty());
        assert!(shows(&app, id, "Please enter your phone number") && shows(&app, id, "Phone number"));
        let field = "fi.mau.telegram.login.phone_number".to_string();
        let _ = send(&mut app, UiMessage::AddAccount(id, AddMsg::Field(field.clone(), "+112".into()))).await;
        assert!(shows(&app, id, "Please enter a valid phone number"));
        assert!(send(&mut app, UiMessage::AddAccount(id, AddMsg::Submit)).await.is_empty(), "a bad form is not sent");
        let _ = send(&mut app, UiMessage::AddAccount(id, AddMsg::Field(field.clone(), "+12015550123".into()))).await;
        let sent = {
            let m = click(&app, id, "Continue");
            one(send(&mut app, m).await)
        };
        let _ = send(&mut app, sent).await;
        assert!(shows(&app, id, "Please enter the code sent to your phone") && shows(&app, id, "Back"));
        let code = "fi.mau.telegram.login.code".to_string();
        let wrong = beeper_core::bridge_stub::WRONG_CODE.to_string();
        let _ = send(&mut app, UiMessage::AddAccount(id, AddMsg::Field(code.clone(), wrong.clone()))).await;
        settle(&mut app, UiMessage::AddAccount(id, AddMsg::Submit)).await;
        assert!(shows(&app, id, "Something went wrong and we couldn't add your account."));
        assert!(shows(&app, id, "Invalid phone code"));
        settle(&mut app, UiMessage::AddAccount(id, AddMsg::StartOver)).await;
        assert!(shows(&app, id, "Please enter your phone number"), "Start Over starts the flow again");
        let _ = send(&mut app, UiMessage::AddAccount(id, AddMsg::Field(field, "+12015550123".into()))).await;
        settle(&mut app, UiMessage::AddAccount(id, AddMsg::Submit)).await;
        let _ = send(&mut app, UiMessage::AddAccount(id, AddMsg::Field(code, "12345".into()))).await;
        settle(&mut app, UiMessage::AddAccount(id, AddMsg::Submit)).await;
        assert!(shows(&app, id, "You have two-factor authentication enabled."));
        let help = "If you've forgotten your password, reset it in Telegram:";
        assert!(!shows(&app, id, help), "the help is only for a wrong password");
        let password = "fi.mau.telegram.login.password".to_string();
        let _ = send(&mut app, UiMessage::AddAccount(id, AddMsg::Field(password.clone(), wrong))).await;
        settle(&mut app, UiMessage::AddAccount(id, AddMsg::Submit)).await;
        assert!(shows(&app, id, "Incorrect password, please try again."));
        for text in [help, "Open the Telegram app.", "Follow the prompts to reset your password, then return to Beeper."] {
            assert!(shows(&app, id, text), "Desktop's help under a wrong password shows {text}");
        }
        let process = app.add_accounts[0].process.clone().expect("a running login");
        settle(&mut app, UiMessage::Window(crate::windows::Msg::Closed(id))).await;
        assert!(app.add_accounts.is_empty());
        let cancel = format!("POST /_matrix/client/unstable/com.beeper.bridge/telegram/_matrix/provision/v3/login/cancel/{process}");
        assert!(bridges.requests().contains(&cancel), "a closed window cancels its login");
    }

    /// The WhatsApp pairing code of the stub: the code with its copy button, Back to the phone number, and the login,
    /// which closes the window and loads the new account. Then the account logs in again.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_whatsapp_pairing_code_connects_and_reconnects_the_account() {
        let (mut app, _) = stub_app("whatsapp").await;
        settle(&mut app, UiMessage::OpenAddAccount("whatsapp".into(), None)).await;
        let id = app.add_accounts[0].id;
        assert_eq!(app.store.read().bridge_accounts.len(), 1, "the QR flow logged in at once with the stub");
        assert_eq!(app.toast.as_ref().map(|t| t.text.as_str()), Some("Connected to WhatsApp"));
        settle(&mut app, UiMessage::Window(crate::windows::Msg::Closed(id))).await;
        let login = app.store.read().bridge_accounts[0].login_id.clone();
        let flows = one(send(&mut app, UiMessage::OpenAddAccount("whatsapp".into(), Some(login))).await);
        let id = app.add_accounts[0].id;
        assert_eq!(app.title(id), "Reconnect WhatsApp");
        assert!(shows(&app, id, "Reconnecting 15550100"), "the header names the account");
        let started = one(send(&mut app, flows).await);
        let _ = send(&mut app, started).await;
        let _ = send(&mut app, UiMessage::AddAccount(id, AddMsg::Menu)).await;
        let started = {
            let m = click(&app, id, "Pairing code");
            one(send(&mut app, m).await)
        };
        let _ = send(&mut app, started).await;
        let _ = send(&mut app, UiMessage::AddAccount(id, AddMsg::Field("phone_number".into(), "+12015550123".into()))).await;
        let shown = one(send(&mut app, UiMessage::AddAccount(id, AddMsg::Submit)).await);
        // The wait of the stub ends at once, so it does not run: a real bridge answers when the code is typed.
        drop(app.update(shown));
        assert!(shows(&app, id, "ABCD-EFGH") && shows(&app, id, "Back"));
        let back = {
            let m = click(&app, id, "Back");
            one(send(&mut app, m).await)
        };
        let _ = send(&mut app, back).await;
        assert!(shows(&app, id, "Please enter your phone number"), "Back shows the step before");
        let _ = send(&mut app, UiMessage::AddAccount(id, AddMsg::Field("phone_number".into(), "+12015550123".into()))).await;
        settle(&mut app, UiMessage::AddAccount(id, AddMsg::Submit)).await;
        assert_eq!(app.toast.as_ref().map(|t| t.text.as_str()), Some("Reconnected to WhatsApp"));
    }

    /// Desktop shows the QR code only in the `scan-qr` stage: once Viber's QR login completes, the window that stays
    /// open with "Connected to Viber" and the history steps shows the Viber mark.
    #[test]
    fn a_completed_qr_login_shows_the_network_mark() {
        let mut app = crate::fuzz::demo_app("complete-mark");
        let _ = app.open_add_account("viber", None);
        let a = &mut app.add_accounts[0];
        (a.process, a.qr) = (Some("process".into()), Some(qr_svg("viber://pair")));
        let done = Step { kind: "complete".into(), step_id: DONE_STEP.into(), instructions: "Logged in".into(), ..Step::default() };
        let _ = app.show_step(0, done);
        assert!(app.add_accounts[0].qr.is_none(), "no QR code");
        let mut sim = iced_test::simulator(app.view_add_account(&app.add_accounts[0]));
        assert!(sim.find("Connected to Viber").is_ok() && sim.find("Done").is_ok());
    }

    /// Desktop's `network-status` banner, the Reconnect button of the account page and the Reconnect item of the
    /// account menu, for a cloud account whose bridge asks for a new login.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_logged_out_account_offers_reconnect() {
        let (mut app, _) = stub_app("reconnect").await;
        let out = serde_json::json!({ "state_event": "BAD_CREDENTIALS", "has_error": true, "message": "Logged out" });
        let bridges =
            serde_json::json!({ "whatsapp": { "bridgeState": { "bridgeType": "whatsapp" }, "remoteState": { "15550100": out } } });
        let accounts = beeper_core::cloud::cloud_accounts(&bridges).expect("valid bridge states");
        app.store.write().set_cloud_accounts(accounts);
        let again = |m: UiMessage| matches!(m, UiMessage::OpenAddAccount(n, Some(l)) if n == "whatsapp" && l == "15550100");
        {
            let mut inbox = iced_test::simulator(app.view_sidebar());
            assert!(inbox.find("WhatsApp disconnected").is_ok(), "the inbox shows it too (Desktop's `mpt`)");
            assert!(inbox.find("Reconnect").is_err(), "Reconnect shows only in its space");
        }
        app.list.selected_account_filter = Some("whatsapp".into());
        let mut sim = iced_test::simulator(app.view_sidebar());
        assert!(sim.find("WhatsApp disconnected").is_ok());
        sim.click("Reconnect").expect("the Reconnect of the banner");
        assert!(sim.into_messages().any(again));
        let _ = app.update(UiMessage::Settings(crate::settings::Msg::Open(crate::SettingsTab::Accounts)));
        let _ = app.update(UiMessage::Accounts(crate::accounts::Msg::Open(Some("whatsapp".into()))));
        let mut sim = iced_test::simulator(app.view_settings());
        assert!(sim.find("Account Disconnected").is_ok() && sim.find("Logged out").is_ok());
        sim.click("Reconnect").expect("the Reconnect of the account page");
        assert!(sim.into_messages().any(again));
        let mut sim = iced_test::simulator(app.account_menu("whatsapp", crate::SpaceMenu::Rail).0);
        sim.click("Reconnect").expect("the Reconnect of the account menu");
        assert!(sim.into_messages().any(again));
        settle(&mut app, UiMessage::OpenAddAccount("whatsapp".into(), Some("15550100".into()))).await;
        assert_eq!(app.add_accounts[0].login, "15550100", "the window logs the account in again");
    }

    /// The X cookie step of the stub: the browser window opens at once, and a machine without a browser says which
    /// browsers log in. A window that the user closes offers the button again, and the fields of a finished login go
    /// to the bridge.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_cookie_login_sends_the_fields_of_the_browser_window() {
        let (mut app, bridges) = stub_app("twitter").await;
        settle(&mut app, UiMessage::OpenAddAccount("twitter".into(), None)).await;
        let id = app.add_accounts[0].id;
        assert!(shows(&app, id, "Click the button below to open the browser and begin authentication with X."));
        assert!(shows(&app, id, "To log in to X, install Chrome, Chromium, Brave, Edge or Vivaldi."));
        assert_eq!(app.add_accounts[0].other_flows().count(), 0, "X has one flow, so no Login options");
        app.find_browser = || Some("/nonexistent/chromium".into());
        let open = click(&app, id, "Open X Login");
        drop(app.update(open));
        assert!(shows(&app, id, "Continue login in the browser window"));
        let attempt = app.add_accounts[0].run;
        let closed = Outcome { finished: false, last_url: "https://x.com/i/flow/login".into(), ..Outcome::default() };
        let _ = send(&mut app, UiMessage::AddAccount(id, AddMsg::Browser(attempt, Ok(closed)))).await;
        assert!(shows(&app, id, "Open X Login") && !shows(&app, id, "Continue login in the browser window"));
        let fields = serde_json::json!({ "ct0": "a", "auth_token": "b" }).as_object().cloned().unwrap();
        let done = Outcome { finished: true, last_url: "https://x.com/home".into(), fields };
        let submit = app.update(UiMessage::AddAccount(id, AddMsg::Browser(attempt, Ok(done))));
        assert!(shows(&app, id, "Authenticating..."));
        for m in run(submit).await {
            settle(&mut app, m).await;
        }
        assert_eq!(app.toast.as_ref().map(|t| t.text.as_str()), Some("Connected to X"));
        let process = bridges.requests().into_iter().find(|r| r.ends_with("/fi.mau.twitter.login.enter_cookies/cookies"));
        assert!(process.is_some(), "the fields went to the cookie step");
    }

    /// Desktop's `PlatformRemoteStatus` with the stub's `/platform-status`: the message of the platform under the
    /// header, a `disabled` status that hides the login, and a `confirm` status that hides it until "Continue →".
    #[tokio::test(flavor = "multi_thread")]
    async fn the_platform_status_shows_its_message_and_gates_the_login() {
        let (mut app, bridges) = stub_app("platform-status").await;
        let status =
            |msgs: Vec<UiMessage>| msgs.into_iter().find(|m| matches!(m, UiMessage::AddAccount(_, AddMsg::Status(_)))).expect("a status");
        let opened = status(send(&mut app, UiMessage::OpenAddAccount("signal".into(), None)).await);
        let id = app.add_accounts[0].id;
        assert!(shows(&app, id, "Loading...") && !shows(&app, id, "Signal logins may be slow."));
        let _ = send(&mut app, opened).await;
        assert!(bridges.requests().contains(&"GET /platform-status".to_string()));
        assert!(shows(&app, id, "Signal logins may be slow.") && shows(&app, id, "Continue →"));
        assert!(!shows(&app, id, "Loading..."), "a status that asks to confirm hides the login");
        let go = click(&app, id, "Continue →");
        let _ = send(&mut app, go).await;
        assert!(shows(&app, id, "Signal logins may be slow.") && shows(&app, id, "Loading..."));
        assert!(!shows(&app, id, "Continue →"));
        settle(&mut app, UiMessage::Window(crate::windows::Msg::Closed(id))).await;
        let opened = status(send(&mut app, UiMessage::OpenAddAccount("gmessages".into(), None)).await);
        let id = app.add_accounts[0].id;
        let _ = send(&mut app, opened).await;
        assert!(shows(&app, id, "Google Messages logins are paused.") && shows(&app, id, "Connect to Google Messages"));
        assert!(!shows(&app, id, "Loading...") && !shows(&app, id, "Continue →"), "a disabled platform has no login");
    }

    /// Desktop's `PhoneInput` in the Telegram phone step of the stub: the selector with the flag and the dial code,
    /// its dropdown with the keys of the window, a picked country, its mask, a pasted number that goes to its
    /// country, and the `+<dial code><digits>` that the form sends.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_phone_field_picks_formats_and_guesses_the_country() {
        use crate::phone::COUNTRIES;
        use iced::keyboard::{key::Named, Key};
        let (mut app, bridges) = stub_app("phone-field").await;
        let flows = one(send(&mut app, UiMessage::OpenAddAccount("telegram".into(), None)).await);
        let id = app.add_accounts[0].id;
        let started = one(send(&mut app, flows).await);
        let _ = send(&mut app, started).await;
        let started = one(send(&mut app, UiMessage::AddAccount(id, AddMsg::Flow("phone".into()))).await);
        let _ = send(&mut app, started).await;
        let field = "fi.mau.telegram.login.phone_number";
        let phone = |app: &BeeperGuiApp| {
            let a = &app.add_accounts[0];
            (a.values[field].clone(), a.phones[field].input.clone())
        };
        assert_eq!(phone(&app), (String::new(), String::new()), "the value stays empty until it changes");
        assert!(shows(&app, id, "Enter phone number"));
        let dial = format!("+{}", COUNTRIES[crate::phone::default_country()].dial);
        let toggle = click(&app, id, &dial);
        assert!(matches!(toggle, UiMessage::AddAccount(_, AddMsg::Phone(_, PhoneMsg::Toggle))));
        assert!(!shows(&app, id, "United Kingdom"), "the dropdown is closed");
        let _ = send(&mut app, toggle).await;
        assert!(shows(&app, id, "United Kingdom") && shows(&app, id, "+44"));
        // The window's keys go to the dropdown: Escape closes it and not the window.
        let escape = UiMessage::WindowKey(id, Key::Named(Named::Escape), Default::default());
        let _ = send(&mut app, escape).await;
        assert!(!shows(&app, id, "United Kingdom") && app.add_accounts.len() == 1);
        let m = click(&app, id, &dial);
        let _ = send(&mut app, m).await;
        for t in ["u", "n", "i", "t", "e", "d", " ", "k"] {
            let _ = send(&mut app, UiMessage::WindowText(id, t.into())).await;
        }
        let _ = send(&mut app, UiMessage::WindowKey(id, Key::Named(Named::Enter), Default::default())).await;
        assert_eq!(phone(&app), ("+44".to_string(), String::new()), "typing a name and Enter pick the country");
        assert!(!shows(&app, id, "United Kingdom") && shows(&app, id, "+44"));
        let m = click(&app, id, "+44");
        let _ = send(&mut app, m).await;
        // A press on the selector only toggles the dropdown, and a press on the input closes it like Desktop's blur.
        let msgs: Vec<UiMessage> = clicks(app.view(id), "+44");
        assert!(matches!(msgs.as_slice(), [UiMessage::AddAccount(_, AddMsg::Phone(_, PhoneMsg::Toggle))]));
        let mut sim = iced_test::simulator(app.view(id));
        sim.click(iced::widget::Id::from(phone::input_id(id, field))).expect("the input");
        assert!(sim.into_messages().any(|m| matches!(m, UiMessage::AddAccount(_, AddMsg::Phone(_, PhoneMsg::Blur)))));
        // The simulator does not run the task that scrolls the list to the selected country, so it shows the top.
        let pick = click(&app, id, "Afghanistan");
        assert!(matches!(pick, UiMessage::AddAccount(_, AddMsg::Phone(_, PhoneMsg::Pick(_)))));
        let _ = send(&mut app, pick).await;
        assert_eq!(phone(&app).0, "+93");
        let gb = COUNTRIES.iter().position(|c| c.iso2 == "gb").expect("the United Kingdom");
        let _ = send(&mut app, UiMessage::AddAccount(id, AddMsg::Phone(field.into(), PhoneMsg::Pick(gb)))).await;
        // Typing formats the number with the country's mask, and a letter does nothing.
        for c in "2079460958x".chars() {
            let text = format!("{}{c}", app.add_accounts[0].phones[field].input);
            let _ = send(&mut app, UiMessage::AddAccount(id, AddMsg::Phone(field.into(), PhoneMsg::Input(text)))).await;
        }
        assert_eq!(phone(&app), ("+442079460958".to_string(), "2079 460958".to_string()));
        assert!(shows(&app, id, "2079 460958") && !shows(&app, id, "Please enter a valid phone number"));
        // A pasted number with a dial code goes to its country.
        let paste = PhoneMsg::Paste("+1 201-555-0123".into());
        let _ = send(&mut app, UiMessage::AddAccount(id, AddMsg::Phone(field.into(), paste))).await;
        assert_eq!(phone(&app), ("+12015550123".to_string(), "(201) 555-0123".to_string()));
        assert_eq!(COUNTRIES[app.add_accounts[0].phones[field].country].iso2, "us");
        assert!(shows(&app, id, "(201) 555-0123") && shows(&app, id, "+1"));
        let sent = {
            let m = click(&app, id, "Continue");
            one(send(&mut app, m).await)
        };
        let _ = send(&mut app, sent).await;
        assert!(shows(&app, id, "Please enter the code sent to your phone"));
        let sent = serde_json::json!({ field: "+12015550123" });
        assert_eq!(bridges.bodies().last(), Some(&sent), "the form sends Desktop's `phone`");
    }
}
