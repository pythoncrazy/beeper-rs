//! Desktop's `LoginBrowserWindow` with the `BridgeV2AuthComponent` of a Beeper Cloud bridge: it adds an account of
//! a network, or logs an account in again. The bridge describes each step (see `beeper_core::bridge_login`).
use crate::i18n::{tr, trf};
use crate::style::{brand_svg, btn_style, fill_box, ic, icon, input_style, med, semi, shadow_box, txt, CLEAR};
use crate::{menu_item, BeeperGuiApp, El, Toast, UiMessage, ERROR_CIRCLE};
use beeper_core::bridge_login::{self, Field, Flow, Step};
use beeper_core::config::{cloud_bridge, find_network};
use beeper_core::matrix::Provision;
use beeper_core::models::NetworkSpec;
use iced::widget::{
    button, column, container, image, mouse_area, pick_list, rich_text, row, scrollable, span, stack, svg, text_input,
    Column,
};
use iced::{window, Center, Color, Fill, Padding, Size, Task};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;

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

/// A 24px stroke icon like the ones of Desktop's icon set.
fn stroke(paths: &str) -> svg::Handle {
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">{paths}</svg>"#
    );
    svg::Handle::from_memory(svg.into_bytes())
}

static COPY: LazyLock<svg::Handle> = LazyLock::new(|| {
    stroke(
        r#"<path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/><rect x="9" y="9" width="13" height="13" rx="2"/>"#,
    )
});
static EYE: LazyLock<svg::Handle> = LazyLock::new(|| {
    stroke(r#"<path d="M2 12s3.6-7 10-7 10 7 10 7-3.6 7-10 7S2 12 2 12z"/><circle cx="12" cy="12" r="3"/>"#)
});
static EYE_OFF: LazyLock<svg::Handle> = LazyLock::new(|| {
    stroke(concat!(
        r#"<path d="M9.9 4.2A10 10 0 0 1 12 4c6.4 0 10 8 10 8a17 17 0 0 1-2.2 3.2M6.6 6.6A17 17 0 0 0 2 12s3.6 8 10 8"#,
        r#"a9.7 9.7 0 0 0 5.4-1.6"/><path d="M2 2l20 20"/><path d="M14.1 14.1a3 3 0 1 1-4.2-4.2"/>"#
    ))
});
static CLEAR_FIELD: LazyLock<svg::Handle> = LazyLock::new(|| stroke(r#"<path d="M18 6 6 18M6 6l12 12"/>"#));
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
    shown: BTreeSet<String>,
    qr: Option<svg::Handle>,
    emoji: Option<image::Handle>,
    /// Counts the login processes, so that the replies of an earlier process are dropped.
    run: u32,
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
    Close,
}

/// Desktop's `di`: the validation error of a field value.
fn invalid(f: &Field, v: &str) -> Option<String> {
    if v.trim().is_empty() {
        return Some(tr("This field is required").into());
    }
    let digits = v.chars().filter(char::is_ascii_digit).count();
    let bad = match f.kind.as_str() {
        "phone_number" => (!(7..=15).contains(&digits)
            || !v.chars().all(|c| c.is_ascii_digit() || " +-().".contains(c)))
        .then_some("Please enter a valid phone number"),
        "domain" => (!DOMAIN.is_match(v)).then_some("Please enter a valid domain (e.g., example.com)"),
        "email" => (!v.split_once('@').is_some_and(|(a, b)| !a.is_empty() && b.contains('.')))
            .then_some("Please enter a valid email address"),
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
        _ if min > 0 && n < min => {
            return Some(chars("Must be at least %d character", "Must be at least %d characters", min))
        }
        _ if max > 0 && n > max => {
            return Some(chars("Must be at most %d character", "Must be at most %d characters", max))
        }
        _ => {}
    }
    // Desktop ignores a pattern that does not compile.
    let pattern = regex::Regex::new(&f.pattern).ok().filter(|_| !f.pattern.is_empty());
    pattern.filter(|p| !p.is_match(v)).map(|_| tr("Invalid format").into())
}

/// The QR code of a `display_and_wait` step as an SVG, like Desktop's `QRCodeSVG` with level `M`.
fn qr_svg(data: &str) -> svg::Handle {
    use qrcodegen::{QrCode, QrCodeEcc};
    let code = QrCode::encode_text(data, QrCodeEcc::Medium).expect("the QR data fits in a QR code");
    let n = code.size();
    let mut path = String::new();
    for y in 0..n {
        for x in (0..n).filter(|&x| code.get_module(x, y)) {
            path.push_str(&format!("M{x},{y}h1v1h-1z"));
        }
    }
    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {n} {n}" shape-rendering="crispEdges"><path fill="#fff" d="M0,0h{n}v{n}H0z"/><path fill="#000" d="{path}"/></svg>"##
    );
    svg::Handle::from_memory(svg.into_bytes())
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
            st.bridge_accounts.iter().find(|a| a.bridge_id == bridge && a.login_id == *l).and_then(|a| a.display_text())
        });
        let (id, open) = window::open(window::Settings {
            resizable: false,
            min_size: None,
            position: window::Position::Centered,
            ..Self::window_settings(SIZE)
        });
        self.add_accounts.push(AddAccount {
            id,
            network,
            bridge,
            login: login.unwrap_or_default(),
            who,
            flows: Vec::new(),
            flow: String::new(),
            process: None,
            step: None,
            error: None,
            failed: None,
            busy: true,
            menu: false,
            values: BTreeMap::new(),
            touched: BTreeSet::new(),
            shown: BTreeSet::new(),
            qr: None,
            emoji: None,
            run: 0,
        });
        let p = self.add_provision(self.add_accounts.len() - 1);
        let flows = bridge_login::flows(p, self.cloud.api_url.clone());
        let flows = Task::perform(flows, move |r| UiMessage::AddAccount(id, AddMsg::Flows(r)));
        Task::batch([open.discard(), flows])
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
        let cancel = self.add_accounts[i]
            .process
            .take()
            .map(|process| Task::perform(bridge_login::cancel(p.clone(), process), |_| UiMessage::Noop));
        let a = &mut self.add_accounts[i];
        a.run += 1;
        (a.flow, a.step, a.error, a.failed, a.busy, a.menu, a.qr, a.emoji) =
            (flow.clone(), None, None, None, true, false, None, None);
        let (id, run) = (a.id, a.run);
        let start =
            Task::perform(bridge_login::start(p, flow), move |r| UiMessage::AddAccount(id, AddMsg::Started(run, r)));
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
            a.process = None;
            a.step = Some(step);
            self.show(toast);
            let refresh = self.refresh_cloud_accounts();
            return if show { refresh } else { Task::batch([refresh, window::close(id)]) };
        }
        a.values =
            step.user_input.iter().flat_map(|u| &u.fields).map(|f| (f.id.clone(), f.default_value.clone())).collect();
        (a.touched, a.shown) = (BTreeSet::new(), BTreeSet::new());
        let display = step.display_and_wait.clone();
        a.qr = display.as_ref().filter(|d| d.kind == "qr").map(|d| qr_svg(&d.data));
        a.emoji = None;
        let process = a.process.clone().expect("a running login process");
        a.step = Some(step.clone());
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
            AddMsg::Started(r, _) | AddMsg::Next(r, _) | AddMsg::Emoji(r, _) if r != run => {}
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
            AddMsg::Field(f, v) => {
                a.touched.insert(f.clone());
                a.values.insert(f, v);
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
            AddMsg::Copy(code) => return iced::clipboard::write(code),
            AddMsg::Close => return window::close(id),
        }
        Task::none()
    }

    pub fn add_account_title(&self, id: window::Id) -> Option<String> {
        let a = self.add_accounts.iter().find(|a| a.id == id)?;
        let text = if a.login.is_empty() { "Add %s" } else { "Reconnect %s" };
        Some(text.replace("%s", a.network.display_name))
    }

    /// Desktop's `LoginWindow` with the `subview-add-account` of the window `a`.
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
        let error_line = |e: &'a str| -> El<'a> {
            row![svg(ERROR_CIRCLE.clone()).width(16).height(16), txt(e, 13.0).color(p.danger)]
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
        let instructions = |s: &'a Step| (!s.instructions.is_empty()).then(|| para(s.instructions.clone()));
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
                "user_input" => self.view_add_form(a, s),
                "cookies" => {
                    let text =
                        trf("Click the button below to open the browser and begin authentication with %s.", &[&name]);
                    // beeper-rs cannot embed Desktop's isolated browser window that reads the cookies.
                    column![para(text), primary(trf("Open %s Login", &[&name]), None)].spacing(16).into()
                }
                _ => {
                    let done = primary(tr("Done").into(), Some(AddMsg::Close));
                    column![para(s.instructions.clone()), done].spacing(16).into()
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
                let plate =
                    container(svg(qr).width(QR_SIZE).height(QR_SIZE)).padding(8).style(fill_box(Color::WHITE, 12.0));
                let logo = container(svg(brand_svg(a.network.bridge_id)).width(20).height(20))
                    .padding(2)
                    .style(fill_box(Color::WHITE, 6.0));
                stack![plate, container(logo).center(QR_SIZE + 16.0)].into()
            }
            None => svg(brand_svg(a.network.bridge_id)).width(52).height(52).into(),
        };
        let failed = a.failed.as_deref().map(error_line);
        let main = column![mark, semi(header, 17.0).color(p.strong).center(), body]
            .push(failed)
            .spacing(24)
            .align_x(Center)
            .padding([24, 32]);
        let limits = CLOUD_LIMITS.iter().find(|l| l.0 == a.network.bridge_id).map(|l| {
            row![txt(tr("Not supported:"), 12.0).color(p.strong), txt(tr(l.1), 12.0).color(p.sub).width(Fill)]
                .spacing(4)
        });
        let about = rich_text![
            span(tr("Your login information and chats will be synced across devices and data is stored encrypted on Beeper servers."))
                .color(p.sub),
            span(" "),
            span(tr("Learn more")).color(p.accent).link(UiMessage::OpenUrl(LEARN_MORE.into())),
        ]
        .on_link_click(|m: UiMessage| m)
        .size(12)
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
                let label = label.unwrap_or_else(|| if f.name.is_empty() { f.id.clone() } else { f.name.clone() });
                menu_item(p, None, label, "", (!a.busy).then(|| go(AddMsg::Flow(f.id.clone()))))
            });
            let list = container(Column::with_children(items).padding([4, 0])).width(200);
            let list = list.style(shadow_box(p.menu, p.menu_border, 6.0, 16.0));
            mouse_area(container(list).align_right(Fill).height(Fill).padding(Padding::ZERO.top(30).right(8)))
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
            let secret = matches!(f.kind.as_str(), "password" | "token");
            let shown = a.shown.contains(&f.id);
            let id = f.id.clone();
            let control: El<'a> = match f.kind.as_str() {
                "select" if !f.options.is_empty() => {
                    pick_list(f.options.as_slice(), Some(value.to_string()).filter(|v| !v.is_empty()), move |v| {
                        go(AddMsg::Field(id.clone(), v))
                    })
                    .placeholder(trf("Select %s...", &[&f.name]))
                    .width(Fill)
                    .into()
                }
                _ => {
                    let hint = match f.kind.as_str() {
                        "phone_number" => tr("Enter phone number").to_string(),
                        _ if secret => "••••••••".into(),
                        _ => f.name.clone(),
                    };
                    let input = text_input(&hint, value)
                        .on_input(move |v| go(AddMsg::Field(id.clone(), v)))
                        .on_submit(go(AddMsg::Submit))
                        .secure(secret && !shown)
                        .size(14)
                        .padding([8, 10])
                        .style(input_style(p.input, p.strong, p.tertiary, p.accent, 8.0));
                    let accessory = |handle: svg::Handle, msg: AddMsg| {
                        button(svg(handle).width(16).height(16))
                            .padding(4)
                            .style(btn_style([CLEAR, p.hover, p.hover], [p.strong; 3], 6.0))
                            .on_press(go(msg))
                    };
                    let eye = secret.then(|| {
                        accessory(if shown { EYE_OFF.clone() } else { EYE.clone() }, AddMsg::Reveal(f.id.clone()))
                    });
                    let clear = (!secret && f.kind != "phone_number" && !value.is_empty())
                        .then(|| accessory(CLEAR_FIELD.clone(), AddMsg::Field(f.id.clone(), String::new())));
                    row![input].push(eye).push(clear).spacing(4).align_y(Center).into()
                }
            };
            let error = a.touched.contains(&f.id).then(|| invalid(f, value)).flatten();
            column![semi(f.name.as_str(), 13.0).color(p.strong), control]
                .push(error.map(|e| txt(e, 12.0).color(p.danger)))
                .push(
                    (!f.description.is_empty())
                        .then(|| txt(f.description.as_str(), 12.0).color(p.strong.scale_alpha(0.7))),
                )
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
            .then(|| txt(s.instructions.as_str(), 14.0).color(p.strong).center().width(Fill));
        let start_over = a.error.as_deref().map(|e| {
            column![
                txt(tr("Something went wrong and we couldn't add your account."), 13.0).color(p.strong),
                txt(e, 13.0).color(p.danger),
                button(med(tr("Start Over"), 13.0)).on_press_maybe((!a.busy).then(|| go(AddMsg::StartOver))),
            ]
            .spacing(8)
            .align_x(Center)
        });
        column![]
            .push(intro)
            .extend(rows)
            .push(submit)
            .push(back)
            .push(start_over)
            .spacing(16)
            .align_x(Center)
            .width(Fill)
            .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_validate_like_desktop() {
        let field = |kind: &str| Field { kind: kind.into(), ..Field::default() };
        assert_eq!(invalid(&field("username"), " ").as_deref(), Some("This field is required"));
        assert_eq!(invalid(&field("phone_number"), "+1 555 0100").as_deref(), None);
        assert_eq!(invalid(&field("phone_number"), "12").as_deref(), Some("Please enter a valid phone number"));
        assert_eq!(invalid(&field("email"), "a@b").as_deref(), Some("Please enter a valid email address"));
        assert_eq!(invalid(&field("domain"), "bsky.social").as_deref(), None);
        assert_eq!(invalid(&field("url"), "x.com").as_deref(), Some("Please enter a valid URL"));
        let code = Field { min_length: 5, max_length: 5, ..field("2fa_code") };
        assert_eq!(invalid(&code, "123").as_deref(), Some("Must be exactly 5 characters"));
        let pin = Field { pattern: "^[0-9]+$".into(), ..field("2fa_code") };
        assert_eq!(invalid(&pin, "12a").as_deref(), Some("Invalid format"));
        assert_eq!(invalid(&Field { pattern: "(".into(), ..field("text") }, "x"), None, "a bad pattern is ignored");
    }

    #[test]
    fn every_addable_network_has_a_cloud_bridge_but_bluesky() {
        let none: Vec<_> = crate::login::addable().filter(|n| cloud_bridge(n).is_none()).map(|n| n.bridge_id).collect();
        assert_eq!(none, ["bluesky"]);
        assert_eq!(cloud_bridge(find_network("slackgo").unwrap()), Some("slackgo"));
    }

    /// A signed-in app whose homeserver and Beeper API are the stub bridges of `beeper_core::bridge_stub`.
    async fn stub_app(name: &str) -> (BeeperGuiApp, beeper_core::bridge_stub::Bridges) {
        use beeper_core::models::{MatrixSession, Profile};
        let (url, bridges) = beeper_core::bridge_stub::homeserver().await;
        let dir = std::env::temp_dir().join(format!("beeper-rs-add-account-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let config = beeper_core::config::CentralConfig::new(dir, "127.0.0.1".into(), 23631, None);
        let store = beeper_core::store::StoreLock::shared(beeper_core::store::BeeperStore::from_config(config));
        let session =
            MatrixSession { user_id: "@me:beeper.com".into(), access_token: "t".into(), device_id: "D".into() };
        store.write().sign_in(session, url.clone(), &Profile::default(), Vec::new());
        let mut app = BeeperGuiApp::boot(store).0;
        app.cloud = std::sync::Arc::new(beeper_core::cloud::Cloud::new(&url, &url));
        (app, bridges)
    }

    /// The messages that a task outputs. The window and clipboard actions of the task do nothing here.
    async fn run(task: Task<UiMessage>) -> Vec<UiMessage> {
        use iced::futures::StreamExt;
        let Some(stream) = iced_runtime::task::into_stream(task) else { return Vec::new() };
        let out = stream.filter_map(|a| async move {
            match a {
                iced_runtime::Action::Output(m) => Some(m),
                _ => None,
            }
        });
        out.collect().await
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

    /// The one message of the Add Account window among the messages of a task.
    fn one(msgs: Vec<UiMessage>) -> UiMessage {
        let mut ours: Vec<_> = msgs.into_iter().filter(|m| matches!(m, UiMessage::AddAccount(..))).collect();
        assert_eq!(ours.len(), 1, "one message of the window");
        ours.remove(0)
    }

    /// The Telegram flows of the stub: the QR code of the preferred flow, the phone flow of "Login options", a
    /// wrong phone number, a wrong code, and the cancel of the login when the window closes.
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
        assert!(shows(&app, id, "Scan the QR code on your phone to log in"));
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
        let _ = send(&mut app, UiMessage::AddAccount(id, AddMsg::Field(field.clone(), "12".into()))).await;
        assert!(shows(&app, id, "Please enter a valid phone number"));
        assert!(send(&mut app, UiMessage::AddAccount(id, AddMsg::Submit)).await.is_empty(), "a bad form is not sent");
        let _ = send(&mut app, UiMessage::AddAccount(id, AddMsg::Field(field, "15550100".into()))).await;
        let failed = {
            let m = click(&app, id, "Continue");
            one(send(&mut app, m).await)
        };
        let _ = send(&mut app, failed).await;
        assert!(shows(&app, id, "Something went wrong and we couldn't add your account."));
        assert!(shows(&app, id, "Phone number must be in international format"));
        settle(&mut app, UiMessage::AddAccount(id, AddMsg::StartOver)).await;
        let field = "fi.mau.telegram.login.phone_number".to_string();
        let _ = send(&mut app, UiMessage::AddAccount(id, AddMsg::Field(field, "+15550100".into()))).await;
        settle(&mut app, UiMessage::AddAccount(id, AddMsg::Submit)).await;
        assert!(shows(&app, id, "Please enter the code sent to your phone") && shows(&app, id, "Back"));
        let code = "fi.mau.telegram.login.code".to_string();
        let wrong = beeper_core::bridge_stub::WRONG_CODE.to_string();
        let _ = send(&mut app, UiMessage::AddAccount(id, AddMsg::Field(code, wrong))).await;
        settle(&mut app, UiMessage::AddAccount(id, AddMsg::Submit)).await;
        assert!(shows(&app, id, "Invalid phone code"));
        let process = app.add_accounts[0].process.clone().expect("a running login");
        settle(&mut app, UiMessage::WindowClosed(id)).await;
        assert!(app.add_accounts.is_empty());
        let cancel = format!(
            "POST /_matrix/client/unstable/com.beeper.bridge/telegram/_matrix/provision/v3/login/cancel/{process}"
        );
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
        settle(&mut app, UiMessage::WindowClosed(id)).await;
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
        let _ =
            send(&mut app, UiMessage::AddAccount(id, AddMsg::Field("phone_number".into(), "+15550100".into()))).await;
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
        let _ =
            send(&mut app, UiMessage::AddAccount(id, AddMsg::Field("phone_number".into(), "+15550100".into()))).await;
        settle(&mut app, UiMessage::AddAccount(id, AddMsg::Submit)).await;
        assert_eq!(app.toast.as_ref().map(|t| t.text.as_str()), Some("Reconnected to WhatsApp"));
    }

    /// Desktop's `network-status` banner, the Reconnect button of the account page and the Reconnect item of the
    /// account menu, for a cloud account whose bridge asks for a new login.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_logged_out_account_offers_reconnect() {
        let (mut app, _) = stub_app("reconnect").await;
        let out = serde_json::json!({ "state_event": "BAD_CREDENTIALS", "has_error": true, "message": "Logged out" });
        let bridges = serde_json::json!({ "whatsapp": { "bridgeState": { "bridgeType": "whatsapp" }, "remoteState": { "15550100": out } } });
        let accounts = beeper_core::cloud::cloud_accounts(&bridges).expect("valid bridge states");
        app.store.write().set_cloud_accounts(accounts);
        let again =
            |m: UiMessage| matches!(m, UiMessage::OpenAddAccount(n, Some(l)) if n == "whatsapp" && l == "15550100");
        assert!(iced_test::simulator(app.view_sidebar()).find("WhatsApp disconnected").is_err(), "only in its space");
        app.selected_account_filter = Some("whatsapp".into());
        let mut sim = iced_test::simulator(app.view_sidebar());
        assert!(sim.find("WhatsApp disconnected").is_ok());
        sim.click("Reconnect").expect("the Reconnect of the banner");
        assert!(sim.into_messages().any(again));
        let _ = app.update(UiMessage::OpenSettings(crate::SettingsTab::Accounts));
        let _ = app.update(UiMessage::OpenAccount(Some("whatsapp".into())));
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

    /// The X cookie step of the stub: beeper-rs cannot open Desktop's browser window that reads the cookies.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_cookie_login_shows_the_browser_step() {
        let (mut app, _) = stub_app("twitter").await;
        settle(&mut app, UiMessage::OpenAddAccount("twitter".into(), None)).await;
        let id = app.add_accounts[0].id;
        assert!(shows(&app, id, "Click the button below to open the browser and begin authentication with X."));
        assert!(shows(&app, id, "Open X Login"));
        assert_eq!(app.add_accounts[0].other_flows().count(), 0, "X has one flow, so no Login options");
    }
}
