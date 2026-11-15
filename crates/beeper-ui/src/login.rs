use crate::i18n::{tr, trf, trn, trx};
use crate::style::{
    bold, bordered_box, brand_svg, btn_style, fill_box, ic, icon, input_style, med, semi, shadow_box, txt, Palette,
    CLEAR,
};
use crate::{BeeperGuiApp, El, UiMessage};
use beeper_core::cloud::{Cloud, CloudError, CodeResult};
use beeper_core::config::{account_network, NETWORK_SPECS};
use beeper_core::import::{read_beeper_data, BeeperData};
use beeper_core::matrix::{DeviceInfo, E2eeSetup};
use beeper_core::models::{BridgeAccountInfo, MatrixSession, Profile};
use beeper_core::store::{BeeperStore, Setup, NOTE_CHAT_ID};
use iced::gradient::Linear;
use iced::widget::{
    button, column, container, mouse_area, operation, rich_text, row, scrollable, span, stack, svg, text_editor,
    text_input, tooltip, Space,
};
use iced::{border, window, Background, Center, Color, Degrees, Fill, Gradient, Padding, Size, Task, Theme};
use std::path::PathBuf;
use std::sync::{Arc, LazyLock};

const HEADING: Color = Color::from_rgb8(132, 100, 238);
const EMAIL_INPUT: &str = "login-email";
const CODE_INPUT: &str = "login-code";
const USERNAME_INPUT: &str = "login-username";
const NO_EMAIL_LOGIN: &str = "The server does not support email-based login. Try logging in with a username instead.";

const WELCOME_ART: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 314 396"><defs><linearGradient id="b" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#8f6cf6"/><stop offset="1" stop-color="#2f5cf8"/></linearGradient></defs><ellipse cx="157" cy="330" rx="66" ry="7" fill="#e3e3e6"/><g transform="rotate(-9 157 190)"><path d="M77 120h160a28 28 0 0 1 28 28v76a28 28 0 0 1-28 28h-96l-38 30v-30h-26a28 28 0 0 1-28-28v-76a28 28 0 0 1 28-28z" fill="url(#b)"/><circle cx="117" cy="186" r="13" fill="#fff"/><circle cx="157" cy="186" r="13" fill="#fff"/><circle cx="197" cy="186" r="13" fill="#fff" opacity=".6"/></g><rect x="22" y="62" width="40" height="40" rx="12" fill="#34c759" transform="rotate(-12 42 82)"/><rect x="250" y="46" width="34" height="34" rx="10" fill="#ff8a3d" transform="rotate(14 267 63)"/><rect x="262" y="262" width="30" height="30" rx="9" fill="#ff4f9a" transform="rotate(-10 277 277)"/><circle cx="40" cy="268" r="17" fill="#ffd23f"/><circle cx="34" cy="263" r="2.6" fill="#3a2e00"/><circle cx="46" cy="263" r="2.6" fill="#3a2e00"/><path d="M32 272q8 7 16 0" stroke="#3a2e00" stroke-width="2.4" fill="none" stroke-linecap="round"/><path d="M226 92l5 13 13 5-13 5-5 13-5-13-13-5 13-5z" fill="#ffd23f"/><path d="M92 300l3 8 8 3-8 3-3 8-3-8-8-3 8-3z" fill="#ffd23f"/><circle cx="136" cy="44" r="7" fill="#9b7cf9"/><circle cx="292" cy="170" r="6" fill="#2f5cf8"/></svg>"##;

const EMAIL_ART: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 314 314"><ellipse cx="157" cy="262" rx="70" ry="6" fill="#e3e3e6"/><g transform="rotate(-8 160 170)"><rect x="70" y="110" width="180" height="120" rx="14" fill="#f3f3f5" stroke="#c9c9ce" stroke-width="2"/><path d="M74 118l86 66 86-66" fill="none" stroke="#c9c9ce" stroke-width="2" stroke-linejoin="round"/><path d="M74 224l64-50M246 224l-64-50" stroke="#dcdce0" stroke-width="2"/></g><path d="M18 74l112-22-62 74-12-34z" fill="#4f8df8"/><path d="M56 92l74-40-50 60z" fill="#2f6cf2"/><circle cx="62" cy="232" r="28" fill="#2ecc5a"/><path d="M36 228h52M62 204v56M44 212q18 18 36 0M44 248q18-18 36 0" stroke="#1f9e43" stroke-width="2" fill="none"/><circle cx="262" cy="196" r="34" fill="none" stroke="#8463ee" stroke-width="14"/><circle cx="262" cy="196" r="10" fill="#8463ee"/><path d="M236 52l4 11 11 4-11 4-4 11-4-11-11-4 11-4z" fill="#ffd23f"/><path d="M44 152l4 10 10 4-10 4-4 10-4-10-10-4 10-4z" fill="#ffd23f"/><path d="M196 252l3 8 8 3-8 3-3 8-3-8-8-3 8-3z" fill="#ffd23f"/></svg>"##;

const CODE_ART: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 314 240"><defs><linearGradient id="l" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#8b6cf3"/><stop offset="1" stop-color="#5b43d6"/></linearGradient></defs><path d="M127 104V82a30 30 0 0 1 60 0v22" fill="none" stroke="#9d86f5" stroke-width="16" stroke-linecap="round"/><rect x="104" y="98" width="106" height="98" rx="20" fill="url(#l)"/><circle cx="157" cy="138" r="13" fill="#fff"/><path d="M151 146h12l4 26h-20z" fill="#fff"/><circle cx="28" cy="118" r="10" fill="#2ecc5a"/><circle cx="66" cy="196" r="9" fill="#2ecc5a"/><circle cx="246" cy="60" r="10" fill="#2ecc5a"/><circle cx="226" cy="190" r="8" fill="#2ecc5a"/><circle cx="296" cy="140" r="9" fill="#2ecc5a"/><circle cx="96" cy="84" r="8" fill="#2ecc5a"/></svg>"##;

const GOOGLE_G: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 48 48"><path fill="#FFC107" d="M43.6 20.5H42V20H24v8h11.3C33.7 32.7 29.2 36 24 36c-6.6 0-12-5.4-12-12s5.4-12 12-12c3.1 0 5.8 1.2 7.9 3.1l5.7-5.7C34 6.1 29.3 4 24 4 13 4 4 13 4 24s9 20 20 20 20-9 20-20c0-1.3-.1-2.4-.4-3.5z"/><path fill="#FF3D00" d="M6.3 14.7l6.6 4.8C14.7 15.1 19 12 24 12c3.1 0 5.8 1.2 7.9 3.1l5.7-5.7C34 6.1 29.3 4 24 4 16.3 4 9.7 8.3 6.3 14.7z"/><path fill="#4CAF50" d="M24 44c5.2 0 9.9-2 13.4-5.2l-6.2-5.2C29.2 35.1 26.7 36 24 36c-5.2 0-9.6-3.3-11.3-7.9l-6.5 5C9.5 39.6 16.2 44 24 44z"/><path fill="#1976D2" d="M43.6 20.5H42V20H24v8h11.3c-.8 2.2-2.2 4.2-4.1 5.6l6.2 5.2C37 39.2 44 34 44 24c0-1.3-.1-2.4-.4-3.5z"/></svg>"##;

static ART: LazyLock<[svg::Handle; 4]> =
    LazyLock::new(|| [WELCOME_ART, EMAIL_ART, CODE_ART, GOOGLE_G].map(|s| svg::Handle::from_memory(s.as_bytes())));

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Screen {
    Import,
    #[default]
    Welcome,
    Email,
    Code,
    Username,
    /// Desktop's `AwaitVerification`, `AwaitRecoveryCode` and `IndicateSyncing`.
    Verify,
    RecoveryKey,
    Syncing,
    /// Desktop's `RecoveryCodePreamble`, `AwaitNewRecoveryCode` (which shows the new key, then asks for it again) and
    /// `ConfirmResetRecoveryCode`.
    Preamble,
    NewKey,
    ConfirmKey,
    Reset,
    Accounts,
}

#[derive(Default)]
pub struct Login {
    pub screen: Screen,
    pub email: String,
    pub code: String,
    pub username: String,
    pub error: Option<String>,
    request: String,
    lead_token: String,
    pub busy: bool,
    menu: bool,
    query: String,
    source: Option<PathBuf>,
    import: Option<Arc<BeeperData>>,
    /// The other devices that can verify this device, "Try another way" was pressed, and "They match" was pressed.
    pub devices: Vec<DeviceInfo>,
    pub another_way: bool,
    pub waiting: bool,
    /// Desktop's `RecoveryCodeInput`, and the keys that were tried, which a paste on focus does not submit again.
    pub key_input: text_editor::Content,
    pub tried_keys: Vec<String>,
    /// The new recovery key, and whether "Copy to Clipboard" copied it.
    pub new_key: Option<String>,
    pub copied: bool,
}

impl Login {
    /// The screens before the sign-in ends.
    pub fn signing_in(&self) -> bool {
        matches!(self.screen, Screen::Import | Screen::Welcome | Screen::Email | Screen::Code | Screen::Username)
    }

    pub fn new(source: Option<PathBuf>) -> Self {
        Login { screen: if source.is_some() { Screen::Import } else { Screen::Welcome }, source, ..Login::default() }
    }
}

#[derive(Clone)]
pub enum Outcome {
    Requested(String),
    Lead(String, Vec<String>),
    Session(MatrixSession, Profile, Vec<BridgeAccountInfo>),
    Read(Arc<BeeperData>),
    Imported(Arc<BeeperData>, MatrixSession, Profile, Vec<BridgeAccountInfo>),
}

#[derive(Clone)]
pub enum LoginMsg {
    Import,
    Fresh,
    Show(Screen),
    Google,
    Email(String),
    SubmitEmail,
    Code(String),
    Resend,
    Username(String),
    SubmitUsername,
    Menu,
    StartOver,
    Query(String),
    Continue,
    Done(Result<Outcome, String>),
    E2ee(Result<E2eeSetup, String>),
    AnotherWay,
    /// "Enter Recovery Key", which cancels the verification.
    UseRecoveryKey,
    KeyEdit(text_editor::Action),
    /// "Paste from Clipboard", and the text of the clipboard.
    PasteKey,
    Pasted(Option<String>),
    /// The window got the focus: Desktop submits a valid recovery key from the clipboard.
    KeyOnFocus(Option<String>),
    SubmitRecoveryKey,
    Recovered(Result<(), String>),
    /// "Save Key", and "Reset Recovery Key": a new recovery key.
    NewKey,
    MadeKey(Result<String, String>),
    CopyKey,
    /// "Next" under the new key, which asks for it again.
    KeyNext,
    SubmitConfirmKey,
    /// "Lost your recovery key?", which cancels the verification.
    LostKey,
}

impl From<LoginMsg> for UiMessage {
    fn from(m: LoginMsg) -> Self {
        UiMessage::Login(m)
    }
}

fn previous(s: Screen) -> Option<Screen> {
    match s {
        Screen::Email => Some(Screen::Welcome),
        Screen::Code => Some(Screen::Email),
        Screen::RecoveryKey | Screen::Reset => Some(Screen::Verify),
        Screen::ConfirmKey => Some(Screen::NewKey),
        Screen::Import
        | Screen::Welcome
        | Screen::Username
        | Screen::Verify
        | Screen::Syncing
        | Screen::Preamble
        | Screen::NewKey
        | Screen::Accounts => None,
    }
}

fn session(cloud: &Cloud, jwt: &str) -> Result<Outcome, CloudError> {
    cloud.finish(jwt).map(|(s, p, a)| Outcome::Session(s, p, a))
}

pub const FREE_ACCOUNTS: usize = 10;

/// The networks that you can add an account on here.
pub fn addable() -> impl Iterator<Item = &'static beeper_core::models::NetworkSpec> {
    NETWORK_SPECS[1..].iter().filter(|n| cfg!(target_os = "macos") || n.bridge_id != "imessage")
}

/// The Connect grid. A tile opens the Add Account window of its network (Desktop's `wj`) when beeper-rs can add
/// an account on it. A demo account adds none.
pub fn network_grid(st: &BeeperStore, query: &str, p: &Palette, ink: Color) -> El<'static> {
    let (accounts, features) = (&st.bridge_accounts, &st.features);
    let q = query.trim().to_lowercase();
    let tiles: Vec<El<'static>> = addable()
        .filter(|n| n.display_name.to_lowercase().contains(&q) || n.bridge_id.contains(&q))
        .map(|n| {
            let count = accounts.iter().filter(|a| account_network(&a.bridge_id).bridge_id == n.bridge_id).count();
            let dot = |_| {
                container(Space::new())
                    .width(12)
                    .height(12)
                    .style(fill_box(Color::from_rgb8(n.brand_rgb[0], n.brand_rgb[1], n.brand_rgb[2]), 6.0))
                    .into()
            };
            let chip = (count > 0).then(|| {
                container(row((0..count.min(4)).map(dot)).spacing(2)).padding(2).style(fill_box(p.chip, 999.0))
            });
            let tile = column![
                svg(brand_svg(n.bridge_id)).width(36).height(36),
                semi(n.display_name, 11.0).color(ink).center().width(Fill)
            ]
            .push(chip)
            .spacing(10)
            .align_x(Center)
            .padding(8);
            // Desktop's tile tooltip (`App:132302`): beeper-rs adds accounts on the Beeper Cloud bridges only.
            let bridge = beeper_core::config::cloud_bridge(n);
            let open = !st.config.demo && bridge.is_some_and(|b| count == 0 || features.multi_account(b));
            let tip = match (bridge, count, open) {
                (None, ..) => trf("Can't add %s", &[&n.display_name]),
                (_, 0, _) => trf("Connect %s", &[&n.display_name]),
                (_, _, true) => tr("Add another").into(),
                (_, _, false) => trf("You can’t add another %s account", &[&n.display_name]),
            };
            let tile = button(tile)
                .padding(0)
                .style(btn_style([CLEAR; 3], [ink; 3], 8.0))
                .on_press_maybe(open.then(|| UiMessage::OpenAddAccount(n.bridge_id.into(), None)));
            let tip = container(txt(tip, 13.0).color(p.strong))
                .padding([6, 12])
                .style(shadow_box(p.elevated, p.chip, 16.0, 10.0));
            tooltip(tile, tip, tooltip::Position::Bottom).into()
        })
        .collect();
    if tiles.is_empty() {
        return container(txt(tr("No networks found"), 13.0).color(p.on_secondary)).center_x(Fill).padding(24).into();
    }
    let mut rows = Vec::new();
    let mut tiles = tiles.into_iter().peekable();
    while tiles.peek().is_some() {
        let cells: Vec<El<'static>> = tiles.by_ref().take(4).map(|t| container(t).width(Fill).into()).collect();
        let pad = (cells.len()..4).map(|_| Space::new().width(Fill).into());
        rows.push(row(cells.into_iter().chain(pad)).into());
    }
    column(rows).spacing(24).padding(8).into()
}

fn card_style(bg: Color) -> impl Fn(&Theme) -> container::Style {
    let shadow = iced::Shadow {
        color: Color::from_rgba(0.0, 0.0, 0.0, 0.03),
        offset: iced::Vector::new(0.0, 2.0),
        blur_radius: 4.0,
    };
    move |_| container::Style { background: Some(bg.into()), border: border::rounded(12), shadow, ..Default::default() }
}

pub(crate) fn brand_gradient() -> Background {
    let g = Linear::new(Degrees(190.1))
        .add_stop(0.0, Color::from_rgb8(99, 83, 242))
        .add_stop(1.0, Color::from_rgb8(20, 82, 248));
    Background::Gradient(Gradient::Linear(g))
}

fn pill<'a>(p: &Palette, content: El<'a>, height: f32, bg: Option<Color>, msg: Option<UiMessage>) -> El<'a> {
    let fg = if bg.is_some() { p.strong } else { Color::WHITE };
    button(container(content).center_x(Fill).center_y(height))
        .width(Fill)
        .padding(0)
        .style(move |_: &Theme, s| button::Style {
            background: Some(bg.map_or_else(brand_gradient, Background::from)),
            text_color: if s == button::Status::Disabled { fg.scale_alpha(0.6) } else { fg },
            border: border::rounded(height / 2.0),
            ..Default::default()
        })
        .on_press_maybe(msg)
        .into()
}

pub(crate) fn actions(items: Vec<El<'_>>, bottom: f32) -> El<'_> {
    column(items).spacing(8).padding(Padding::new(0.0).left(18.0).right(18.0).bottom(bottom)).into()
}

fn label<'a>(s: &'a str, lead: Option<El<'a>>) -> El<'a> {
    row![].push(lead).push(med(s, 13.0)).spacing(6).align_y(Center).into()
}

pub(crate) fn heading<'a>(s: &'a str, size: f32) -> El<'a> {
    container(bold(s, size).color(HEADING).line_height(iced::widget::text::LineHeight::Absolute(40.0.into())).center())
        .center_x(Fill)
        .into()
}

pub(crate) fn art<'a>(i: usize, height: f32) -> El<'a> {
    svg(ART[i].clone()).width(Fill).height(height).into()
}

fn field<'a>(
    p: &Palette,
    id: &'static str,
    hint: &'a str,
    value: &'a str,
    on: fn(String) -> LoginMsg,
    submit: LoginMsg,
) -> El<'a> {
    text_input(hint, value)
        .id(id)
        .on_input(move |v| on(v).into())
        .on_submit(submit.into())
        .size(14)
        .padding([10, 12])
        .style(input_style(p.chip, p.strong, p.on_secondary, p.accent, 19.0))
        .into()
}

impl BeeperGuiApp {
    pub fn login_window_settings() -> window::Settings {
        window::Settings { resizable: false, min_size: None, ..Self::window_settings(Size::new(330.0, 620.0)) }
    }

    pub fn refresh_cloud_accounts(&self) -> Task<UiMessage> {
        let (cloud, token) = (self.cloud.clone(), self.store.read().beeper_state.access_token.clone());
        Task::perform(
            async move { tokio::task::spawn_blocking(move || cloud.whoami(&token)).await.expect("whoami worker thread") },
            |r| UiMessage::CloudAccounts(r.map_err(|e| e.message)),
        )
    }

    pub fn enter_app(&mut self) -> Task<UiMessage> {
        let login = self.login_window.take().expect("login window is open");
        self.select_chat(NOTE_CHAT_ID);
        let (id, open) = window::open(self.main_window_settings());
        self.main_window = Some(id);
        Task::batch([open.map(UiMessage::WindowOpened), window::close(login)])
    }

    fn background(&mut self, f: impl FnOnce(&Cloud) -> Result<Outcome, String> + Send + 'static) -> Task<UiMessage> {
        self.login.busy = true;
        self.login.error = None;
        let cloud = self.cloud.clone();
        Task::perform(
            async move { tokio::task::spawn_blocking(move || f(&cloud)).await.expect("login window worker thread") },
            |r| LoginMsg::Done(r).into(),
        )
    }

    fn cloud_task(
        &mut self,
        f: impl FnOnce(&Cloud) -> Result<Outcome, CloudError> + Send + 'static,
    ) -> Task<UiMessage> {
        self.background(move |c| f(c).map_err(|e| e.message))
    }

    pub fn update_login(&mut self, msg: LoginMsg) -> Task<UiMessage> {
        let l = &mut self.login;
        l.menu = matches!(msg, LoginMsg::Menu) && !l.menu;
        match msg {
            LoginMsg::Import => {
                let dir = l.source.clone().expect("import screen has a Beeper Desktop dir");
                return self.background(move |_| read_beeper_data(&dir).map(|d| Outcome::Read(Arc::new(d))));
            }
            LoginMsg::Fresh => {
                self.store.write().finish_setup(Setup::Fresh);
                self.login = Login::default();
            }
            LoginMsg::Show(s) => {
                l.screen = s;
                l.error = None;
                if s == Screen::Verify {
                    return self.start_login_verification();
                }
                if s == Screen::Email {
                    return operation::focus(EMAIL_INPUT);
                }
            }
            LoginMsg::Google => l.error = Some("Google sign-in is not available in beeper-rs. Use email.".into()),
            LoginMsg::Email(v) => {
                l.email = v;
                l.error = None;
            }
            LoginMsg::SubmitEmail | LoginMsg::Resend => {
                let email = l.email.trim().to_string();
                if email.is_empty() {
                    l.error = Some(tr("Please enter your email.").into());
                    return Task::none();
                }
                l.code.clear();
                return self.cloud_task(move |c| {
                    let (request, methods) = c.start_login()?;
                    if !methods.iter().any(|m| m == "email") {
                        return Err(CloudError { status: 409, message: NO_EMAIL_LOGIN.into() });
                    }
                    c.send_login_email(&request, &email)?;
                    Ok(Outcome::Requested(request))
                });
            }
            LoginMsg::Code(v) => {
                let code: String = v.chars().filter(char::is_ascii_digit).take(6).collect();
                let submit = code.len() == 6 && code != l.code && !l.busy;
                l.code = code.clone();
                l.error = None;
                if submit {
                    let request = l.request.clone();
                    return self.cloud_task(move |c| match c.send_login_code(&request, &code)? {
                        CodeResult::Token(jwt) => session(c, &jwt),
                        CodeResult::Registration { lead_token, username_suggestions } => {
                            Ok(Outcome::Lead(lead_token, username_suggestions))
                        }
                    });
                }
            }
            LoginMsg::Username(v) => {
                l.username = v;
                l.error = None;
            }
            LoginMsg::SubmitUsername => {
                let (request, lead, name) = (l.request.clone(), l.lead_token.clone(), l.username.trim().to_string());
                if name.is_empty() {
                    l.error = Some("Please enter a username.".into());
                    return Task::none();
                }
                return self.cloud_task(move |c| session(c, &c.register(&request, &lead, &name)?));
            }
            LoginMsg::Menu => {}
            LoginMsg::StartOver => self.login = Login::new(l.source.take()),
            LoginMsg::Query(q) => l.query = q,
            LoginMsg::Continue => {
                self.login = Login::default();
                return self.enter_app();
            }
            LoginMsg::E2ee(r) => return self.e2ee_checked(r),
            LoginMsg::AnotherWay => l.another_way = true,
            LoginMsg::UseRecoveryKey => return self.use_recovery_key(),
            LoginMsg::KeyEdit(action) => {
                crate::verify::edit_recovery_key(&mut l.key_input, action);
                l.error = None;
            }
            LoginMsg::PasteKey => return iced::clipboard::read().map(|t| LoginMsg::Pasted(t).into()),
            LoginMsg::Pasted(text) => {
                l.key_input =
                    text_editor::Content::with_text(&crate::verify::format_recovery_key(&text.unwrap_or_default()));
                l.error = None;
            }
            LoginMsg::KeyOnFocus(text) => {
                let key = beeper_core::matrix::recovery_key_chars(&text.unwrap_or_default());
                let fresh = crate::verify::valid_recovery_key(&key) && !l.tried_keys.contains(&key);
                if fresh && l.screen == Screen::RecoveryKey && !l.busy {
                    l.key_input = text_editor::Content::with_text(&crate::verify::format_recovery_key(&key));
                    return self.update_login(LoginMsg::SubmitRecoveryKey);
                }
            }
            LoginMsg::SubmitRecoveryKey => {
                l.busy = true;
                let key = beeper_core::matrix::recovery_key_chars(&l.key_input.text());
                l.tried_keys.push(key.clone());
                let recover = beeper_core::matrix::recover(self.store.clone(), key);
                return Task::perform(recover, |r| LoginMsg::Recovered(r).into());
            }
            LoginMsg::NewKey => return self.make_recovery_key(),
            LoginMsg::MadeKey(r) => {
                l.busy = false;
                match r {
                    Ok(key) => l.new_key = Some(key),
                    Err(e) => l.error = Some(e),
                }
            }
            LoginMsg::CopyKey => {
                l.copied = true;
                return iced::clipboard::write(l.new_key.clone().expect("a shown key"));
            }
            LoginMsg::KeyNext => (l.screen, l.key_input, l.error) = (Screen::ConfirmKey, Default::default(), None),
            LoginMsg::SubmitConfirmKey => {
                match beeper_core::matrix::confirm_recovery_key(&self.store, &l.key_input.text()) {
                    Ok(()) => self.advance_onboarding(),
                    Err(e) => l.error = Some(e),
                }
            }
            LoginMsg::LostKey => return self.lost_key(),
            LoginMsg::Recovered(r) => {
                l.busy = false;
                l.error = r.err().map(|e| tr("Error: %(message)s").replace("%(message)s", &e));
                self.advance_onboarding();
            }
            LoginMsg::Done(r) => {
                l.busy = false;
                l.error = r.as_ref().err().cloned();
                match r {
                    Err(_) => {}
                    Ok(Outcome::Requested(request)) => {
                        l.request = request;
                        l.screen = Screen::Code;
                        return operation::focus(CODE_INPUT);
                    }
                    Ok(Outcome::Lead(token, suggestions)) => {
                        l.lead_token = token;
                        l.username = suggestions.into_iter().next().unwrap_or_default();
                        l.screen = Screen::Username;
                        return operation::focus(USERNAME_INPUT);
                    }
                    Ok(Outcome::Read(data)) => {
                        l.import = Some(data);
                        l.screen = Screen::Welcome;
                    }
                    Ok(Outcome::Session(session, profile, accounts)) => match l.import.take() {
                        Some(data) if data.crypto.as_ref().is_some_and(|c| c.user_id != session.user_id) => {
                            let user = &data.crypto.as_ref().expect("checked above").user_id;
                            l.error = Some(format!("Sign in as {user} to finish the import."));
                            l.import = Some(data);
                        }
                        Some(data) => {
                            let dir = self.store.read().matrix_dir();
                            return self.background(move |_| {
                                if let Some(c) = &data.crypto {
                                    let import = beeper_core::matrix::import_crypto(&dir, c);
                                    tokio::runtime::Handle::current().block_on(import)?;
                                }
                                Ok(Outcome::Imported(data, session, profile, accounts))
                            });
                        }
                        None => {
                            let homeserver = self.cloud.homeserver.clone();
                            self.store.write().sign_in(session, homeserver, &profile, accounts);
                            return self.check_e2ee();
                        }
                    },
                    Ok(Outcome::Imported(data, session, profile, accounts)) => {
                        let homeserver = self.cloud.homeserver.clone();
                        let mut st = self.store.write();
                        st.import_beeper(Arc::unwrap_or_clone(data));
                        st.sign_in(session, homeserver, &profile, accounts);
                        drop(st);
                        return self.check_e2ee();
                    }
                }
            }
        }
        Task::none()
    }

    pub fn view_login(&self) -> El<'_> {
        let (l, p) = (&self.login, &self.pal);
        let go = |m: LoginMsg| (!l.busy).then(|| m.into());
        let error = column![]
            .push(l.error.as_deref().map(|e| txt(e, 12.0).color(p.text_danger).center().width(Fill)))
            .padding(Padding::new(0.0).left(18.0).right(18.0).bottom(if l.error.is_some() { 12.0 } else { 0.0 }));
        let link = |s: &'static str, m: UiMessage| span(s).color(p.accent).link(m);
        let body: El<'_> = match l.screen {
            Screen::Import => {
                let dir = l.source.as_deref().expect("import screen has a Beeper Desktop dir").display().to_string();
                let info = format!(
                    "beeper-rs found Beeper Desktop data in\n{dir}\n\nCopy your account and chats into beeper-rs? Beeper Desktop stays unchanged."
                );
                column![
                    Space::new().height(40),
                    heading("Your Chats", 36.0),
                    container(art(0, 300.0)).height(Fill).center_y(Fill),
                    error,
                    actions(
                        vec![
                            container(txt(info, 12.0).color(p.on_secondary).center().width(Fill))
                                .padding(Padding::ZERO.bottom(8))
                                .into(),
                            pill(p, label("Start Fresh", None), 36.0, Some(p.chip), go(LoginMsg::Fresh)),
                            pill(
                                p,
                                label(if l.busy { "Copying…" } else { "Copy from Beeper Desktop" }, None),
                                36.0,
                                None,
                                go(LoginMsg::Import)
                            ),
                        ],
                        24.0
                    ),
                ]
                .into()
            }
            Screen::Welcome => {
                let terms = rich_text![
                    span("By continuing, you agree to our ").color(p.text),
                    link("terms\nof use", UiMessage::OpenUrl("https://www.beeper.com/terms".into())),
                    span(" and have read our ").color(p.text),
                    link("privacy policy", UiMessage::OpenUrl("https://www.beeper.com/privacy".into())),
                    span(".").color(p.text),
                ]
                .on_link_click(|m: UiMessage| m)
                .size(11)
                .line_height(iced::widget::text::LineHeight::Absolute(14.0.into()))
                .width(Fill)
                .center();
                let google = svg(ART[3].clone()).width(18).height(18);
                let mail = ic(icon::EMAIL, 16.0).color(Color::WHITE);
                column![
                    container(art(0, 372.0)).height(Fill).center_y(Fill),
                    error,
                    actions(
                        vec![
                            container(terms).padding(Padding::ZERO.bottom(8)).into(),
                            pill(
                                p,
                                label("Continue with Google", Some(google.into())),
                                36.0,
                                Some(p.chip),
                                go(LoginMsg::Google)
                            ),
                            pill(
                                p,
                                label(tr("Continue with Email"), Some(mail.into())),
                                36.0,
                                None,
                                go(LoginMsg::Show(Screen::Email))
                            ),
                        ],
                        24.0
                    ),
                ]
                .into()
            }
            Screen::Email => column![
                Space::new().height(40),
                heading(tr("Enter your Email"), 36.0),
                Space::new().height(36),
                art(1, 314.0),
                Space::new().height(Fill),
                error,
                actions(
                    vec![
                        field(p, EMAIL_INPUT, tr("Email"), &l.email, LoginMsg::Email, LoginMsg::SubmitEmail),
                        pill(p, label(trx("Next", "button label"), None), 33.0, None, go(LoginMsg::SubmitEmail)),
                    ],
                    24.0
                ),
            ]
            .into(),
            Screen::Code => {
                let active = l.code.len().min(5);
                let slots = row((0..6).map(|i| {
                    let digit: Option<El<'_>> =
                        l.code.chars().nth(i).map(|c| semi(c.to_string(), 28.0).color(p.strong).into());
                    let caret = (i == l.code.len())
                        .then(|| container(Space::new()).width(1.5).height(28).style(fill_box(p.strong, 0.0)).into());
                    let edge = match (&l.error, i == active) {
                        (Some(_), _) => p.text_danger,
                        (None, true) => p.accent,
                        (None, false) => CLEAR,
                    };
                    container(digit.or(caret).unwrap_or_else(|| Space::new().into()))
                        .center_x(41)
                        .center_y(56)
                        .style(bordered_box(p.chip, edge, 1.5, 12.0))
                        .into()
                }))
                .spacing(8);
                let capture = text_input("", &l.code)
                    .id(CODE_INPUT)
                    .on_input(|v| LoginMsg::Code(v).into())
                    .size(28)
                    .padding(14)
                    .style(input_style(CLEAR, CLEAR, CLEAR, CLEAR, 0.0));
                let otp = stack![container(slots).center_x(Fill), container(capture).height(56)].height(56);
                let info = format!("Check {}\nfor your login code", l.email.trim());
                let foot = rich_text![
                    span("Didn't get a code? ").color(p.on_secondary),
                    span(tr("Resend")).color(p.accent).link(UiMessage::Login(LoginMsg::Resend))
                ]
                .on_link_click(move |m: UiMessage| if l.busy { UiMessage::Noop } else { m })
                .size(11)
                .width(Fill)
                .center();
                column![
                    Space::new().height(50),
                    heading(tr("Enter Code"), 36.0),
                    art(2, 240.0),
                    otp,
                    Space::new().height(Fill),
                    error,
                    actions(
                        vec![
                            container(txt(info, 13.0).color(p.on_secondary).center().width(Fill))
                                .padding(Padding::ZERO.bottom(8))
                                .into(),
                            pill(
                                p,
                                label(tr("Open Email"), None),
                                33.0,
                                None,
                                Some(UiMessage::OpenUrl("mailto:".into()))
                            ),
                            container(foot).padding(Padding::ZERO.top(8)).into(),
                        ],
                        24.0
                    ),
                ]
                .into()
            }
            Screen::Accounts => {
                let st = self.store.read();
                let search = row![
                    ic(icon::SEARCH, 16.0).color(p.tertiary),
                    text_input(tr("Search..."), &l.query)
                        .on_input(|v| LoginMsg::Query(v).into())
                        .size(14)
                        .padding([10, 0])
                        .style(input_style(CLEAR, p.fg, p.tertiary, CLEAR, 0.0)),
                ]
                .spacing(10)
                .align_y(Center)
                .padding([0, 16]);
                let divider = container(Space::new()).width(Fill).height(1).style(fill_box(p.chip, 0.0));
                let grid = scrollable(network_grid(&st, &l.query, p, p.fg)).height(Fill);
                let go_on =
                    container(pill(p, label(tr("Continue"), None), 30.0, None, Some(LoginMsg::Continue.into())))
                        .padding(Padding::new(12.0).top(8.0));
                let remaining = FREE_ACCOUNTS.saturating_sub(st.bridge_accounts.len());
                let foot = txt(
                    trn("%d free account remaining.", "%d free accounts remaining.", remaining, &[&remaining]),
                    12.0,
                )
                .color(p.on_secondary);
                column![
                    container(bold(tr("Connect Chat Accounts"), 26.0).color(HEADING)).center_x(Fill).center_y(48),
                    Space::new().height(8),
                    container(column![search, divider, grid, go_on]).height(Fill).style(card_style(p.card)),
                    container(foot).center_x(Fill).center_y(31),
                ]
                .padding([0, 8])
                .into()
            }
            Screen::Verify => self.view_onboarding_verify(),
            Screen::RecoveryKey => self.view_onboarding_key(),
            Screen::Syncing => self.view_onboarding_syncing(),
            Screen::Preamble => self.view_onboarding_preamble(),
            Screen::NewKey | Screen::ConfirmKey => self.view_onboarding_new_key(),
            Screen::Reset => self.view_onboarding_reset(),
            Screen::Username => column![
                Space::new().height(34),
                heading(tr("Pick a Username"), 28.0),
                container(art(0, 300.0)).height(Fill).center_y(Fill),
                error,
                actions(
                    vec![
                        field(
                            p,
                            USERNAME_INPUT,
                            tr("Username"),
                            &l.username,
                            LoginMsg::Username,
                            LoginMsg::SubmitUsername
                        ),
                        pill(p, label(trx("Next", "button label"), None), 33.0, None, go(LoginMsg::SubmitUsername)),
                    ],
                    52.0
                ),
            ]
            .into(),
        };
        let chevron = button(container(ic(icon::EXPAND_MORE, 16.0).color(p.strong)).center(28))
            .padding(0)
            .style(btn_style([CLEAR, p.chip, p.chip], [p.strong; 3], 6.0))
            .on_press(LoginMsg::Menu.into());
        let controls = container(chevron).align_right(Fill).height(36).padding(Padding::ZERO.top(2).right(2));
        let onboarding = l.screen == Screen::Accounts;
        let page = container(column![controls, body]).width(Fill).height(Fill);
        let page = if onboarding { page } else { page.style(fill_box(p.primary_container, 8.0)) };
        let item = |s: &'static str, m: UiMessage| -> El<'static> {
            button(txt(s, 13.0).color(p.strong))
                .width(Fill)
                .padding([6, 10])
                .style(btn_style([CLEAR, p.chip, p.chip], [p.strong; 3], 6.0))
                .on_press(m)
                .into()
        };
        let menu = l.menu.then(|| {
            let back = previous(l.screen).map(|s| item(tr("Go Back"), LoginMsg::Show(s).into()));
            let last = if onboarding {
                item(tr("Report a Problem"), UiMessage::OpenUrl(crate::ISSUES_URL.into()))
            } else {
                item(tr("Start Over"), LoginMsg::StartOver.into())
            };
            let items = column![].push(back).push(last).padding(4).width(160);
            mouse_area(
                container(container(items).style(shadow_box(p.elevated, p.chip, 8.0, 14.0)))
                    .align_right(Fill)
                    .height(Fill)
                    .padding(Padding::ZERO.top(34).right(6)),
            )
            .on_press(LoginMsg::Menu.into())
        });
        let (pad, frame) = if onboarding { (0, p.chip) } else { (8, p.window) };
        container(stack![page].push(menu)).padding(pad).style(fill_box(frame, 0.0)).into()
    }
}
