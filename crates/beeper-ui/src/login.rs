use crate::i18n::{tr, trf, trn, trx, trxf};
use crate::style::{
    bold, bordered_box, brand_svg, btn_style, fill_box, glow_box, gradient_text, ic, icon, input_style, med, semi,
    shadow_box, txt, Palette, CLEAR,
};
use crate::verify::Brand;
use crate::{BeeperGuiApp, El, UiMessage};
use beeper_core::cloud::{Cloud, CloudError, CodeResult};
use beeper_core::config::{account_network, NETWORK_SPECS};
use beeper_core::import::{read_beeper_data, BeeperData};
use beeper_core::matrix::{DeviceInfo, E2eeSetup};
use beeper_core::models::{BridgeAccountInfo, MatrixSession, Profile};
use beeper_core::store::{BeeperStore, Setup, NOTE_CHAT_ID};
use iced::gradient::Linear;
use iced::keyboard::{key::Named, Key, Modifiers};
use iced::widget::{
    button, column, container, mouse_area, operation, rich_text, row, scrollable, span, stack, svg, text_editor,
    text_input, tooltip, Space,
};
use iced::{border, window, Background, Center, Color, Degrees, Fill, Gradient, Padding, Size, Task, Theme};
use std::path::PathBuf;
use std::sync::{Arc, LazyLock};

/// The room over a `field` for its floating label: the label rises 60 % of the 38 px field and is 26.125 px high. A
/// screen with a field puts the same room under its heading, so that its gaps are Desktop's.
const FLOAT_LABEL: f32 = 38.0 * 0.6 + 38.0 * 0.6875 / 2.0 - 38.0 / 2.0;
/// The room of Desktop's floating `small` error over the form of the code screen: 8 px and one line of 10.4 px text.
const ERROR_ROOM: f32 = 8.0 + 13.0;
/// The start of the animations of the sign-in window.
pub(crate) static EPOCH: LazyLock<std::time::Instant> = LazyLock::new(std::time::Instant::now);
/// The size of the sign-in window.
pub(crate) const SIZE: Size = Size::new(330.0, 620.0);
const EMAIL_INPUT: &str = "login-email";
const CODE_INPUT: &str = "login-code";
const USERNAME_INPUT: &str = "login-username";
const TERMS_URL: &str = "https://www.beeper.com/terms-onboarding";
const PRIVACY_URL: &str = "https://www.beeper.com/privacy";
/// Desktop's `--color-on-error-container`: the error toast.
const ERROR_RED: Color = Color::from_rgb8(220, 54, 46);
const NO_EMAIL_LOGIN: &str = "The server does not support email-based login. Try logging in with a username instead.";

/// Desktop's `logo-google.svg`.
const GOOGLE_G: &str = r##"<svg viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg"><path fill-rule="evenodd" d="M23.52 12.2724C23.52 11.4216 23.4432 10.6044 23.3016 9.81836H12V14.46H18.4584C18.3247 15.1942 18.0435 15.8937 17.6318 16.5162C17.2202 17.1386 16.6865 17.6712 16.0632 18.0816V21.0936H19.9416C22.2108 19.0032 23.52 15.9264 23.52 12.2724Z" fill="#4285F4"/><path fill-rule="evenodd" d="M12 24C15.24 24 17.9568 22.926 19.9416 21.0924L16.0632 18.0816C14.9892 18.8016 13.6152 19.2276 12 19.2276C8.87399 19.2276 6.22799 17.1156 5.28599 14.28H1.27679V17.388C2.27521 19.3761 3.80684 21.0474 5.70051 22.2151C7.59418 23.3827 9.77527 24.0007 12 24Z" fill="#34A853"/><path fill-rule="evenodd" d="M5.286 14.2801C5.046 13.5601 4.9092 12.7921 4.9092 12.0001C4.9092 11.2081 5.046 10.4401 5.286 9.72006V6.61206H1.2768C0.436818 8.28391 -0.00043132 10.1291 3.19265e-07 12.0001C3.19265e-07 13.9369 0.4632 15.7681 1.2768 17.3881L5.2848 14.2801H5.286Z" fill="#FBBC05"/><path fill-rule="evenodd" d="M12 4.7724C13.7616 4.7724 15.3432 5.3784 16.5876 6.5676L20.0292 3.126C17.9508 1.188 15.234 0 12 0C7.30799 0 3.25199 2.688 1.27679 6.612L5.28479 9.72C6.23039 6.8832 8.87519 4.7724 12 4.7724Z" fill="#EA4335"/></svg>"##;

/// Desktop's `EmailFill`, `OutlineChevronDownSmall`, `Done`, `Sync` and `CloseSmall` icons.
static ICONS: LazyLock<[svg::Handle; 5]> = LazyLock::new(|| {
    [
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 14"><path fill="currentColor" d="M9.914 7.734q-.383 0-.75-.32L1.36.54Q1.985.134 2.945.133h13.946q.96 0 1.578.406l-7.805 6.875q-.36.32-.75.32M.695 12.336a2.5 2.5 0 0 1-.148-.5 3.5 3.5 0 0 1-.055-.656V2.555q0-.407.055-.696.063-.288.133-.406l5.797 5.11zm2.016 1.266q-.43 0-.766-.094a1.8 1.8 0 0 1-.547-.25l5.946-5.945 1.14 1.015q.345.298.704.453.36.15.726.149.367 0 .727-.149.366-.156.71-.453l1.141-1.015 5.945 5.945q-.218.156-.554.25a2.8 2.8 0 0 1-.758.094zm16.43-1.266-5.782-5.773 5.797-5.11q.063.117.125.406.063.29.063.696v8.625q0 .375-.063.656a3 3 0 0 1-.14.5"/></svg>"#,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" fill="none"><path d="M6 8l4 4 4-4" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/></svg>"#,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20"><path fill="currentColor" d="m7.958 15-4.75-4.75 1.188-1.187 3.562 3.562 7.646-7.646 1.188 1.188z"/></svg>"#,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20"><path fill="currentColor" d="M3.334 16.667v-1.666h2.291l-.333-.292q-1.084-.959-1.521-2.187a7.4 7.4 0 0 1-.437-2.48q0-2.312 1.385-4.114t3.615-2.386v1.75a4.940 4.940 0 0 0-2.417 1.844A4.930 4.930 0 0 0 5 10.042q0 .938.354 1.823.354.886 1.104 1.636l.209.208v-2.042h1.667v5zm8.333-.208v-1.75a4.940 4.940 0 0 0 2.417-1.844A4.930 4.930 0 0 0 15 9.960q0-.938-.354-1.823a4.900 4.900 0 0 0-1.104-1.635l-.208-.209v2.042h-1.667v-5h5v1.667h-2.292l.334.291q1.020 1.020 1.490 2.220.468 1.197.468 2.447 0 2.313-1.386 4.115t-3.614 2.385"/></svg>"#,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20"><path fill="currentColor" d="M7.042 13.999 6 12.957l2.938-2.958L6 7.060 7.042 6.020 10 8.957l2.938-2.937 1.041 1.041L11.042 10l2.937 2.958-1.041 1.042L10 11.060z"/></svg>"#,
    ]
    .map(|s| svg::Handle::from_memory(s.as_bytes()))
});

static BUBBLE_ARROW: LazyLock<svg::Handle> = LazyLock::new(|| {
    svg::Handle::from_memory(br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 9"><path d="M0 0.5h16L8 8.5z" fill="#fff" stroke="#a0a0a0"/><path d="M0 0h16v1H0z" fill="#fff"/></svg>"##.as_slice())
});

/// The first frames of Desktop's Lottie animations `username.json`, `email.json` and `lock.json`, made with
/// velato, and the Google logo.
static ART: LazyLock<[svg::Handle; 4]> = LazyLock::new(|| {
    [
        include_bytes!("../assets/onboarding/lottie-username.svg").as_slice(),
        include_bytes!("../assets/onboarding/lottie-email.svg"),
        include_bytes!("../assets/onboarding/lottie-lock.svg"),
        GOOGLE_G.as_bytes(),
    ]
    .map(svg::Handle::from_memory)
});

/// The layers of Desktop's `AnimatedCompositeIllustration`, bottom first: the shadow, which stays, then the 13
/// layers that float.
static SCENE: LazyLock<([svg::Handle; 13], iced::widget::image::Handle)> = LazyLock::new(|| {
    let svgs = [
        include_bytes!("../assets/onboarding/composite-beeper-dimension-shadow.svg").as_slice(),
        include_bytes!("../assets/onboarding/composite-instagram-dimension.svg"),
        include_bytes!("../assets/onboarding/composite-smile-dimension.svg"),
        include_bytes!("../assets/onboarding/composite-bubbles-dimension03.svg"),
        include_bytes!("../assets/onboarding/composite-camera.svg"),
        include_bytes!("../assets/onboarding/composite-haha.svg"),
        include_bytes!("../assets/onboarding/composite-haha-small.svg"),
        include_bytes!("../assets/onboarding/composite-haha-medium.svg"),
        include_bytes!("../assets/onboarding/composite-lock.svg"),
        include_bytes!("../assets/onboarding/composite-signal-dimension.svg"),
        include_bytes!("../assets/onboarding/composite-sparkle.svg"),
        include_bytes!("../assets/onboarding/composite-telegram-dimension.svg"),
        include_bytes!("../assets/onboarding/composite-whatsapp-dimension.svg"),
    ];
    let pager = iced::widget::image::Handle::from_bytes(include_bytes!("../assets/onboarding/beeper.webp").as_slice());
    (svgs.map(svg::Handle::from_memory), pager)
});

/// The size of the welcome illustration.
const SCENE_SIZE: Size = Size::new(314.0, 396.0);

/// CSS `ease-in-out`, `cubic-bezier(0.42, 0, 0.58, 1)`, at `x` in 0..1.
fn ease_in_out(x: f32) -> f32 {
    let bezier = |s: f32, a: f32, b: f32| 3.0 * a * s * (1.0 - s) * (1.0 - s) + 3.0 * b * s * s * (1.0 - s) + s * s * s;
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..20 {
        let mid = (lo + hi) / 2.0;
        if bezier(mid, 0.42, 0.58) < x {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    bezier((lo + hi) / 2.0, 0.0, 1.0)
}

/// Desktop's `AnimatedCompositeIllustration`: the shadow, then 13 layers that each float up and down by
/// `z / limit * intensity / 2` % of their height, with `z = (n + 1) * 5`, `limit = 13` and `intensity = 0.5`, in
/// 2 s each way with `easeInOut`, and `z / limit` s ahead of the start.
fn welcome_scene<'a>() -> El<'a> {
    let (svgs, pager) = &*SCENE;
    let t = EPOCH.elapsed().as_secs_f32();
    let layer = |n: usize, el: El<'a>| -> El<'a> {
        let r = (n + 1) as f32 * 5.0 / 13.0;
        let amplitude = r * 0.5 / 2.0 / 100.0 * SCENE_SIZE.height;
        let u = (t + r) / 2.0;
        let e = ease_in_out(u.fract());
        let y = if (u as u32).is_multiple_of(2) { 1.0 - 2.0 * e } else { 2.0 * e - 1.0 };
        iced::widget::pin(el).y(amplitude * y).into()
    };
    let fit = |h: &svg::Handle| -> El<'a> { svg(h.clone()).width(SCENE_SIZE.width).height(SCENE_SIZE.height).into() };
    let pager = iced::widget::image(pager.clone()).width(SCENE_SIZE.width).height(SCENE_SIZE.height).into();
    // The pager photo is the third floating layer.
    let floating = svgs[1..3].iter().map(fit).chain([pager]).chain(svgs[3..].iter().map(fit));
    let layers = std::iter::once(fit(&svgs[0])).chain(floating.enumerate().map(|(n, el)| layer(n, el)));
    container(iced::widget::Stack::with_children(layers).width(SCENE_SIZE.width).height(SCENE_SIZE.height))
        .center(Fill)
        .into()
}

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

/// The controls that Tab reaches on a screen, in Desktop's DOM order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    Menu,
    Terms,
    Google,
    Email,
    Field,
    Next,
    OpenEmail,
    Resend,
    Fresh,
    Import,
}

impl Screen {
    fn stops(self) -> &'static [Stop] {
        match self {
            Screen::Import => &[Stop::Menu, Stop::Fresh, Stop::Import],
            Screen::Welcome => &[Stop::Menu, Stop::Terms, Stop::Google, Stop::Email],
            Screen::Email | Screen::Username => &[Stop::Menu, Stop::Field, Stop::Next],
            Screen::Code => &[Stop::Menu, Stop::Field, Stop::OpenEmail, Stop::Resend],
            Screen::Verify
            | Screen::RecoveryKey
            | Screen::Reset
            | Screen::Syncing
            | Screen::Preamble
            | Screen::NewKey
            | Screen::ConfirmKey
            | Screen::Accounts => &[],
        }
    }

    /// The text field of the screen, which Desktop focuses when the screen shows.
    fn field(self) -> Option<&'static str> {
        match self {
            Screen::Email => Some(EMAIL_INPUT),
            Screen::Code => Some(CODE_INPUT),
            Screen::Username => Some(USERNAME_INPUT),
            _ => None,
        }
    }

    /// Desktop's `OnboardingFrame` menu: Go Back to `previousScreen`, Report a Problem, and Start Over before the
    /// sign-in or Logout after it, when `withStartOver` is set.
    fn menu(self) -> (Option<Screen>, Option<(&'static str, UiMessage)>) {
        let start_over = Some(("Start Over", LoginMsg::StartOver.into()));
        let logout = Some(("Logout", UiMessage::Logout));
        match self {
            Screen::Import | Screen::Welcome | Screen::Syncing | Screen::NewKey => (None, None),
            Screen::Email => (Some(Screen::Welcome), None),
            Screen::Code => (Some(Screen::Email), start_over),
            Screen::Username => (None, start_over),
            Screen::Verify | Screen::Preamble | Screen::Accounts => (None, logout),
            Screen::RecoveryKey | Screen::Reset => (Some(Screen::Verify), logout),
            Screen::ConfirmKey => (Some(Screen::NewKey), logout),
        }
    }
}

#[derive(Default)]
pub struct Login {
    pub screen: Screen,
    pub email: String,
    pub code: String,
    pub username: String,
    /// The usernames that the server suggests, which the sync icon of the field goes through.
    suggestions: Vec<String>,
    /// The error under the heading of a screen, and Desktop's red error toast at the top of the window.
    pub error: Option<String>,
    pub toast: Option<String>,
    /// When Desktop's auto paste of a code from the clipboard showed its toast.
    pasted: Option<std::time::Instant>,
    /// The codes sent since the last code request, which the clipboard does not paste again (Desktop's stale codes).
    tried_codes: Vec<String>,
    /// The bubble of Chromium's `type=email` check over the email field.
    pub invalid: Option<String>,
    /// The index of the control in `Screen::stops` that has the keyboard focus.
    pub focus: Option<usize>,
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
    /// The keys did not come 60 s after `IndicateSyncing` showed.
    pub late: bool,
    /// Desktop's `RecoveryCodeInput`, and the keys that were tried, which a paste on focus does not submit again.
    pub key_input: text_editor::Content,
    pub tried_keys: Vec<String>,
    /// The new recovery key, and whether "Copy to Clipboard" copied it.
    pub new_key: Option<String>,
    pub copied: bool,
}

impl Login {
    /// The screens with the floating welcome illustration, and the toast of a pasted code until it goes.
    pub fn animated(&self) -> bool {
        matches!(self.screen, Screen::Import | Screen::Welcome | Screen::Syncing) || self.pasted() || self.waiting
    }

    /// Desktop's `isNewAccount`: the user made the account in this sign-in.
    pub fn new_account(&self) -> bool {
        !self.lead_token.is_empty()
    }

    /// The toast of a code from the clipboard shows for 3 s.
    fn pasted(&self) -> bool {
        self.pasted.is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(3))
    }

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
    /// The sync icon of the username field: the next suggestion.
    NextSuggestion,
    SubmitCode,
    /// "Open Email": Desktop's `openEmailClient`.
    OpenEmail,
    Menu,
    StartOver,
    CloseToast,
    Query(String),
    Continue,
    Done(Result<Outcome, CloudError>),
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
    /// 60 s on `IndicateSyncing`.
    KeysLate,
}

impl From<LoginMsg> for UiMessage {
    fn from(m: LoginMsg) -> Self {
        UiMessage::Login(m)
    }
}

/// Chromium's message for a value that is not a valid `type=email` address (`EmailInputType::TypeMismatchText`).
fn email_mismatch(v: &str) -> Option<String> {
    let local_ok = |c: char| c.is_ascii_alphanumeric() || ".!#$%&'*+/=?^_`{|}~-".contains(c);
    let domain_ok = |c: char| c.is_alphanumeric() || c == '-' || c == '.';
    let label_ok = |l: &str| l.len() <= 63 && !l.starts_with('-') && !l.ends_with('-');
    let Some((local, domain)) = v.split_once('@') else {
        return Some(format!("Please include an '@' in the email address. '{v}' is missing an '@'."));
    };
    let bad = |part: &str, ok: &dyn Fn(char) -> bool| part.chars().find(|c| !ok(*c));
    Some(if local.is_empty() {
        format!("Please enter a part followed by '@'. '{v}' is incomplete.")
    } else if domain.is_empty() {
        format!("Please enter a part following '@'. '{v}' is incomplete.")
    } else if let Some(c) = bad(local, &local_ok) {
        format!("A part followed by '@' should not contain the symbol '{c}'.")
    } else if let Some(c) = bad(domain, &domain_ok) {
        format!("A part following '@' should not contain the symbol '{c}'.")
    } else if domain.starts_with('.') || domain.ends_with('.') || domain.contains("..") {
        format!("'.' is used at a wrong position in '{domain}'.")
    } else if domain.split('.').all(label_ok) {
        return None;
    } else {
        "Please enter an email address.".into()
    })
}

/// Desktop's username input (`Lzt`): lower case `a-z`, `0-9` and `-` only.
fn username_input(v: &str) -> String {
    v.to_lowercase().chars().filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-').collect()
}

/// Desktop's username rules (`Pzt`): 3 to 32 of `a-z`, `0-9` and `-` with no `-` at the ends, and not a name that
/// ends with "bot", has "beep", "support" or "admin" in it, or is all digits.
fn username_ok(name: &str) -> bool {
    let end = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit();
    (3..=32).contains(&name.len())
        && name.chars().all(|c| end(c) || c == '-')
        && name.starts_with(end)
        && name.ends_with(end)
        && !name.ends_with("bot")
        && !["beep", "support", "admin"].iter().any(|w| name.contains(w))
        && !name.chars().all(|c| c.is_ascii_digit())
}

/// Desktop's `openEmailClient` on Linux: `gtk-launch` of the default `mailto` handler that `xdg-settings` names, and
/// `mailto:` when there is none or it does not start.
fn open_email_client() -> Task<UiMessage> {
    let launch = || {
        let handler = std::process::Command::new("xdg-settings")
            .args(["get", "default-url-scheme-handler", "mailto"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|h| !h.is_empty());
        handler.is_some_and(|h| std::process::Command::new("gtk-launch").arg(h).status().is_ok_and(|s| s.success()))
    };
    Task::perform(async move { tokio::task::spawn_blocking(launch).await.expect("mail client thread") }, |opened| {
        if opened {
            UiMessage::Noop
        } else {
            UiMessage::OpenUrl("mailto:".into())
        }
    })
}

/// An error of beeper-rs itself, which shows in the toast like a network error.
fn local_error(message: String) -> CloudError {
    CloudError { status: 0, message }
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

/// Desktop's onboarding error toast: 300x32 at the top of the window, with an info icon and a close button.
/// Desktop's `showToast`: a neutral toast with centered text and no close button, which goes after 3 s.
fn toast<'a>(p: &Palette, text: &'a str, error: bool) -> El<'a> {
    let (bg, fg) = if error { (ERROR_RED, Color::WHITE) } else { (p.card, p.strong) };
    let text = txt(text, 13.0).line_height(iced::Pixels(16.0)).color(fg).width(Fill);
    let line: El<'a> = match error {
        true => {
            let close = button(ic(icon::CLOSE, 14.0).color(fg))
                .padding(3)
                .style(btn_style([CLEAR; 3], [fg; 3], 4.0))
                .on_press(LoginMsg::CloseToast.into());
            row![ic(icon::INFO, 20.0).color(fg), text, close].spacing(6).align_y(Center).into()
        }
        false => text.center().into(),
    };
    let card = container(line).padding([0, 6]).width(300).center_y(32).style(glow_box(bg, p.shadow, 10.0, 1.0, 10.0));
    container(card).center_x(Fill).padding(Padding::ZERO.top(44)).into()
}

/// Chromium's validation bubble over the email field, with its orange "!" icon and its arrow.
fn bubble(text: &str) -> El<'_> {
    let mark =
        container(bold("!", 15.0).color(Color::WHITE)).center(22).style(fill_box(Color::from_rgb8(232, 89, 12), 2.0));
    let line = row![mark, txt(text, 14.0).line_height(iced::Pixels(16.0)).color(Color::BLACK).width(Fill)].spacing(8);
    let card = container(line).padding(8).width(162).style(shadow_box(
        Color::WHITE,
        Color::from_rgb8(160, 160, 160),
        2.0,
        4.0,
    ));
    let arrow = svg(BUBBLE_ARROW.clone()).width(16).height(9);
    let tip = container(column![card, container(arrow).padding(Padding::ZERO.left(8))]);
    container(tip).align_bottom(Fill).padding(Padding::ZERO.left(85).bottom(112)).into()
}

/// Desktop's `--gradient-brand`, `linear-gradient(190.1deg, #6953f2 -7.24%, #0c52f9 109.66%)`.
/// iced spans a gradient over a shorter line than CSS, so the stops are the CSS colors at the ends of the iced line
/// in a 278x36 button.
pub(crate) fn brand_gradient() -> Background {
    lit_gradient(1.0)
}

/// `brand_gradient` made `k` times brighter, like `filter: brightness(k)`.
fn lit_gradient(k: f32) -> Background {
    let c = |r: f32, g: f32, b: f32| {
        Color::from_rgb((r * k / 255.0).min(1.0), (g * k / 255.0).min(1.0), (b * k / 255.0).min(1.0))
    };
    let g = Linear::new(Degrees(190.1)).add_stop(0.0, c(82.5, 82.8, 243.7)).add_stop(1.0, c(36.4, 82.3, 247.2));
    Background::Gradient(Gradient::Linear(g))
}

/// Desktop's `BrandButton` in `size: "small"`: a full-width pill around `content`, 8 px above and below it. Without
/// a message it is disabled. While `loading` it is disabled and shows three bouncing dots over its hidden content.
pub(crate) fn brand_button<'a>(
    p: &Palette,
    content: El<'a>,
    variant: Brand,
    msg: Option<UiMessage>,
    loading: bool,
) -> El<'a> {
    let msg = msg.filter(|_| !loading);
    let (bg, fg, edge) = match (msg.is_some(), variant) {
        (false, _) => (Some(p.chip), p.on_secondary, CLEAR),
        (true, Brand::Primary) => (None, Color::WHITE, CLEAR),
        (true, Brand::Secondary) => (Some(CLEAR), p.accent, p.accent),
        (true, Brand::Muted) => (Some(p.chip), p.strong, CLEAR),
        (true, Brand::Danger) => (Some(p.danger), Color::WHITE, CLEAR),
    };
    let dots = loading.then(|| {
        // `BrandButton-module__bounce`: each dot moves 33 % of its size up and down in 0.5 s, a third apart.
        let t = EPOCH.elapsed().as_secs_f32() / 0.5;
        let dot = |k: usize| {
            let u = t + k as f32 / 3.0;
            let f = u.fract();
            let e = ease_in_out(f);
            let y = 1.32 * (2.0 * if (u as u32).is_multiple_of(2) { 1.0 - e } else { e } - 1.0);
            let dot = container(Space::new()).width(4).height(4).style(fill_box(fg, 2.0));
            container(dot).padding(Padding::ZERO.top(2.0 + y)).height(8).into()
        };
        container(row((0..3).map(dot)).spacing(8)).center(Fill)
    });
    let hover = matches!(variant, Brand::Primary | Brand::Secondary) && msg.is_some();
    button(stack![container(content).center_x(Fill)].push(dots))
        .width(Fill)
        .padding([9, 11])
        .style(move |_: &Theme, s| {
            // `filter: brightness(1.05)` on hover.
            let k = if hover && s == button::Status::Hovered { 1.05 } else { 1.0 };
            let lift = |c: Color| Color { r: (c.r * k).min(1.0), g: (c.g * k).min(1.0), b: (c.b * k).min(1.0), a: c.a };
            button::Style {
                background: Some(bg.map_or_else(|| lit_gradient(k), |c| lift(c).into())),
                text_color: if loading { CLEAR } else { lift(fg) },
                border: border::rounded(9999).color(edge).width(1),
                ..Default::default()
            }
        })
        .on_press_maybe(msg)
        .into()
}

pub(crate) fn actions(items: Vec<El<'_>>, bottom: f32) -> El<'_> {
    column(items).spacing(8).padding(Padding::new(0.0).left(18.0).right(18.0).bottom(bottom)).into()
}

/// The content of a `BrandButton`: 13 px medium text, after an 18 px icon with an 8 px gap.
fn label<'a>(s: impl iced::widget::text::IntoFragment<'a>, lead: Option<El<'a>>) -> El<'a> {
    let text = med(s, 13.0).line_height(iced::Pixels(14.95));
    row![].push(lead).push(text).spacing(8).align_y(Center).into()
}

/// Desktop's `BrandHeading` with `gradient` on a 40 px line.
pub(crate) fn heading<'a>(s: &str, size: f32) -> El<'a> {
    let (text, width) = gradient_text(s, size, 40.0);
    container(svg(text).width(width).height(40)).center_x(Fill).into()
}

pub(crate) fn art<'a>(i: usize, height: f32) -> El<'a> {
    svg(ART[i].clone()).width(Fill).height(height).into()
}

/// Desktop's `BrandInput` with `floatingPlaceholder`: text in the color of the placeholder, and the placeholder
/// as a small chip over the top edge once the field has text. Enter submits the form through the key handler of
/// the window, like Desktop's Enter hotkey.
pub(crate) fn field<'a, M: Into<UiMessage> + 'a>(
    p: &Palette,
    id: &'static str,
    hint: &'a str,
    value: &'a str,
    on: Option<fn(String) -> M>,
) -> El<'a> {
    field_with_icons(p, id, hint, value, on, Vec::new())
}

/// A `field` with `BrandInput`'s `rightIcons`: icons 8 px from the right edge, 2 px apart.
fn field_with_icons<'a, M: Into<UiMessage> + 'a>(
    p: &Palette,
    id: &'static str,
    hint: &'a str,
    value: &'a str,
    on: Option<fn(String) -> M>,
    icons: Vec<El<'a>>,
) -> El<'a> {
    let room = if icons.is_empty() { 12.0 } else { 8.0 + 22.0 * icons.len() as f32 };
    let input = text_input(hint, value)
        .id(id)
        .on_input_maybe(on.map(|on| move |v| on(v).into()))
        .size(14)
        .line_height(iced::Pixels(18.0))
        .padding(Padding::new(10.0).left(12.0).right(room))
        .style(input_style(p.chip, p.on_secondary, p.on_secondary, p.accent, 19.0));
    let icons = (!icons.is_empty()).then(|| {
        container(row(icons).spacing(2).align_y(Center)).align_right(Fill).center_y(38).padding(Padding::ZERO.right(8))
    });
    let input = stack![input].push(icons);
    // `translate(26px, -60%) scale(0.6875)` of the label, with its origin at the left middle of the field.
    let chip = (!value.is_empty()).then(|| {
        let hint = txt(hint, 14.0 * 0.6875).line_height(iced::Pixels(18.0 * 0.6875)).color(p.on_secondary);
        let chip = container(hint).padding([10.0 * 0.6875, 12.0 * 0.6875]).style(fill_box(p.card, 99.0));
        container(chip).padding(Padding::ZERO.left(26))
    });
    stack![column![Space::new().height(FLOAT_LABEL), input]].push(chip).into()
}

impl BeeperGuiApp {
    pub fn login_window_settings() -> window::Settings {
        let settings = Self::window_settings(SIZE);
        window::Settings { resizable: false, position: window::Position::Centered, ..settings }
    }

    pub fn refresh_cloud_accounts(&self) -> Task<UiMessage> {
        let (cloud, token) = (self.cloud.clone(), self.store.read().beeper_state.access_token.clone());
        Task::perform(
            async move { tokio::task::spawn_blocking(move || cloud.whoami(&token)).await.expect("whoami worker thread") },
            |r| UiMessage::CloudAccounts(r.map_err(|e| e.message)),
        )
    }

    /// Desktop's `showHomePage` after the onboarding: the same window becomes the main window, resizable, at
    /// 1200x800.
    pub fn enter_app(&mut self) -> Task<UiMessage> {
        let id = self.login_window.take().expect("login window is open");
        self.select_chat(NOTE_CHAT_ID);
        self.main_window = Some(id);
        Task::batch([window::set_resizable(id, true), window::resize(id, crate::MAIN_SIZE)])
    }

    /// Desktop's `InnerApp`: when encrypted messaging of this device goes back to the verification or to the keys, the
    /// main window shows the onboarding again at the onboarding size.
    pub fn leave_app(&mut self) -> Task<UiMessage> {
        let stage = self.store.read().beeper_state.stage();
        let back = matches!(stage, "pre-verification" | "awaiting-secrets" | "awaiting-key-backup");
        let Some(id) = self.main_window.take_if(|_| back && self.login_window.is_none()) else { return Task::none() };
        (self.login_window, self.login) = (Some(id), Login::default());
        Task::batch([window::set_resizable(id, false), window::resize(id, SIZE), self.resume_onboarding()])
    }

    /// Desktop's onboarding of a signed-in device before the stage "done": it starts at the first onboarding screen
    /// and checks encrypted messaging.
    pub fn resume_onboarding(&mut self) -> Task<UiMessage> {
        self.login.screen = Screen::Email;
        self.check_e2ee()
    }

    fn background(
        &mut self,
        f: impl FnOnce(&Cloud) -> Result<Outcome, CloudError> + Send + 'static,
    ) -> Task<UiMessage> {
        self.login.busy = true;
        (self.login.error, self.login.toast) = (None, None);
        let cloud = self.cloud.clone();
        Task::perform(
            async move { tokio::task::spawn_blocking(move || f(&cloud)).await.expect("login window worker thread") },
            |r| LoginMsg::Done(r).into(),
        )
    }

    /// Shows `screen` with its text field focused, like Desktop's `autoFocus`.
    fn show_screen(&mut self, screen: Screen) -> Task<UiMessage> {
        let l = &mut self.login;
        (l.screen, l.error, l.toast, l.invalid) = (screen, None, None, None);
        l.focus = screen.stops().iter().position(|s| *s == Stop::Field);
        // Desktop mounts a new email form, so the screen shows empty.
        if screen == Screen::Email {
            l.email.clear();
        }
        if screen == Screen::Verify {
            return self.start_login_verification();
        }
        screen.field().map_or_else(Task::none, operation::focus)
    }

    pub fn update_login(&mut self, msg: LoginMsg) -> Task<UiMessage> {
        let l = &mut self.login;
        l.menu = matches!(msg, LoginMsg::Menu) && !l.menu;
        match msg {
            LoginMsg::Import => {
                let dir = l.source.clone().expect("import screen has a Beeper Desktop dir");
                return self.background(move |_| {
                    read_beeper_data(&dir).map(|d| Outcome::Read(Arc::new(d))).map_err(local_error)
                });
            }
            LoginMsg::Fresh => {
                self.store.write().finish_setup(Setup::Fresh);
                self.login = Login::default();
            }
            LoginMsg::Show(s) => return self.show_screen(s),
            LoginMsg::Google => l.toast = Some("Google sign-in is not available in beeper-rs. Use email.".into()),
            LoginMsg::Email(v) => {
                l.email = v;
                (l.toast, l.invalid) = (None, None);
                l.focus = Screen::Email.stops().iter().position(|s| *s == Stop::Field);
            }
            LoginMsg::SubmitEmail | LoginMsg::Resend => {
                let email = l.email.trim().to_string();
                if email.is_empty() {
                    l.toast = Some(tr("Please enter your email.").into());
                    return Task::none();
                }
                if let Some(e) = email_mismatch(&email) {
                    l.invalid = Some(e);
                    return operation::focus(EMAIL_INPUT);
                }
                l.code.clear();
                l.tried_codes.clear();
                return self.background(move |c| {
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
                let submit = code.len() == 6 && code != l.code;
                l.code = code;
                (l.error, l.toast) = (None, None);
                if submit {
                    return self.update_login(LoginMsg::SubmitCode);
                }
            }
            LoginMsg::SubmitCode if l.busy => {}
            LoginMsg::SubmitCode => {
                let (request, code) = (l.request.clone(), l.code.clone());
                l.tried_codes.push(code.clone());
                return self.background(move |c| match c.send_login_code(&request, &code)? {
                    CodeResult::Token(jwt) => session(c, &jwt),
                    CodeResult::Registration { lead_token, username_suggestions } => {
                        Ok(Outcome::Lead(lead_token, username_suggestions))
                    }
                });
            }
            LoginMsg::Username(v) => {
                l.username = username_input(&v);
                (l.error, l.toast) = (None, None);
            }
            LoginMsg::NextSuggestion => {
                let at = l.suggestions.iter().position(|s| *s == l.username).map_or(0, |i| i + 1);
                let name = l.suggestions[at % l.suggestions.len()].clone();
                return self.update_login(LoginMsg::Username(name));
            }
            LoginMsg::SubmitUsername => {
                let (request, lead, name) = (l.request.clone(), l.lead_token.clone(), l.username.clone());
                assert!(username_ok(&name), "Next takes a valid username only");
                return self.background(move |c| session(c, &c.register(&request, &lead, &name)?));
            }
            LoginMsg::Menu => {}
            LoginMsg::CloseToast => l.toast = None,
            LoginMsg::OpenEmail => return open_email_client(),
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
            LoginMsg::KeyOnFocus(text) if l.screen == Screen::Code => {
                // Desktop's `iF` with `Hk`: a 6-digit code in the clipboard that was not sent yet goes in, with a toast.
                let text = text.unwrap_or_default();
                let code: String = text.split_whitespace().collect();
                let digits = !text.is_empty() && text.chars().all(|c| c.is_ascii_digit() || c.is_whitespace());
                if digits && code.len() == 6 && !l.tried_codes.contains(&code) && !l.busy {
                    (l.code, l.pasted) = (code, Some(std::time::Instant::now()));
                    return self.update_login(LoginMsg::SubmitCode);
                }
            }
            LoginMsg::KeyOnFocus(text) => {
                let key = crate::verify::clipboard_key(text, &l.tried_keys);
                if let Some(key) = key.filter(|_| l.screen == Screen::RecoveryKey && !l.busy) {
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
                    // Desktop's `ReauthRequiredModal` asks for a sign-in code, then the key is made again.
                    Err(e) if e == beeper_core::matrix::REAUTH => {
                        l.error = Some(e);
                        return self.open_reauth(LoginMsg::NewKey.into());
                    }
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
                    Ok(()) => return self.advance_onboarding(),
                    Err(e) => l.error = Some(e),
                }
            }
            LoginMsg::LostKey => return self.lost_key(),
            LoginMsg::KeysLate => l.late = true,
            LoginMsg::Recovered(r) => {
                l.busy = false;
                l.error = r.err().map(|e| tr("Error: %(message)s").replace("%(message)s", &e));
                return self.advance_onboarding();
            }
            LoginMsg::Done(r) => {
                l.busy = false;
                match r {
                    // The code and username screens show a refusal of the server under the heading, and other
                    // errors in the toast.
                    Err(e) if matches!(l.screen, Screen::Code | Screen::Username) && (400..500).contains(&e.status) => {
                        l.error = Some(e.message);
                    }
                    Err(e) => l.toast = Some(e.message),
                    Ok(Outcome::Requested(request)) => {
                        l.request = request;
                        return self.show_screen(Screen::Code);
                    }
                    Ok(Outcome::Lead(token, suggestions)) => {
                        l.lead_token = token;
                        l.username = suggestions.first().map(|s| username_input(s)).unwrap_or_default();
                        l.suggestions = suggestions;
                        return self.show_screen(Screen::Username);
                    }
                    Ok(Outcome::Read(data)) => {
                        l.import = Some(data);
                        l.screen = Screen::Welcome;
                    }
                    Ok(Outcome::Session(session, profile, accounts)) => match l.import.take() {
                        Some(data) if data.crypto.as_ref().is_some_and(|c| c.user_id != session.user_id) => {
                            let user = &data.crypto.as_ref().expect("checked above").user_id;
                            l.toast = Some(format!("Sign in as {user} to finish the import."));
                            l.import = Some(data);
                        }
                        Some(data) => {
                            let dir = self.store.read().matrix_dir();
                            return self.background(move |_| {
                                if let Some(c) = &data.crypto {
                                    let import = beeper_core::matrix::import_crypto(&dir, c);
                                    tokio::runtime::Handle::current().block_on(import).map_err(local_error)?;
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
                    container(welcome_scene()).height(Fill),
                    actions(
                        vec![
                            container(txt(info, 12.0).color(p.on_secondary).center().width(Fill))
                                .padding(Padding::ZERO.bottom(8))
                                .into(),
                            brand_button(
                                p,
                                label("Start Fresh", None),
                                Brand::Muted,
                                self.login_act(Stop::Fresh),
                                false
                            ),
                            brand_button(
                                p,
                                label("Copy from Beeper Desktop", None),
                                Brand::Primary,
                                self.login_act(Stop::Import),
                                l.busy
                            ),
                        ],
                        24.0
                    ),
                ]
                .into()
            }
            Screen::Welcome => {
                // Chromium's focus ring of a link: a 1 px orange outline.
                let ring = (l.focus.map(|i| l.screen.stops()[i]) == Some(Stop::Terms))
                    .then(|| border::rounded(2).color(Color::from_rgb8(229, 151, 0)).width(1));
                // Desktop's one sentence with the two links in it, wrapped like `text-wrap: balance`.
                let sentence = tr("By continuing, you agree to our %1$s and have read our %2$s.");
                let legal = |s| trx(s, "onboarding legal link label");
                let (terms_label, privacy_label) = (legal("terms of use"), legal("privacy policy"));
                let mut spans = Vec::new();
                let mut rest = sentence;
                while let Some(at) = rest.find("%1$s").into_iter().chain(rest.find("%2$s")).min() {
                    spans.push(span(&rest[..at]).color(p.text));
                    spans.push(match &rest[at..at + 4] {
                        "%1$s" => link(terms_label, UiMessage::OpenUrl(TERMS_URL.into())).border_maybe(ring),
                        _ => link(privacy_label, UiMessage::OpenUrl(PRIVACY_URL.into())),
                    });
                    rest = &rest[at + 4..];
                }
                spans.push(span(rest).color(p.text));
                let plain = sentence.replace("%1$s", terms_label).replace("%2$s", privacy_label);
                let terms = rich_text(spans)
                    .on_link_click(|m: UiMessage| m)
                    .size(11)
                    .line_height(iced::widget::text::LineHeight::Absolute(14.0.into()))
                    .width(crate::style::balanced_width(&plain, 278.0, 11.0))
                    .center();
                let terms = container(terms).center_x(Fill);
                let google = svg(ART[3].clone()).width(18).height(18);
                let google_label = trxf("Continue with %s", "button label", &[&"Google"]);
                let mail =
                    svg(ICONS[0].clone()).width(18).height(18).style(|_, _| svg::Style { color: Some(Color::WHITE) });
                column![
                    container(welcome_scene()).height(Fill),
                    Space::new().height(24),
                    actions(
                        vec![
                            container(terms).padding(Padding::ZERO.bottom(8)).into(),
                            brand_button(
                                p,
                                label(google_label, Some(google.into())),
                                Brand::Muted,
                                self.login_act(Stop::Google),
                                false
                            ),
                            brand_button(
                                p,
                                label(tr("Continue with Email"), Some(mail.into())),
                                Brand::Primary,
                                self.login_act(Stop::Email),
                                false
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
                Space::new().height(FLOAT_LABEL),
                Space::new().height(Fill),
                art(1, 314.0),
                Space::new().height(Fill),
                actions(
                    vec![
                        field(p, EMAIL_INPUT, tr("Email"), &l.email, (!l.busy).then_some(LoginMsg::Email)),
                        brand_button(
                            p,
                            label(trx("Next", "button label"), None),
                            Brand::Primary,
                            self.login_act(Stop::Next),
                            l.busy
                        ),
                    ],
                    24.0
                ),
            ]
            .into(),
            Screen::Code => {
                // Desktop's `OTPInput`: six 41.3x56 slots 8 px apart, with a 2 px ring outside the focused slot, and
                // red digits and rings on all slots after an error.
                let error = l.error.is_some();
                let focused = l.focus.map(|i| l.screen.stops()[i]) == Some(Stop::Field);
                let slots = row((0..6).map(|i| {
                    let ink = if error { ERROR_RED } else { p.strong };
                    let digit: Option<El<'_>> = l
                        .code
                        .chars()
                        .nth(i)
                        .map(|c| semi(c.to_string(), 28.0).line_height(iced::Pixels(36.0)).color(ink).into());
                    let at = focused && i == l.code.len().min(5);
                    let caret = (at && i == l.code.len())
                        .then(|| container(Space::new()).width(1.5).height(28).style(fill_box(ink, 0.0)).into());
                    let edge = match (error, at) {
                        (true, _) => ERROR_RED,
                        (false, true) => p.accent,
                        (false, false) => CLEAR,
                    };
                    let slot = container(digit.or(caret).unwrap_or_else(|| Space::new().into()))
                        .center_x(41.3)
                        .center_y(56)
                        .style(fill_box(if error { CLEAR } else { p.chip }, 12.0));
                    container(slot).padding(2).style(bordered_box(CLEAR, edge, 2.0, 14.0)).into()
                }))
                .spacing(4);
                let capture = text_input("", &l.code)
                    .id(CODE_INPUT)
                    .on_input(|v| LoginMsg::Code(v).into())
                    .size(28)
                    .padding(14)
                    .style(input_style(CLEAR, CLEAR, CLEAR, CLEAR, 0.0));
                let otp = stack![container(slots).center_x(Fill), container(capture).height(60)].height(60);
                // `otpContainer`: the lock animation, 70 px over the heading, and the 4 px under its inline box, with the
                // slots on its bottom edge.
                let lock =
                    stack![column![art(2, 314.0), Space::new().height(4 + 2)], container(otp).align_bottom(Fill)];
                let info = format!("Check {}\nfor your login code", l.email.trim());
                let foot = rich_text![
                    span("Didn't get a code? ").color(p.on_secondary),
                    span(tr("Resend")).color(p.accent).link(UiMessage::Login(LoginMsg::Resend))
                ]
                .on_link_click(move |m: UiMessage| if l.busy { UiMessage::Noop } else { m })
                .size(11)
                .line_height(iced::Pixels(14.0))
                .width(Fill)
                .center();
                // Desktop's floating `small` error, 8 px over the form.
                let error = l.error.as_deref().map(|e| txt(e, 10.4).color(p.text_danger).center().width(Fill));
                let error =
                    container(column![].push(error)).height(ERROR_ROOM - 8.0).align_bottom(Fill).padding([0, 18]);
                // Desktop's `space-between` column with the 70 px overlap: the lock starts 20 px under the top, and
                // takes the room of the error and of the 2 px ring there too, so its gaps are Desktop's.
                let form = column![
                    Space::new().height(20.0 + 2.0 + ERROR_ROOM),
                    Space::new().height(Fill),
                    lock,
                    Space::new().height(Fill),
                    error,
                    Space::new().height(8),
                    actions(
                        vec![
                            container(
                                txt(info, 13.0)
                                    .line_height(iced::Pixels(16.0))
                                    .color(p.on_secondary)
                                    .center()
                                    .width(Fill)
                            )
                            .padding(Padding::ZERO.bottom(8))
                            .into(),
                            brand_button(
                                p,
                                label(tr("Open Email"), None),
                                Brand::Primary,
                                self.login_act(Stop::OpenEmail),
                                l.busy
                            ),
                            container(foot).padding(Padding::ZERO.top(8)).height(36).center_y(36).into(),
                        ],
                        24.0
                    ),
                ];
                let top = column![Space::new().height(50), heading(tr("Enter Code"), 36.0)].height(Fill);
                stack![top, form].into()
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
                // Desktop's continue bar shows once a network account is connected (`hasConnectedNetwork`).
                let go_on = st.has_connected_network().then(|| {
                    let go_on = brand_button(
                        p,
                        label(tr("Continue"), None),
                        Brand::Primary,
                        Some(LoginMsg::Continue.into()),
                        false,
                    );
                    column![
                        container(Space::new()).width(Fill).height(1).style(fill_box(p.border_weak, 0.0)),
                        container(go_on).padding(12)
                    ]
                });
                let remaining = FREE_ACCOUNTS.saturating_sub(st.bridge_accounts.len());
                let foot = txt(
                    trn("%d free account remaining.", "%d free accounts remaining.", remaining, &[&remaining]),
                    12.0,
                )
                .color(p.on_secondary);
                column![
                    {
                        let (headline, width) = gradient_text(tr("Connect Chat Accounts"), 26.0, 48.0);
                        container(svg(headline).width(width).height(48)).center_x(Fill)
                    },
                    Space::new().height(8),
                    container(column![search, divider, grid].push(go_on)).height(Fill).style(card_style(p.card)),
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
            Screen::Username => {
                // Desktop checks a name from 3 characters on, and shows the same text for every name it refuses.
                let valid = (l.username.len() >= 3).then(|| username_ok(&l.username));
                let refused = (valid == Some(false)).then(|| tr("Username is not available. Try something else."));
                let icon = |i: usize, size: f32, color: Color, msg: Option<UiMessage>| -> El<'_> {
                    let glyph = svg(ICONS[i].clone())
                        .width(size)
                        .height(size)
                        .style(move |_, _| svg::Style { color: Some(color) });
                    match msg {
                        Some(m) => button(container(glyph).center(20))
                            .padding(0)
                            .style(btn_style([CLEAR; 3], [color; 3], 4.0))
                            .on_press(m)
                            .into(),
                        None => container(glyph).center(20).into(),
                    }
                };
                // `--color-solids-green` and `--color-on-surface-variant`.
                let (green, grey) = (Color::from_rgb8(25, 186, 59), p.muted);
                let icons = [
                    (valid == Some(true)).then(|| icon(2, 20.0, green, None)),
                    (!l.suggestions.is_empty()).then(|| icon(3, 18.0, grey, Some(LoginMsg::NextSuggestion.into()))),
                    (!l.username.is_empty())
                        .then(|| icon(4, 20.0, grey, Some(LoginMsg::Username(String::new()).into()))),
                ];
                let name = field_with_icons(
                    p,
                    USERNAME_INPUT,
                    tr("Username"),
                    &l.username,
                    (!l.busy).then_some(LoginMsg::Username),
                    icons.into_iter().flatten().collect(),
                );
                // Desktop's floating `small` error, 20 px over the field: a layer, so that it moves nothing.
                let error =
                    l.error.as_deref().or(refused).map(|e| txt(e, 10.4).color(p.text_danger).center().width(Fill));
                let over_form = 20.0 + 38.0 + 8.0 + 33.0 + 52.0;
                let error = column![
                    Space::new().height(Fill),
                    column![].push(error).padding([0, 18]),
                    Space::new().height(over_form)
                ];
                let screen = column![
                    Space::new().height(34),
                    heading(tr("Pick a Username"), 32.0),
                    // The room of the floating label, so that the gaps are Desktop's.
                    Space::new().height(FLOAT_LABEL),
                    Space::new().height(Fill),
                    art(0, 314.0),
                    Space::new().height(Fill),
                    actions(
                        vec![
                            name,
                            brand_button(
                                p,
                                label(trx("Next", "button label"), None),
                                Brand::Primary,
                                self.login_act(Stop::Next),
                                l.busy
                            ),
                        ],
                        52.0
                    ),
                ];
                stack![screen, error].into()
            }
        };
        let ink = p.text;
        let chevron = svg(ICONS[1].clone()).width(18).height(18).style(move |_, _| svg::Style { color: Some(ink) });
        let chevron = button(container(chevron).center(28))
            .padding(0)
            .style(btn_style([CLEAR; 3], [p.text; 3], 6.0))
            .on_press(LoginMsg::Menu.into());
        let controls = container(chevron).align_right(Fill).height(36).padding(Padding::ZERO.top(2).right(2));
        let onboarding = l.screen == Screen::Accounts;
        let page = container(column![controls, body]).width(Fill).height(Fill);
        let page = if onboarding { page } else { page.style(fill_box(p.primary_container, 8.0)) };
        let (pad, frame) = if onboarding { (0.0, p.chip) } else { (8.0, p.window) };
        // Desktop's menu is a native menu, over the toast.
        let menu = l.menu.then(|| {
            let (back, last) = l.screen.menu();
            let item =
                |label: &'static str, hotkey, msg| crate::menu_item(p, None, tr(label).into(), hotkey, Some(msg));
            let items = column![]
                .push(back.map(|s| item("Go Back", "", LoginMsg::Show(s).into())))
                .push(item("Report a Problem", "Ctrl Shift P", UiMessage::OpenUrl(crate::ISSUES_URL.into())))
                .push(last.map(|(label, msg)| item(label, "", msg)));
            let items = container(items.padding([4, 0])).width(210).style(shadow_box(p.menu, p.menu_border, 6.0, 16.0));
            let at = Padding::ZERO.top(34.0 + pad).right(6.0 + pad);
            mouse_area(container(items).align_right(Fill).height(Fill).padding(at)).on_press(LoginMsg::Menu.into())
        });
        stack![container(page).padding(pad).style(fill_box(frame, 0.0))]
            .push(l.toast.as_deref().map(|t| toast(p, t, true)))
            .push(l.pasted().then(|| toast(p, tr("Code auto pasted from clipboard"), false)))
            .push(l.invalid.as_deref().map(bubble))
            .push(menu)
            .into()
    }

    /// What a control does when it is clicked or pressed with the keyboard. A busy screen disables its buttons.
    pub(crate) fn login_act(&self, stop: Stop) -> Option<UiMessage> {
        let msg: UiMessage = match (stop, self.login.screen) {
            (Stop::Menu, _) => return Some(LoginMsg::Menu.into()),
            (Stop::Terms, _) => return Some(UiMessage::OpenUrl(TERMS_URL.into())),
            (Stop::OpenEmail, _) => return Some(LoginMsg::OpenEmail.into()),
            (Stop::Google, _) => LoginMsg::Google.into(),
            (Stop::Email, _) => LoginMsg::Show(Screen::Email).into(),
            (Stop::Resend, _) => LoginMsg::Resend.into(),
            (Stop::Fresh, _) => LoginMsg::Fresh.into(),
            (Stop::Import, _) => LoginMsg::Import.into(),
            (Stop::Field | Stop::Next, Screen::Email) => LoginMsg::SubmitEmail.into(),
            // Desktop disables Next until the name is valid.
            (Stop::Field | Stop::Next, Screen::Username) if username_ok(&self.login.username) => {
                LoginMsg::SubmitUsername.into()
            }
            (Stop::Field | Stop::Next, Screen::Username) => return None,
            (Stop::Field, Screen::Code) => LoginMsg::SubmitCode.into(),
            (stop, screen) => unreachable!("{screen:?} has no {stop:?}"),
        };
        (!self.login.busy).then_some(msg)
    }

    /// A key in the sign-in window: the sign-in hotkeys, Tab and Shift+Tab through `Screen::stops`, Enter on the
    /// focused control or on the form (Desktop's Enter hotkey of the onboarding forms), and Escape on the menu.
    pub fn login_key(&mut self, key: Key, mods: Modifiers) -> Task<UiMessage> {
        if let Some((h, digit)) = self.find_hotkey(&key, mods).filter(|(h, _)| crate::SIGN_IN_HOTKEYS.contains(h)) {
            return self.run_hotkey(h, digit);
        }
        let l = &mut self.login;
        let stops = l.screen.stops();
        match key {
            Key::Named(Named::Escape) => l.menu = false,
            Key::Named(Named::Tab) if !stops.is_empty() && (mods - Modifiers::SHIFT).is_empty() => {
                let n = stops.len();
                let at = match (l.focus, mods.shift()) {
                    (None, false) => 0,
                    (None, true) => n - 1,
                    (Some(i), false) => (i + 1) % n,
                    (Some(i), true) => (i + n - 1) % n,
                };
                l.focus = Some(at);
                // Focusing an ID that no widget has takes the focus from the text field.
                let id = if stops[at] == Stop::Field { l.screen.field().expect("a screen with a field") } else { "" };
                return operation::focus(id);
            }
            Key::Named(Named::Enter) if mods.is_empty() => {
                let stop = l.focus.map(|i| stops[i]).or(l.screen.field().map(|_| Stop::Field));
                return stop.and_then(|s| self.login_act(s)).map_or_else(Task::none, |m| self.update(m));
            }
            _ => {}
        }
        Task::none()
    }

    /// Text typed in the sign-in window outside a text field: Space presses the focused control, and a digit goes
    /// into the code, like Desktop's `keydown` handler of the code screen.
    pub fn login_text(&mut self, text: &str) -> Task<UiMessage> {
        let l = &mut self.login;
        if l.screen == Screen::Code && text.len() == 1 && text.chars().all(|c| c.is_ascii_digit()) {
            l.focus = Screen::Code.stops().iter().position(|s| *s == Stop::Field);
            let code = format!("{}{text}", l.code);
            return Task::batch([self.update_login(LoginMsg::Code(code)), operation::focus(CODE_INPUT)]);
        }
        let stop = l.focus.map(|i| l.screen.stops()[i]).filter(|s| *s != Stop::Field && text == " ");
        stop.and_then(|s| self.login_act(s)).map_or_else(Task::none, |m| self.update(m))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn ease_in_out_like_css() {
        // Chromium's `cubic-bezier(0.42, 0, 0.58, 1)` at 0, 0.25, 0.5, 0.75 and 1.
        for (x, y) in [(0.0, 0.0), (0.25, 0.1291), (0.5, 0.5), (0.75, 0.8709), (1.0, 1.0)] {
            assert!((super::ease_in_out(x) - y).abs() < 1e-3, "{x}: {}", super::ease_in_out(x));
        }
    }

    #[test]
    fn the_welcome_illustration_floats_like_desktop() {
        let mut login = super::Login::default();
        assert!(login.animated(), "the welcome screen ticks its animation");
        drop(super::welcome_scene());
        login.screen = super::Screen::Email;
        assert!(!login.animated(), "the Lottie frames of the other screens stay");
    }

    #[test]
    fn username_rules_like_desktop() {
        assert_eq!(super::username_input("Alex_R.S-2 é"), "alexrs-2");
        for (name, ok) in [
            ("alex", true),
            ("a-b", true),
            ("ab", false),
            ("-ab", false),
            ("ab-", false),
            (&"a".repeat(33), false),
            (&"a".repeat(32), true),
            ("robot", false),
            ("beeperfan", false),
            ("mysupport", false),
            ("theadmin", false),
            ("12345", false),
            ("123a", true),
        ] {
            assert_eq!(super::username_ok(name), ok, "{name}");
        }
    }

    #[test]
    fn email_check_like_chromium() {
        let rows = [
            ("alex@example.com", None),
            ("a.b+c@sub-1.example.co", None),
            ("foo", Some("Please include an '@' in the email address. 'foo' is missing an '@'.")),
            ("@foo", Some("Please enter a part followed by '@'. '@foo' is incomplete.")),
            ("foo@", Some("Please enter a part following '@'. 'foo@' is incomplete.")),
            ("f(o@x.com", Some("A part followed by '@' should not contain the symbol '('.")),
            ("foo@x_y.com", Some("A part following '@' should not contain the symbol '_'.")),
            ("foo@a@b", Some("A part following '@' should not contain the symbol '@'.")),
            ("foo@x..com", Some("'.' is used at a wrong position in 'x..com'.")),
            ("foo@-x.com", Some("Please enter an email address.")),
        ];
        for (v, want) in rows {
            assert_eq!(super::email_mismatch(v).as_deref(), want, "{v}");
        }
    }
}
