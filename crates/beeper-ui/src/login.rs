use crate::focus::Focus;
use crate::i18n::{tr, trx, trxf};
use crate::prelude::*;
use crate::style::gradient_text;
use beeper_core::import::{read_beeper_data, BeeperData};
use beeper_core::models::{BridgeAccountInfo, MatrixSession, Profile};
use beeper_core::store::Setup;
use iced::keyboard::key::Named;
use iced::widget::{column, operation};
use iced::{border, window};

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
const PASSWORD_INPUT: &str = "login-password";
const TERMS_URL: &str = "https://www.beeper.com/terms-onboarding";
const PRIVACY_URL: &str = "https://www.beeper.com/privacy";
/// Desktop's `--color-on-error-container`: the error toast.
const ERROR_RED: Color = Color::from_rgb8(220, 54, 46);
/// Desktop's `Y0.UNKNOWN_ERROR` and `Y0.NO_EMAIL_SUPPORT` of `provisionLogin`.
const UNKNOWN_ERROR: &str = "Something went wrong. Please try again or contact support.";
const NO_EMAIL_LOGIN: &str = "The server does not support email-based login. Try logging in with a username instead.";
/// The Next presses with no email after which Desktop offers its password login.
const PASSWORD_LOGIN_PRESSES: u8 = 10;

/// Desktop's `BasketballSpinner`, the icon of the username field while the server checks the name.
static SPINNER: LazyLock<svg::Handle> = LazyLock::new(|| {
    svg::Handle::from_memory(br##"<svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 18 18"><path fill="#C362FF" stroke="#000" stroke-miterlimit="10" stroke-width=".505" d="M9.074 16.93a7.925 7.925 0 1 0 0-15.85 7.925 7.925 0 0 0 0 15.85Z"/><path stroke="#000" stroke-miterlimit="10" stroke-width=".505" d="M9.348 16.922c2.381-.082 4.19-3.695 4.039-8.070-.15-4.374-2.203-7.854-4.584-7.772S4.614 4.775 4.765 9.150c.15 4.374 2.202 7.853 4.583 7.772ZM8.803 1.084l.544 15.843M1.154 9.276l15.842-.544"/><path fill="#C362FF" fill-rule="evenodd" clip-rule="evenodd" style="mix-blend-mode:multiply" d="M16.996 8.734a7.9 7.9 0 0 0-2.618-5.615A7.926 7.926 0 0 1 2.516 13.454a7.926 7.926 0 0 0 14.48-4.72"/></svg>"##.as_slice())
});

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

/// Desktop's Lottie animations `username.json`, `email.json`, `lock.json` and `key.json`.
static ART: LazyLock<[Arc<crate::lottie::Animation>; 4]> = LazyLock::new(|| {
    [
        include_bytes!("../assets/onboarding/username.json").as_slice(),
        include_bytes!("../assets/onboarding/email.json"),
        include_bytes!("../assets/onboarding/lock.json"),
        include_bytes!("../assets/onboarding/key.json"),
    ]
    .map(|json| Arc::new(crate::lottie::Animation::new(json)))
});

static GOOGLE: LazyLock<svg::Handle> = LazyLock::new(|| svg::Handle::from_memory(GOOGLE_G.as_bytes()));

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
pub(crate) fn ease_in_out(x: f32) -> f32 {
    crate::cubic_bezier(x, [0.42, 0.0, 0.58, 1.0])
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
    container(iced::widget::Stack::with_children(layers).width(SCENE_SIZE.width).height(SCENE_SIZE.height)).center(Fill).into()
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
    /// The password field of the password login.
    Password,
    Next,
    /// "Login with password" or "Use magic code".
    Toggle,
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
    /// The slot of Desktop's `OTPInput` with the focus while the code field has it: each digit is an input of its own.
    pub slot: usize,
    pub username: String,
    /// The usernames that the server suggests, which the sync icon of the field goes through.
    suggestions: Vec<String>,
    /// Desktop's `UsernameInput`: the answer of the check of the name (`None` until it comes), whether the check
    /// runs, and the number of the last change, so that the check of an older name is dropped.
    pub available: Option<bool>,
    pub checking: bool,
    pub check: u32,
    /// Desktop's hidden password login of the email screen: the Next presses with no email, the
    /// "Login with password" link that the tenth one shows, and whether the screen asks for a username and a password.
    empty_nexts: u8,
    pub password_link: bool,
    pub password_login: bool,
    pub user: String,
    pub password: String,
    /// The error under the heading of a screen, and Desktop's red error toast at the top of the window.
    pub error: Option<String>,
    pub toast: Option<String>,
    /// When Desktop's auto paste of a code from the clipboard showed its toast.
    pasted: Option<std::time::Instant>,
    /// The codes sent since the last code request, which the clipboard does not paste again (Desktop's stale codes).
    tried_codes: Vec<String>,
    /// The bubble of Chromium's `type=email` check over the email field.
    pub invalid: Option<String>,
    /// The control of `Screen::stops` that has the keyboard focus.
    pub focus: Focus<Stop>,
    request: String,
    lead_token: String,
    pub busy: bool,
    menu: bool,
    query: String,
    /// Clicks on the Connect Chat Accounts heading: the tenth one finishes the onboarding, like Desktop's.
    headline_clicks: u8,
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
    /// The control with the keyboard focus, when the screen has it.
    fn stop(&self) -> Option<Stop> {
        self.focus.get().filter(|s| self.stops().contains(s))
    }

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

    /// The controls that Tab reaches: the password login adds its field and its link to the email screen.
    fn stops(&self) -> &'static [Stop] {
        match (self.screen, self.password_login, self.password_link) {
            (Screen::Email, true, _) => &[Stop::Menu, Stop::Field, Stop::Password, Stop::Next, Stop::Toggle],
            (Screen::Email, false, true) => &[Stop::Menu, Stop::Field, Stop::Next, Stop::Toggle],
            (screen, ..) => screen.stops(),
        }
    }

    fn focus_on(&mut self, stop: Stop) {
        self.focus.set(self.stops().contains(&stop).then_some(stop));
    }

    /// Desktop's `getFirstAvailableSlotIndex` of `OTPInput`: the first empty slot, or the first slot of a whole code.
    fn first_slot(&self) -> usize {
        match self.code.len() {
            n if n < CODE_SLOTS => n,
            _ => 0,
        }
    }
}

/// The slots of Desktop's code input.
const CODE_SLOTS: usize = 6;

/// Desktop's `handleChange` of `OTPInput`: a digit typed in slot `at` takes its place, and the focus goes to the
/// next slot, but not past the last one.
fn slot_digit(code: &str, at: usize, digit: char) -> (String, usize) {
    let mut digits: Vec<char> = code.chars().collect();
    match digits.get_mut(at) {
        Some(d) => *d = digit,
        None => digits.push(digit),
    }
    (digits.into_iter().collect(), (at + 1).min(CODE_SLOTS - 1))
}

/// Desktop's `handlePaste` of `OTPInput`: the digits of the text fill the slots from `at`, and the focus stays.
fn slot_paste(code: &str, at: usize, text: &str) -> String {
    let mut digits: Vec<char> = code.chars().collect();
    for (j, c) in (at..CODE_SLOTS).zip(text.chars().filter(char::is_ascii_digit)) {
        match digits.get_mut(j) {
            Some(d) => *d = c,
            None => digits.push(c),
        }
    }
    digits.into_iter().collect()
}

/// Desktop's `handleKeyDown` of `OTPInput` on slot `at` of `code`: the code and the slot with the focus after the
/// key, or `None` for a key that the slots leave to Chromium (Tab on the last slot).
fn slot_key(code: &str, at: usize, key: Named) -> Option<(String, usize)> {
    let mut digits: Vec<char> = code.chars().collect();
    let filled = at < digits.len();
    let slot = match key {
        // `q[j] = ""`: the digit of the slot goes and the code closes up, and the focus goes back a slot.
        Named::Backspace => {
            if filled {
                digits.remove(at);
            }
            at.saturating_sub(1)
        }
        // `Hzt`: the digits after the slot move into it, and an empty slot loses the last digit.
        Named::Delete => {
            if !digits.is_empty() {
                digits.remove(at.min(digits.len() - 1));
            }
            at
        }
        Named::ArrowLeft => at.saturating_sub(1),
        // → and Tab go only from a slot with a digit.
        Named::ArrowRight | Named::Tab if at < CODE_SLOTS - 1 => at + usize::from(filled),
        Named::ArrowRight => at,
        _ => return None,
    };
    Some((digits.into_iter().collect(), slot))
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
    /// The username and the password of the password login, and its "Login with password" or "Use magic code".
    User(String),
    Password(String),
    TogglePassword,
    SubmitEmail,
    Code(String),
    /// Text typed (`false`) or pasted (`true`) into the focused slot of the code.
    Slot(String, bool),
    /// A press on the slots of the code.
    SlotsPressed,
    Resend,
    Username(String),
    /// The debounced check of the username of a change, and its answer.
    CheckUsername(u32),
    UsernameChecked(u32, Result<bool, CloudError>),
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
    /// A click on the Connect Chat Accounts heading.
    Headline,
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
    /// "Reset Recovery Key": Desktop's `deleteAllBridges` first, then a new key.
    ConfirmReset,
    BridgesDeleted(Result<(), String>),
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
    CloudError { status: 0, message, errcode: None, body: None }
}

/// Desktop's `uge`: the text of a refused sign-in code. After two wrong codes it is "Invalid code. Please try again.";
/// before, the message of the `BeeperAPIError`, or of a failed request, or "Invalid code. Please try again or
/// contact support." without one. The code screen shows it under the heading, or in the toast for a 5xx or a
/// request that got no answer.
fn code_error(e: CloudError) -> CloudError {
    let message = match (e.retries(), e.status) {
        (2.., _) => tr("Invalid code. Please try again.").to_string(),
        (_, 0) => e.message.clone(),
        _ => e.api_message(),
    };
    let message = match message.is_empty() {
        true => tr("Invalid code. Please try again or contact support.").to_string(),
        false => message,
    };
    CloudError { message, ..e }
}

fn session(cloud: &Cloud, jwt: &str) -> Result<Outcome, CloudError> {
    cloud.finish(jwt).map(|(s, p, a)| Outcome::Session(s, p, a))
}

/// Desktop's password login, with its texts for the Matrix error codes of a refusal.
fn password_session(cloud: &Cloud, user: &str, password: &str) -> Result<Outcome, CloudError> {
    let session = cloud.password_login(user, password).map_err(|e| {
        let code = e.errcode.as_deref().unwrap_or_default();
        let message = match () {
            _ if code.contains("M_UNKNOWN") => "Please enter a valid username.",
            _ if code.contains("M_INVALID_PARAM") => "Please enter a valid password.",
            _ if code.contains("M_FORBIDDEN") => "Incorrect username or password. Please try again.",
            _ => "Something went wrong confirming your credentials, try again or contact support.",
        };
        CloudError { message: tr(message).into(), ..e }
    })?;
    let (profile, accounts) = cloud.whoami(&session.access_token)?;
    Ok(Outcome::Session(session, profile, accounts))
}

fn card_style(bg: Color) -> impl Fn(&Theme) -> container::Style {
    let shadow = iced::Shadow { color: Color::from_rgba(0.0, 0.0, 0.0, 0.03), offset: iced::Vector::new(0.0, 2.0), blur_radius: 4.0 };
    move |_| container::Style { background: Some(bg.into()), border: border::rounded(12), shadow, ..Default::default() }
}

/// Desktop's onboarding error toast: 300x32 at the top of the window, with an info icon and a close button.
/// Desktop's `showToast`: a neutral toast with centered text and no close button, which goes after 3 s.
fn toast<'a>(p: &Palette, text: &'a str, error: bool) -> El<'a> {
    let (bg, fg) = if error { (ERROR_RED, Color::WHITE) } else { (p.card, p.strong) };
    let text = txt(text, 13.0).line_height(iced::Pixels(rem(16.0))).color(fg).width(Fill);
    let line: El<'a> = match error {
        true => {
            let close = button(ic(icon::CLOSE, 14.0).color(fg)).bare(fg).padding(3).on_press(LoginMsg::CloseToast.into());
            row![ic(icon::INFO, 20.0).color(fg), text, close].spacing(6).align_y(Center).into()
        }
        false => text.center().into(),
    };
    let card = container(line).padding([0, 6]).width(300).center_y(32).style(glow_box(bg, p.shadow, 10.0, 1.0, 10.0));
    container(card).center_x(Fill).padding(Padding::ZERO.top(44)).into()
}

/// Chromium's validation bubble over the email field, with its orange "!" icon and its arrow.
fn bubble(text: &str) -> El<'_> {
    let mark = container(bold("!", 15.0).color(Color::WHITE)).center(22).style(fill_box(Color::from_rgb8(232, 89, 12), 2.0));
    let line = row![mark, txt(text, 14.0).line_height(iced::Pixels(rem(16.0))).color(Color::BLACK).width(Fill)].spacing(8);
    let card = container(line).padding(8).width(162).style(shadow_box(Color::WHITE, Color::from_rgb8(160, 160, 160), 2.0, 4.0));
    let arrow = svg(BUBBLE_ARROW.clone()).width(16).height(9);
    let tip = container(column![card, container(arrow).padding(Padding::ZERO.left(8))]);
    container(tip).align_bottom(Fill).padding(Padding::ZERO.left(85).bottom(112)).into()
}

/// Desktop's `--gradient-brand`, `linear-gradient(190.1deg, #6953f2 -7.24%, #0c52f9 109.66%)`, or the one of a
/// monochrome theme at 190 degrees.
/// iced spans a gradient over a shorter line than CSS, so the stops are the CSS colors at the ends of the iced line
/// in a 278x36 button.
pub(crate) fn brand_gradient(p: &Palette) -> Background {
    lit_gradient(p, 1.0)
}

/// `brand_gradient` made `k` times brighter, like `filter: brightness(k)`.
fn lit_gradient(p: &Palette, k: f32) -> Background {
    let lit = |c: Color| Color::from_rgb((c.r * k).min(1.0), (c.g * k).min(1.0), (c.b * k).min(1.0));
    let c = |r: f32, g: f32, b: f32| lit(Color::from_rgb(r / 255.0, g / 255.0, b / 255.0));
    let g = match p.brand_stops {
        None => Linear::new(Degrees(190.1)).add_stop(0.0, c(82.5, 82.8, 243.7)).add_stop(1.0, c(36.4, 82.3, 247.2)),
        Some(stops) => crate::ui_theme::brand_line(stops, Size::new(278.0, 36.0), lit),
    };
    Background::Gradient(Gradient::Linear(g))
}

/// Desktop's `BrandButton` in `size: "small"`: a full-width pill around `content`, 8 px above and below it. Without
/// a message it is disabled. While `loading` it is disabled and shows three bouncing dots over its hidden content.
pub(crate) fn brand_button<'a>(p: &Palette, content: El<'a>, variant: Brand, msg: Option<UiMessage>, loading: bool) -> El<'a> {
    let msg = msg.filter(|_| !loading);
    let (bg, fg, edge) = match (msg.is_some(), variant) {
        (false, _) => (Some(p.chip), p.on_secondary, CLEAR),
        (true, Brand::Primary) => (None, Color::WHITE, CLEAR),
        (true, Brand::Secondary) => (Some(CLEAR), p.brand, p.brand),
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
    let gradients = [lit_gradient(p, 1.0), lit_gradient(p, 1.05)];
    // `:not(:disabled):active { transform: scale(.98) }`.
    let pressable = msg.is_some();
    let button = button(stack![container(content).center_x(Fill)].push(dots))
        .width(Fill)
        .padding([9, 11])
        .style(move |_: &Theme, s| {
            // `filter: brightness(1.05)` on hover.
            let k = if hover && s == button::Status::Hovered { 1.05 } else { 1.0 };
            let lift = |c: Color| Color { r: (c.r * k).min(1.0), g: (c.g * k).min(1.0), b: (c.b * k).min(1.0), a: c.a };
            button::Style {
                background: Some(bg.map_or_else(|| gradients[usize::from(k > 1.0)], |c| lift(c).into())),
                text_color: if loading { CLEAR } else { lift(fg) },
                border: border::rounded(9999).color(edge).width(1),
                ..Default::default()
            }
        })
        .on_press_maybe(msg);
    match pressable {
        true => crate::lift::press(button, 0.98).into(),
        false => button.into(),
    }
}

pub(crate) fn actions(items: Vec<El<'_>>, bottom: f32) -> El<'_> {
    column(items).spacing(8).padding(Padding::new(0.0).left(18.0).right(18.0).bottom(bottom)).into()
}

/// The content of a `BrandButton`: 13 px medium text, after an 18 px icon with an 8 px gap.
fn label<'a>(s: impl iced::widget::text::IntoFragment<'a>, lead: Option<El<'a>>) -> El<'a> {
    let text = med(s, 13.0).line_height(iced::Pixels(rem(14.95)));
    row![].push(lead).push(text).spacing(8).align_y(Center).into()
}

/// Desktop's `h3.BrandHeading-module__heading`, 1.625rem: the headings of the recovery key, the sync and Connect Chat
/// Accounts.
pub(crate) const H3: f32 = 26.0;

/// Desktop's `BrandHeading` with `gradient` on a 40 px line.
pub(crate) fn heading<'a>(s: &str, size: f32) -> El<'a> {
    let (text, width) = gradient_text(s, size, 40.0);
    container(svg(text).width(width).height(40)).center_x(Fill).into()
}

/// Desktop's `Lottie` of a screen: 0 username, 1 email, 2 lock and 3 key.
pub(crate) fn art<'a>(i: usize, height: impl Into<iced::Length>) -> El<'a> {
    crate::lottie::lottie(&ART[i], height)
}

/// Desktop's `BrandInput` with `floatingPlaceholder`: text in the color of the placeholder, and the placeholder
/// as a small chip over the top edge once the field has text. Enter submits the form through the key handler of
/// the window, like Desktop's Enter hotkey.
pub(crate) fn field<'a, M: Into<UiMessage> + 'a>(
    p: &Palette, id: &'static str, hint: &'a str, value: &'a str, on: Option<fn(String) -> M>,
) -> El<'a> {
    field_with_icons(p, id, hint, value, on, Vec::new())
}

/// A `field` with `BrandInput`'s `rightIcons`: icons 8 px from the right edge, 2 px apart.
fn field_with_icons<'a, M: Into<UiMessage> + 'a>(
    p: &Palette, id: &'static str, hint: &'a str, value: &'a str, on: Option<fn(String) -> M>, icons: Vec<El<'a>>,
) -> El<'a> {
    let room = if icons.is_empty() { 12.0 } else { 8.0 + 22.0 * icons.len() as f32 };
    let input = brand_input(p, id, hint, value, on).padding(Padding::new(10.0).left(12.0).right(room));
    let icons = (!icons.is_empty())
        .then(|| container(row(icons).spacing(2).align_y(Center)).align_right(Fill).center_y(38).padding(Padding::ZERO.right(8)));
    let input = stack![input].push(icons);
    // `translate(26px, -60%) scale(0.6875)` of the label, with its origin at the left middle of the field.
    let chip = (!value.is_empty()).then(|| {
        let hint = txt(hint, 14.0 * 0.6875).line_height(iced::Pixels(rem(18.0 * 0.6875))).color(p.on_secondary);
        let chip = container(hint).padding([10.0 * 0.6875, 12.0 * 0.6875]).style(fill_box(p.card, 99.0));
        container(chip).padding(Padding::ZERO.left(26))
    });
    stack![column![Space::new().height(FLOAT_LABEL), input]].push(chip).into()
}

/// The text field of Desktop's `BrandInput`, 38 px high.
fn brand_input<'a, M: Into<UiMessage> + 'a>(
    p: &Palette, id: &'static str, hint: &'a str, value: &'a str, on: Option<fn(String) -> M>,
) -> text_input::TextInput<'a, UiMessage, Theme> {
    text_input(hint, value)
        .id(id)
        .on_input_maybe(on.map(|on| move |v| on(v).into()))
        .size(rem(14.0))
        .line_height(iced::Pixels(rem(18.0)))
        .padding(Padding::new(10.0).left(12.0).right(12.0))
        .style(input_style(p.chip, p.on_secondary, p.on_secondary, p.accent, 19.0))
}

impl BeeperGuiApp {
    pub fn login_window_settings() -> window::Settings {
        let settings = Self::window_settings(SIZE);
        window::Settings { resizable: false, position: window::Position::Centered, ..settings }
    }

    pub fn refresh_cloud_accounts(&self) -> Task<UiMessage> {
        let (cloud, token) = (self.cloud.clone(), self.store.read().beeper_state.access_token.clone());
        Task::perform(async move { tokio::task::spawn_blocking(move || cloud.whoami(&token)).await.expect("whoami worker thread") }, |r| {
            UiMessage::Accounts(accounts::Msg::Cloud(r.map_err(|e| e.message)))
        })
    }

    /// Desktop's `showHomePage` after the onboarding: the same window becomes the main window, resizable, at
    /// 1200x800, and shows `chat`.
    pub fn enter_app(&mut self, chat: &str) -> Task<UiMessage> {
        let id = self.login_window.take().expect("login window is open");
        self.select_chat(chat);
        self.main_window = Some(id);
        Task::batch([window::set_resizable(id, true), self.resize_around(id, crate::MAIN_SIZE)])
    }

    /// Desktop's `InnerApp`: when encrypted messaging of this device goes back to the verification or to the keys, the
    /// main window shows the onboarding again at the onboarding size.
    pub fn leave_app(&mut self) -> Task<UiMessage> {
        let stage = self.store.read().beeper_state.stage();
        let back = matches!(stage, "pre-verification" | "awaiting-secrets" | "awaiting-key-backup");
        let Some(id) = self.main_window.take_if(|_| back && self.login_window.is_none()) else { return Task::none() };
        (self.login_window, self.login) = (Some(id), Login::default());
        Task::batch([window::set_resizable(id, false), self.resize_around(id, SIZE), self.resume_onboarding()])
    }

    /// Desktop's onboarding of a signed-in device before the stage "done": it starts at the first onboarding screen
    /// and checks encrypted messaging.
    pub fn resume_onboarding(&mut self) -> Task<UiMessage> {
        self.login.screen = Screen::Email;
        self.check_e2ee()
    }

    fn background(&mut self, f: impl FnOnce(&Cloud) -> Result<Outcome, CloudError> + Send + 'static) -> Task<UiMessage> {
        self.login.busy = true;
        (self.login.error, self.login.toast) = (None, None);
        let cloud = self.cloud.clone();
        Task::perform(async move { tokio::task::spawn_blocking(move || f(&cloud)).await.expect("login window worker thread") }, |r| {
            LoginMsg::Done(r).into()
        })
    }

    /// Shows `screen` with its text field focused, like Desktop's `autoFocus`.
    fn show_screen(&mut self, screen: Screen) -> Task<UiMessage> {
        let l = &mut self.login;
        (l.screen, l.error, l.toast, l.invalid) = (screen, None, None, None);
        // Desktop mounts a new email form, so the screen shows empty, without its password login.
        if screen == Screen::Email {
            l.email.clear();
            (l.empty_nexts, l.password_link, l.password_login) = (0, false, false);
        }
        l.focus_on(Stop::Field);
        l.slot = l.first_slot();
        if screen == Screen::Verify {
            return self.start_login_verification();
        }
        screen.field().map_or_else(Task::none, operation::focus)
    }

    /// Desktop's `authenticationMode` of the email screen: a switch clears the toast and the fields of the other
    /// mode, and focuses the first field.
    fn set_password_login(&mut self, on: bool) -> Task<UiMessage> {
        let l = &mut self.login;
        (l.password_login, l.toast, l.invalid) = (on, None, None);
        if on {
            l.email.clear();
        } else {
            (l.user, l.password) = Default::default();
        }
        l.focus_on(Stop::Field);
        operation::focus(EMAIL_INPUT)
    }

    /// Desktop's `UsernameInput`: a name of 3 characters or more is checked 300 ms after its last change, while the
    /// field shows a spinner.
    fn check_username(&mut self) -> Task<UiMessage> {
        let l = &mut self.login;
        l.check += 1;
        if l.username.len() < 3 {
            (l.checking, l.available) = (false, None);
            self.timers.username.cancel();
            return Task::none();
        }
        l.checking = true;
        let check = LoginMsg::CheckUsername(l.check).into();
        self.timers.username.after(std::time::Duration::from_millis(300), check)
    }

    pub fn update_login(&mut self, msg: LoginMsg) -> Task<UiMessage> {
        let l = &mut self.login;
        l.menu = matches!(msg, LoginMsg::Menu) && !l.menu;
        match msg {
            LoginMsg::Import => {
                let dir = l.source.clone().expect("import screen has a Beeper Desktop dir");
                return self.background(move |_| read_beeper_data(&dir).map(|d| Outcome::Read(Arc::new(d))).map_err(local_error));
            }
            LoginMsg::Fresh => {
                self.store.write().finish_setup(Setup::Fresh);
                self.login = Login::default();
            }
            LoginMsg::Show(s) => return self.show_screen(s),
            LoginMsg::Google => l.toast = Some("Google sign-in is not available in beeper-rs. Use email.".into()),
            msg @ (LoginMsg::Email(..)
            | LoginMsg::User(..)
            | LoginMsg::Password(..)
            | LoginMsg::TogglePassword
            | LoginMsg::SubmitEmail
            | LoginMsg::Resend
            | LoginMsg::Code(..)
            | LoginMsg::Slot(..)
            | LoginMsg::SlotsPressed
            | LoginMsg::SubmitCode
            | LoginMsg::Username(..)
            | LoginMsg::CheckUsername(..)
            | LoginMsg::UsernameChecked(..)
            | LoginMsg::NextSuggestion
            | LoginMsg::SubmitUsername) => return self.login_email(msg),
            LoginMsg::Menu => {}
            LoginMsg::CloseToast => l.toast = None,
            LoginMsg::OpenEmail => return open_email_client(),
            LoginMsg::StartOver => self.login = Login::new(l.source.take()),
            LoginMsg::Query(q) => l.query = q,
            // Desktop's tenth click on the heading finishes the onboarding, without its Continue.
            LoginMsg::Headline => {
                l.headline_clicks += 1;
                if l.headline_clicks == 10 {
                    self.login = Login::default();
                    return self.enter_app(NOTE_CHAT_ID);
                }
            }
            // Desktop's Continue selects the first read chat (`selectFirstReadThread`), and a second later shows its
            // toast of the mobile apps.
            LoginMsg::Continue => {
                self.login = Login::default();
                let first_read = self.listed_chats(&self.store.read()).iter().find(|c| !c.is_unread()).map(|c| c.id.clone());
                let enter = self.enter_app(first_read.as_deref().unwrap_or(NOTE_CHAT_ID));
                let later = async { tokio::time::sleep(std::time::Duration::from_secs(1)).await };
                let toast = Task::perform(later, |()| UiMessage::Toast(crate::toasts::Msg::MobileApps(true)));
                return Task::batch([enter, toast]);
            }
            LoginMsg::E2ee(r) => return self.e2ee_checked(r),
            LoginMsg::AnotherWay => l.another_way = true,
            msg @ (LoginMsg::UseRecoveryKey
            | LoginMsg::KeyEdit(..)
            | LoginMsg::PasteKey
            | LoginMsg::Pasted(..)
            | LoginMsg::KeyOnFocus(..)
            | LoginMsg::SubmitRecoveryKey
            | LoginMsg::NewKey
            | LoginMsg::ConfirmReset
            | LoginMsg::BridgesDeleted(..)
            | LoginMsg::MadeKey(..)
            | LoginMsg::CopyKey
            | LoginMsg::KeyNext
            | LoginMsg::SubmitConfirmKey
            | LoginMsg::LostKey
            | LoginMsg::KeysLate
            | LoginMsg::Recovered(..)) => return self.login_keys(msg),
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
                        return Task::batch([self.show_screen(Screen::Username), self.check_username()]);
                    }
                    Ok(Outcome::Read(data)) => {
                        l.import = Some(data);
                        l.screen = Screen::Welcome;
                    }
                    // The import stays until it is done, so that a sign-in after a refusal or a failed import of
                    // the keys copies the data too.
                    Ok(Outcome::Session(session, profile, accounts)) => match l.import.clone() {
                        Some(data) if data.user_id.as_ref().is_some_and(|u| *u != session.user_id) => {
                            let user = data.user_id.as_deref().expect("checked above");
                            l.toast = Some(format!("Sign in as {user} to finish the import."));
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
                        l.import = None;
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

    /// The recovery key steps of sign-in: entering, resetting and saving a key.
    fn login_keys(&mut self, msg: LoginMsg) -> Task<UiMessage> {
        let l = &mut self.login;
        match msg {
            LoginMsg::UseRecoveryKey => return self.use_recovery_key(),
            LoginMsg::KeyEdit(action) => {
                crate::verify::edit_recovery_key(&mut l.key_input, action);
                l.error = None;
            }
            LoginMsg::PasteKey => return iced::clipboard::read().map(|t| LoginMsg::Pasted(t).into()),
            LoginMsg::Pasted(text) => {
                l.key_input = text_editor::Content::with_text(&crate::verify::format_recovery_key(&text.unwrap_or_default()));
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
            LoginMsg::ConfirmReset => {
                l.busy = true;
                let (cloud, token) = (self.cloud.clone(), self.store.read().beeper_state.access_token.clone());
                let delete = async move {
                    tokio::task::spawn_blocking(move || cloud.delete_all_bridges(&token).map_err(|e| e.message))
                        .await
                        .expect("delete bridges worker thread")
                };
                return Task::perform(delete, |r| LoginMsg::BridgesDeleted(r).into());
            }
            LoginMsg::BridgesDeleted(Ok(())) => return self.make_recovery_key(),
            LoginMsg::BridgesDeleted(Err(e)) => {
                eprintln!("[deleteAllBridges]:: {e}");
                l.busy = false;
                l.toast = Some(tr("Failed to reset. Please check your connection and try again.").into());
            }
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
            LoginMsg::SubmitConfirmKey => match beeper_core::matrix::confirm_recovery_key(&self.store, &l.key_input.text()) {
                Ok(()) => return self.advance_onboarding(),
                Err(e) => l.error = Some(e),
            },
            LoginMsg::LostKey => return self.lost_key(),
            LoginMsg::KeysLate => l.late = true,
            LoginMsg::Recovered(r) => {
                l.busy = false;
                l.error = r.err().map(|e| tr("Error: %(message)s").replace("%(message)s", &e));
                return self.advance_onboarding();
            }
            _ => unreachable!("not a login_keys message"),
        }
        Task::none()
    }

    /// The email, code and username steps of sign-in.
    fn login_email(&mut self, msg: LoginMsg) -> Task<UiMessage> {
        let l = &mut self.login;
        match msg {
            LoginMsg::Email(v) => {
                l.email = v;
                (l.toast, l.invalid) = (None, None);
                l.focus_on(Stop::Field);
            }
            LoginMsg::User(v) => {
                (l.user, l.toast) = (v, None);
                l.focus_on(Stop::Field);
            }
            LoginMsg::Password(v) => {
                (l.password, l.toast) = (v, None);
                l.focus_on(Stop::Password);
            }
            LoginMsg::TogglePassword => {
                let on = !l.password_login;
                return self.set_password_login(on);
            }
            LoginMsg::SubmitEmail if l.password_login => {
                let (user, password) = (l.user.clone(), l.password.clone());
                return self.background(move |c| password_session(c, &user, &password));
            }
            LoginMsg::SubmitEmail | LoginMsg::Resend => {
                let email = l.email.trim().to_string();
                if email.is_empty() {
                    l.toast = Some(tr("Please enter your email.").into());
                    // Desktop's tenth Next with no email shows its password login.
                    l.empty_nexts = l.empty_nexts.saturating_add(1);
                    if l.empty_nexts == PASSWORD_LOGIN_PRESSES {
                        l.password_link = true;
                        return self.set_password_login(true);
                    }
                    return Task::none();
                }
                if let Some(e) = email_mismatch(&email) {
                    l.invalid = Some(e);
                    return operation::focus(EMAIL_INPUT);
                }
                (l.code, l.slot) = (String::new(), 0);
                l.tried_codes.clear();
                return self.background(move |c| {
                    // Desktop's `provisionLogin`: a server without the email login, or any failure, refuses.
                    let unknown = |_| local_error(tr(UNKNOWN_ERROR).into());
                    let (request, methods) = c.start_login().map_err(unknown)?;
                    if !methods.iter().any(|m| m == "email") {
                        return Err(local_error(tr(NO_EMAIL_LOGIN).into()));
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
            LoginMsg::Slot(text, pasted) => {
                // Desktop's paste of a whole code anywhere in the window (`onPasteText`) sends it.
                let whole: String = text.split_whitespace().collect();
                let code = match pasted {
                    true if whole.len() == CODE_SLOTS && whole.chars().all(|c| c.is_ascii_digit()) => whole,
                    true => slot_paste(&l.code, l.slot, &text),
                    // A digit takes the place of the digit of the slot, and other text goes nowhere.
                    false => match text.chars().last().filter(char::is_ascii_digit) {
                        Some(d) => {
                            let (code, slot) = slot_digit(&l.code, l.slot, d);
                            l.slot = slot;
                            code
                        }
                        None => return Task::none(),
                    },
                };
                return self.update_login(LoginMsg::Code(code));
            }
            // Desktop's `handleFocus`: a press on the slots clears a refused code and focuses the first empty slot.
            LoginMsg::SlotsPressed => {
                l.focus_on(Stop::Field);
                let clear = l.error.is_some().then(|| self.update_login(LoginMsg::Code(String::new())));
                self.login.slot = self.login.first_slot();
                return Task::batch(clear.into_iter().chain([operation::focus(CODE_INPUT)]));
            }
            LoginMsg::SubmitCode if l.busy => {}
            LoginMsg::SubmitCode => {
                let (request, code) = (l.request.clone(), l.code.clone());
                l.tried_codes.push(code.clone());
                return self.background(move |c| match c.send_login_code(&request, &code).map_err(code_error)? {
                    CodeResult::Token(jwt) => session(c, &jwt),
                    CodeResult::Registration { lead_token, username_suggestions } => Ok(Outcome::Lead(lead_token, username_suggestions)),
                });
            }
            LoginMsg::Username(v) => {
                let name = username_input(&v);
                (l.error, l.toast) = (None, None);
                if name != l.username {
                    l.username = name;
                    return self.check_username();
                }
            }
            // Desktop's `qHt` after the local rules (`Pzt`), which refuse a name without asking the server.
            LoginMsg::CheckUsername(n) if n == l.check => {
                (l.available, l.toast) = (None, None);
                if !username_ok(&l.username) {
                    (l.available, l.checking) = (Some(false), false);
                    return Task::none();
                }
                let (cloud, name) = (self.cloud.clone(), l.username.clone());
                let ask = async move {
                    tokio::task::spawn_blocking(move || cloud.username_available(&name)).await.expect("username check worker thread")
                };
                return Task::perform(ask, move |r| LoginMsg::UsernameChecked(n, r).into());
            }
            LoginMsg::UsernameChecked(n, r) if n == l.check => {
                l.checking = false;
                match r {
                    Ok(available) => l.available = Some(available),
                    Err(e) => {
                        eprintln!("Error validating username: {}", e.message);
                        let text = match e.message.is_empty() {
                            true => tr("Error checking username availability. Please try again.").into(),
                            false => e.message,
                        };
                        (l.available, l.toast) = (None, Some(text));
                    }
                }
            }
            LoginMsg::CheckUsername(_) | LoginMsg::UsernameChecked(..) => {}
            LoginMsg::NextSuggestion => {
                let at = l.suggestions.iter().position(|s| *s == l.username).map_or(0, |i| i + 1);
                let name = l.suggestions[at % l.suggestions.len()].clone();
                return self.update_login(LoginMsg::Username(name));
            }
            LoginMsg::SubmitUsername => {
                let (request, lead, name) = (l.request.clone(), l.lead_token.clone(), l.username.clone());
                return self.background(move |c| session(c, &c.register(&request, &lead, &name)?));
            }
            _ => unreachable!("not a login_email message"),
        }
        Task::none()
    }

    pub fn view_login(&self) -> El<'_> {
        let (l, p) = (&self.login, &self.pal);
        let body: El<'_> = match l.screen {
            Screen::Import => self.view_login_import(),
            Screen::Welcome => self.view_login_welcome(),
            Screen::Email => self.view_login_email(),
            Screen::Code => self.view_login_code(),
            Screen::Accounts => self.view_login_accounts(),
            Screen::Verify => self.view_onboarding_verify(),
            Screen::RecoveryKey => self.view_onboarding_key(),
            Screen::Syncing => self.view_onboarding_syncing(),
            Screen::Preamble => self.view_onboarding_preamble(),
            Screen::NewKey | Screen::ConfirmKey => self.view_onboarding_new_key(),
            Screen::Reset => self.view_onboarding_reset(),
            Screen::Username => self.view_login_username(),
        };
        let ink = p.text;
        let chevron = svg(ICONS[1].clone()).width(18).height(18).style(move |_, _| svg::Style { color: Some(ink) });
        let chevron = button(container(chevron).center(28)).bare(p.text).on_press(LoginMsg::Menu.into());
        let controls = container(chevron).align_right(Fill).height(36).padding(Padding::ZERO.top(2).right(2));
        let onboarding = l.screen == Screen::Accounts;
        let page = container(column![controls, body]).width(Fill).height(Fill);
        let page = if onboarding { page } else { page.style(fill_box(p.primary_container, 8.0)) };
        let (pad, frame) = if onboarding { (0.0, p.chip) } else { (8.0, p.window) };
        // Desktop's menu is a native menu, over the toast.
        let menu = l.menu.then(|| {
            let (back, last) = l.screen.menu();
            let item = |label: &'static str, hotkey: String, msg| crate::menu_item(p, None, tr(label).into(), hotkey, Some(msg));
            let report = crate::menubar::accelerator(self.store.read().settings.hotkey_keys("Report a Problem"));
            let items = column![]
                .push(back.map(|s| item("Go Back", String::new(), LoginMsg::Show(s).into())))
                .push(item("Report a Problem", report, UiMessage::OpenUrl(crate::ISSUES_URL.into())))
                .push(last.map(|(label, msg)| item(label, String::new(), msg)));
            let items = crate::menu_card(p, items, 240);
            let at = Padding::ZERO.top(34.0 + pad).right(6.0 + pad);
            crate::backdrop(container(items).align_right(Fill).height(Fill).padding(at)).on_press(LoginMsg::Menu.into())
        });
        // `onboarding-module__frame` is `--left-pane-bg`.
        let backdrop = p.backdrop.filter(|_| !onboarding);
        let window = stack![crate::ui_theme::window_bg(container(page).padding(pad), backdrop, frame)]
            .push(l.toast.as_deref().map(|t| toast(p, t, true)))
            .push(l.pasted().then(|| toast(p, tr("Code auto pasted from clipboard"), false)))
            .push(l.invalid.as_deref().map(bubble))
            .push(menu)
            .push(self.view_network_menu());
        // Desktop's menu bar and Keyboard Shortcuts modal, which its onboarding window has too.
        let (window, menu_drop) = self.with_menu_bar(window.into());
        let hotkeys = self.hotkeys_modal.as_deref().map(|q| self.view_hotkeys_modal(q, SIZE));
        stack![window].push(menu_drop).push(hotkeys).into()
    }

    /// The import screen, when Beeper Desktop data was found: Start Fresh, or copy its account and chats.
    fn view_login_import(&self) -> El<'_> {
        let (l, p) = (&self.login, &self.pal);
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
                    container(txt(info, 12.0).color(p.on_secondary).center().width(Fill)).padding(Padding::ZERO.bottom(8)).into(),
                    brand_button(p, label("Start Fresh", None), Brand::Muted, self.login_act(Stop::Fresh), false),
                    brand_button(p, label("Copy from Beeper Desktop", None), Brand::Primary, self.login_act(Stop::Import), l.busy),
                ],
                24.0
            ),
        ]
        .into()
    }

    /// The welcome screen: the terms, and Continue with Google or with Email.
    fn view_login_welcome(&self) -> El<'_> {
        let p = &self.pal;
        let google = svg(GOOGLE.clone()).width(18).height(18);
        let google_label = trxf("Continue with %s", "button label", &[&"Google"]);
        let mail = svg(ICONS[0].clone()).width(18).height(18).style(|_, _| svg::Style { color: Some(Color::WHITE) });
        column![
            container(welcome_scene()).height(Fill),
            Space::new().height(24),
            actions(
                vec![
                    container(self.view_login_terms()).padding(Padding::ZERO.bottom(8)).into(),
                    brand_button(p, label(google_label, Some(google.into())), Brand::Muted, self.login_act(Stop::Google), false),
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

    /// Desktop's one sentence of the welcome screen with the links to the terms and the privacy policy in it, wrapped
    /// like `text-wrap: balance`.
    fn view_login_terms(&self) -> El<'_> {
        let p = &self.pal;
        let link = |s: &'static str, m: UiMessage| span(s).color(p.accent).link(m);
        // Chromium's focus ring of a link: a 1 px orange outline.
        let ring = (self.login.stop() == Some(Stop::Terms)).then(|| border::rounded(2).color(Color::from_rgb8(229, 151, 0)).width(1));
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
            .size(rem(11.0))
            .line_height(iced::widget::text::LineHeight::Absolute(rem(14.0).into()))
            .width(crate::style::balanced_width(&plain, 278.0, 11.0))
            .center();
        container(terms).center_x(Fill).into()
    }

    /// The email screen: the email, or the username and the password of a password login, and Next.
    fn view_login_email(&self) -> El<'_> {
        let (l, p) = (&self.login, &self.pal);
        let free = !l.busy;
        let first = match l.password_login {
            true => field(p, EMAIL_INPUT, trx("Username", "input placeholder"), &l.user, free.then_some(LoginMsg::User)),
            false => field(p, EMAIL_INPUT, tr("Email"), &l.email, free.then_some(LoginMsg::Email)),
        };
        // Desktop's password field has no floating label.
        let password = l.password_login.then(|| {
            let hint = trx("Enter Password", "input placeholder");
            brand_input(p, PASSWORD_INPUT, hint, &l.password, free.then_some(LoginMsg::Password)).secure(true).into()
        });
        let next = brand_button(p, label(trx("Next", "button label"), None), Brand::Primary, self.login_act(Stop::Next), l.busy);
        let toggle = l.password_link.then(|| {
            let text = match l.password_login {
                true => trx("Use magic code", "button label"),
                false => trx("Login with password", "button label"),
            };
            let link = button(txt(text, 13.0).color(p.accent)).bare(p.accent);
            container(link.on_press_maybe(self.login_act(Stop::Toggle))).center_x(Fill).into()
        });
        // The illustration gives its room to the fields of the password login.
        let art = match l.password_link {
            true => art(1, Fill),
            false => column![Space::new().height(Fill), art(1, 314.0), Space::new().height(Fill)].into(),
        };
        let form = [Some(first), password, Some(next), toggle].into_iter().flatten().collect();
        column![Space::new().height(40), heading(tr("Enter your Email"), 36.0), Space::new().height(FLOAT_LABEL), art, actions(form, 24.0),]
            .into()
    }

    /// Desktop's `OTPInput`: six 41.3x56 slots 8 px apart, with a 2 px ring outside the focused slot, and red digits
    /// and rings on all slots after an error.
    fn view_code_slots(&self) -> El<'_> {
        let (l, p) = (&self.login, &self.pal);
        let error = l.error.is_some();
        let focused = l.stop() == Some(Stop::Field);
        let slots = row((0..6).map(|i| {
            let ink = if error { ERROR_RED } else { p.strong };
            let digit: Option<El<'_>> =
                l.code.chars().nth(i).map(|c| semi(c.to_string(), 28.0).line_height(iced::Pixels(rem(36.0))).color(ink).into());
            let at = focused && i == l.slot;
            let caret = (at && i >= l.code.len()).then(|| container(Space::new()).width(1.5).height(28).style(fill_box(ink, 0.0)).into());
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
        // Each slot of Desktop is an input; beeper-rs takes their typing and pastes in one empty input over them,
        // and its keys in `code_key`. A press focuses the first empty slot.
        let capture = text_input("", "")
            .id(CODE_INPUT)
            .on_input(|v| LoginMsg::Slot(v, false).into())
            .on_paste(|v| LoginMsg::Slot(v, true).into())
            .size(rem(28.0))
            .padding(14)
            .style(input_style(CLEAR, CLEAR, CLEAR, CLEAR, 0.0));
        let press = mouse_area(container(Space::new()).width(Fill).height(60))
            .interaction(iced::mouse::Interaction::Text)
            .on_press(LoginMsg::SlotsPressed.into());
        stack![container(slots).center_x(Fill), container(capture).height(60), press].height(60).into()
    }

    /// The code screen: Desktop's six slots of the login code over the lock, Open Email and Resend.
    fn view_login_code(&self) -> El<'_> {
        let (l, p) = (&self.login, &self.pal);
        // `otpContainer`: the lock animation, 70 px over the heading, and the 4 px under its inline box, with the
        // slots on its bottom edge.
        let otp = container(self.view_code_slots()).align_bottom(Fill);
        let lock = stack![column![art(2, 314.0), Space::new().height(4 + 2)], otp];
        let info = tr("Check %(email)s for your login code").replace("%(email)s", l.email.trim());
        let foot = rich_text![
            span(format!("{} ", tr("Didn't get a code?"))).color(p.on_secondary),
            span(trx("Resend", "button label")).color(p.accent).link(UiMessage::Login(LoginMsg::Resend))
        ]
        .on_link_click(move |m: UiMessage| if l.busy { UiMessage::Noop } else { m })
        .size(rem(11.0))
        .line_height(iced::Pixels(rem(14.0)));
        // iced finds the link of a rich text as if its line started at the left of the text, so the line takes
        // its own width and the container centers it: a wider centered text would take the clicks on
        // "Resend" somewhere left of it.
        let foot = container(foot).center_x(Fill);
        // Desktop's floating `small` error, 8 px over the form.
        let error = l.error.as_deref().map(|e| txt(e, 10.4).color(p.text_danger).center().width(Fill));
        let error = container(column![].push(error)).height(ERROR_ROOM - 8.0).align_bottom(Fill).padding([0, 18]);
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
                    container(txt(info, 13.0).line_height(iced::Pixels(rem(16.0))).color(p.on_secondary).center().width(Fill))
                        .padding(Padding::ZERO.bottom(8))
                        .into(),
                    brand_button(p, label(tr("Open Email"), None), Brand::Primary, self.login_act(Stop::OpenEmail), l.busy),
                    container(foot).padding(Padding::ZERO.top(8)).height(36).center_y(36).into(),
                ],
                24.0
            ),
        ];
        let top = column![Space::new().height(50), heading(tr("Enter Code"), 36.0)].height(Fill);
        stack![top, form].into()
    }

    /// The onboarding screen that connects chat accounts: the search, the networks and Continue.
    fn view_login_accounts(&self) -> El<'_> {
        let (l, p) = (&self.login, &self.pal);
        let st = self.store.read();
        let search = crate::networks::search_field(p, &l.query, |q| LoginMsg::Query(q).into());
        let divider = container(Space::new()).width(Fill).height(1).style(fill_box(p.chip, 0.0));
        let grid = scrollable(self.view_networks(&st, &l.query)).height(Fill);
        // Desktop's continue bar shows once a network account is connected (`hasConnectedNetwork`).
        let go_on = st.has_connected_network().then(|| {
            let go_on = brand_button(p, label(tr("Continue"), None), Brand::Primary, Some(LoginMsg::Continue.into()), false);
            column![container(Space::new()).width(Fill).height(1).style(fill_box(p.border_weak, 0.0)), container(go_on).padding(12)]
        });
        // One line of the footer takes 31 px, and a wrapped footer takes more.
        let foot = self.view_accounts_footer(&st).map(|f| container(f).padding([3, 0]));
        column![
            {
                let (headline, width) = gradient_text(tr("Connect Chat Accounts"), H3, 48.0);
                let headline = mouse_area(svg(headline).width(width).height(48));
                container(headline.on_press(LoginMsg::Headline.into())).center_x(Fill)
            },
            Space::new().height(8),
            container(column![search, divider, grid].push(go_on)).height(Fill).style(card_style(p.card)),
        ]
        .push(foot)
        .padding([0, 8])
        .into()
    }

    /// The icons of the username field: Desktop's `basketball-spinner` while the name is checked or `done` for a
    /// free name, the next suggestion, and clear.
    fn username_icons(&self) -> Vec<El<'_>> {
        let (l, p) = (&self.login, &self.pal);
        let icon = |i: usize, size: f32, color: Color, msg: Option<UiMessage>| -> El<'_> {
            let glyph = svg(ICONS[i].clone()).width(size).height(size).style(move |_, _| svg::Style { color: Some(color) });
            match msg {
                Some(m) => button(container(glyph).center(20)).bare(color).on_press(m).into(),
                None => container(glyph).center(20).into(),
            }
        };
        // `--color-solids-green` and `--color-on-surface-variant`.
        let (green, grey) = (Color::from_rgb8(25, 186, 59), p.muted);
        // Desktop's `basketball-spinner` while the name is checked, else `done` for a free name.
        let status: Option<El<'_>> = match (l.checking, l.available) {
            (true, _) => Some(container(svg(SPINNER.clone()).width(20).height(20)).center(20).into()),
            (false, Some(true)) if !l.username.is_empty() => Some(icon(2, 20.0, green, None)),
            _ => None,
        };
        let icons = [
            status,
            (!l.suggestions.is_empty()).then(|| icon(3, 18.0, grey, Some(LoginMsg::NextSuggestion.into()))),
            (!l.username.is_empty()).then(|| icon(4, 20.0, grey, Some(LoginMsg::Username(String::new()).into()))),
        ];
        icons.into_iter().flatten().collect()
    }

    /// The screen that picks a username, with its status, its suggestions and its error.
    fn view_login_username(&self) -> El<'_> {
        let (l, p) = (&self.login, &self.pal);
        // Desktop shows the same text for every name that the rules or the server refuse.
        let refused = (l.available == Some(false)).then(|| tr("Username is not available. Try something else."));
        let name = field_with_icons(
            p,
            USERNAME_INPUT,
            tr("Username"),
            &l.username,
            (!l.busy).then_some(LoginMsg::Username),
            self.username_icons(),
        );
        // Desktop's floating `small` error, 20 px over the field: a layer, so that it moves nothing.
        let error = l.error.as_deref().or(refused).map(|e| txt(e, 10.4).color(p.text_danger).center().width(Fill));
        let over_form = 20.0 + 38.0 + 8.0 + 33.0 + 52.0;
        let error = column![Space::new().height(Fill), column![].push(error).padding([0, 18]), Space::new().height(over_form)];
        let screen = column![
            Space::new().height(34),
            heading(tr("Pick a Username"), 32.0),
            // The room of the floating label, so that the gaps are Desktop's.
            Space::new().height(FLOAT_LABEL),
            Space::new().height(Fill),
            art(0, 314.0),
            Space::new().height(Fill),
            actions(
                vec![name, brand_button(p, label(trx("Next", "button label"), None), Brand::Primary, self.login_act(Stop::Next), l.busy),],
                52.0
            ),
        ];
        stack![screen, error].into()
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
            (Stop::Field | Stop::Password | Stop::Next, Screen::Email) => LoginMsg::SubmitEmail.into(),
            (Stop::Toggle, _) => LoginMsg::TogglePassword.into(),
            // Desktop disables Next until the check of the name allows it.
            (Stop::Field | Stop::Next, Screen::Username) if self.login.available == Some(true) => LoginMsg::SubmitUsername.into(),
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
        if let Some(task) = self.code_key(&key, mods) {
            return task;
        }
        let l = &mut self.login;
        let stops = l.stops();
        match key {
            Key::Named(Named::Escape) => {
                (l.menu, self.networks.menu, self.menu_bar, self.hotkeys_modal) = (false, None, None, None);
            }
            Key::Named(Named::Tab) if (mods - Modifiers::SHIFT).is_empty() => {
                return match l.focus.step(stops, mods.shift()) {
                    Some(Stop::Field) => {
                        // Chromium's Tab goes into the first slot of the code, and Shift+Tab into the last one.
                        l.slot = if mods.shift() { CODE_SLOTS - 1 } else { 0 };
                        operation::focus(l.screen.field().expect("a screen with a field"))
                    }
                    Some(Stop::Password) => operation::focus(PASSWORD_INPUT),
                    Some(_) => crate::focus::blur(),
                    None => Task::none(),
                };
            }
            // Desktop's Enter hotkey of the forms of `AwaitRecoveryCode`, `AwaitNewRecoveryCode` and
            // `ConfirmResetRecoveryCode` (`o1`), which `mousetrap` runs in the key box too, without its newline: it
            // submits the key as it is, compares the confirmed key, or resets.
            Key::Named(Named::Enter) if mods.is_empty() && matches!(l.screen, Screen::RecoveryKey | Screen::ConfirmKey | Screen::Reset) => {
                return match (l.busy, l.screen) {
                    (true, _) => Task::none(),
                    (_, Screen::RecoveryKey) => self.update_login(LoginMsg::SubmitRecoveryKey),
                    (_, Screen::ConfirmKey) => self.update_login(LoginMsg::SubmitConfirmKey),
                    _ => self.update_login(LoginMsg::ConfirmReset),
                };
            }
            Key::Named(Named::Enter) if mods.is_empty() => {
                let stop = l.stop().or(l.screen.field().map(|_| Stop::Field));
                return stop.and_then(|s| self.login_act(s)).map_or_else(Task::none, |m| self.update(m));
            }
            _ => {}
        }
        Task::none()
    }

    /// Desktop's `handleKeyDown` of `OTPInput` on the code screen: Backspace, Delete, ←, → and Tab on the focused
    /// slot, and Shift+Tab to the slot before it. Outside the slots, the window's `keydown` handler gives the same
    /// keys to the first empty slot, which takes the focus when they move it, and keeps Tab there.
    fn code_key(&mut self, key: &Key, mods: Modifiers) -> Option<Task<UiMessage>> {
        let l = &mut self.login;
        let Key::Named(k) = *key else { return None };
        let inside = l.stop() == Some(Stop::Field);
        if l.screen != Screen::Code || !(mods - Modifiers::SHIFT).is_empty() {
            return None;
        }
        if k == Named::Tab && mods.shift() {
            // Each slot is an input, so Chromium's Shift+Tab goes to the slot before.
            return (inside && l.slot > 0).then(|| {
                l.slot -= 1;
                Task::none()
            });
        }
        let at = if inside { l.slot } else { l.first_slot() };
        let (code, slot) = slot_key(&l.code, at, k)?;
        let focus = (inside || slot != at).then(|| {
            l.slot = slot;
            l.focus_on(Stop::Field);
            operation::focus(CODE_INPUT)
        });
        // Backspace and Delete change the code through `onChange`, which clears the error.
        let edit = matches!(k, Named::Backspace | Named::Delete).then(|| self.update_login(LoginMsg::Code(code)));
        Some(Task::batch(focus.into_iter().chain(edit)))
    }

    /// Text typed in the sign-in window outside a text field: Space presses the focused control, and a digit goes
    /// into the code, like Desktop's `keydown` handler of the code screen.
    pub fn login_text(&mut self, text: &str) -> Task<UiMessage> {
        let l = &mut self.login;
        if l.screen == Screen::Code && text.len() == 1 && text.chars().all(|c| c.is_ascii_digit()) {
            let at = l.first_slot();
            let (code, slot) = slot_digit(&l.code, at, text.chars().next().expect("a digit"));
            // The focus goes to the next slot, so a digit in the last slot leaves it where it is.
            let inside = at < CODE_SLOTS - 1;
            if inside {
                l.slot = slot;
                l.focus_on(Stop::Field);
            }
            let code = self.update_login(LoginMsg::Code(code));
            return Task::batch([code].into_iter().chain(inside.then(|| operation::focus(CODE_INPUT))));
        }
        let field = |s: &Stop| matches!(s, Stop::Field | Stop::Password);
        let stop = l.stop().filter(|s| !field(s) && text == " ");
        stop.and_then(|s| self.login_act(s)).map_or_else(Task::none, |m| self.update(m))
    }
}

#[cfg(test)]
mod tests {
    /// Desktop's h3 headings of the recovery key fit the 314 px frame of the 330 px sign-in window at their size,
    /// where the 28 px ones were squeezed to its edges.
    #[test]
    fn recovery_key_headings_fit_the_sign_in_window_like_desktop() {
        for title in ["Enter your Recovery Key", "Save Your Recovery Key"] {
            let width = crate::style::gradient_text(title, super::H3, 40.0).1;
            assert!(width <= 330.0 - 2.0 * 8.0, "{title}: {width}");
        }
    }

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
        assert!(!login.animated(), "the Lottie animations of the other screens redraw themselves");
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

    /// Desktop's `OTPInput` keys: Backspace empties the slot and closes the code up, then goes back a slot; Delete
    /// pulls the digits after the slot into it, and takes the last digit from an empty slot; → and Tab go on only
    /// from a slot with a digit; Tab on the last slot is Chromium's.
    #[test]
    fn code_slot_keys_like_desktop() {
        use iced::keyboard::key::Named::{ArrowLeft, ArrowRight, Backspace, Delete, Tab};
        let key = super::slot_key;
        assert_eq!(key("123", 3, Backspace), Some(("123".into(), 2)), "an empty slot goes back without a digit");
        assert_eq!(key("123", 2, Backspace), Some(("12".into(), 1)));
        assert_eq!(key("123456", 1, Backspace), Some(("13456".into(), 0)), "the code closes up");
        assert_eq!(key("", 0, Backspace), Some(("".into(), 0)));
        assert_eq!(key("123456", 1, Delete), Some(("13456".into(), 1)), "Delete keeps the slot");
        assert_eq!(key("123", 3, Delete), Some(("12".into(), 3)), "an empty slot takes the last digit");
        assert_eq!(key("", 0, Delete), Some(("".into(), 0)));
        assert_eq!(key("12", 0, ArrowLeft), Some(("12".into(), 0)));
        assert_eq!(key("12", 1, ArrowRight), Some(("12".into(), 2)));
        assert_eq!(key("12", 2, ArrowRight), Some(("12".into(), 2)), "→ stops at the first empty slot");
        assert_eq!(key("12", 1, Tab), Some(("12".into(), 2)));
        assert_eq!(key("12", 2, Tab), Some(("12".into(), 2)), "Tab stays on an empty slot");
        assert_eq!(key("123456", 5, Tab), None, "Tab leaves the last slot");
        assert_eq!(key("123456", 5, ArrowRight), Some(("123456".into(), 5)));
        assert_eq!(super::slot_digit("123456", 2, '9'), ("129456".into(), 3), "a digit takes the slot's place");
        assert_eq!(super::slot_digit("12345", 5, '6'), ("123456".into(), 5), "the last slot keeps the focus");
        assert_eq!(super::slot_paste("1", 1, "9 8x"), "198", "a paste fills the slots from the focused one");
        assert_eq!(super::slot_paste("12345", 4, "987"), "123498", "up to the last slot");
    }

    use super::{LoginMsg, Outcome, Screen, Stop};
    use crate::fuzz::{signed_out_app, task_actions, task_messages};
    use crate::{BeeperGuiApp, UiMessage};
    use iced_test::simulator::simulator;
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    /// The sign-in window of a new install, whose Beeper API and homeserver are the stub at `url`.
    fn stub_sign_in(name: &str, url: &str) -> BeeperGuiApp {
        let mut app = signed_out_app(name);
        app.cloud = Arc::new(beeper_core::cloud::Cloud::new(url, url));
        app
    }

    /// Sends each message of a task until no task outputs more, like the iced runtime.
    async fn settle(app: &mut BeeperGuiApp, msg: UiMessage) {
        let mut queue = vec![msg];
        while let Some(m) = queue.pop() {
            queue.extend(task_messages(app.update(m)).await);
        }
    }

    /// "Copy from Beeper Desktop" copies only into the Desktop account, also from a Desktop without keys, whose
    /// account is in `beeperState`. A refused sign-in and a failed copy of the keys keep the import, so the next
    /// sign-in copies the data instead of starting fresh.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_import_waits_for_a_sign_in_of_the_desktop_account() {
        use beeper_core::models::{MatrixSession, Profile};
        use beeper_core::store::Setup;
        let mut app = signed_out_app("import-account");
        let chat = beeper_core::models::Chat { id: "!desk:x".into(), title: "From Desktop".into(), ..Default::default() };
        let data = beeper_core::import::BeeperData { user_id: Some("@ann:x".into()), chats: vec![chat], ..Default::default() };
        let _ = app.update(LoginMsg::Done(Ok(Outcome::Read(Arc::new(data)))).into());
        let session = |user: &str| {
            let s = MatrixSession { user_id: user.into(), access_token: "token".into(), device_id: "NEW".into(), jwt: None };
            LoginMsg::Done(Ok(Outcome::Session(s, Profile::default(), Vec::new()))).into()
        };
        let _ = app.update(session("@bob:x"));
        assert_eq!(app.login.toast.as_deref(), Some("Sign in as @ann:x to finish the import."));
        assert!(!app.store.read().signed_in() && app.store.read().setup.is_none(), "nothing is copied for bob");

        let copy = app.update(session("@ann:x"));
        let failed = beeper_core::cloud::CloudError { status: 0, message: "bad key".into(), errcode: None, body: None };
        let _ = app.update(LoginMsg::Done(Err(failed)).into());
        assert!(app.login.import.is_some() && app.store.read().setup.is_none(), "a failed copy keeps the import");

        for m in task_messages(copy).await {
            let _ = app.update(m);
        }
        let st = app.store.read();
        assert_eq!((st.setup, st.beeper_state.user_id.as_str()), (Some(Setup::Imported), "@ann:x"));
        assert!(st.find_chat("!desk:x").is_some(), "the Desktop chat came over");
        assert!(app.login.import.is_none());
    }

    /// A click on "Resend", in the middle of the line under the code, asks for a new code, and a click on "Didn't
    /// get a code?" does not.
    #[test]
    fn resend_takes_the_clicks_on_its_word() {
        let mut app = signed_out_app("resend-click");
        let _ = app.update(LoginMsg::Email("alex@example.com".into()).into());
        let _ = app.update(LoginMsg::Done(Ok(Outcome::Requested("req_1".into()))).into());
        let size = iced::Size::new(330.0, 620.0);
        // The x of the middle of "Resend" and of "Didn't", on the line centered in the window.
        let width = |s: &str| crate::style::text_width(s, 11.0, iced::font::Weight::Normal);
        let line = width("Didn't get a code? Resend");
        let (resend, didnt) = ((size.width + line) / 2.0 - width("Resend") / 2.0, (size.width - line) / 2.0 + 10.0);
        let open_email = {
            let mut sim = iced_test::Simulator::with_size(Default::default(), size, app.view_login());
            sim.find("Open Email").expect("the Open Email button").bounds()
        };
        // Whether a click at `x` on some height under Open Email sends Resend.
        let resends = |x: f32| {
            (0..40).step_by(2).any(|dy| {
                let at = iced::Point::new(x, open_email.y + open_email.height + dy as f32);
                let mut sim = iced_test::Simulator::with_size(Default::default(), size, app.view_login());
                sim.point_at(at);
                let _ = sim.simulate([iced::Event::Mouse(iced::mouse::Event::CursorMoved { position: at })]);
                let _ = sim.simulate(iced_test::simulator::click());
                sim.into_messages().any(|m| matches!(m, UiMessage::Login(LoginMsg::Resend)))
            })
        };
        assert!(resends(resend), "a click on Resend");
        assert!(!resends(didnt), "a click on the words before it");
    }

    /// Desktop's six slot inputs of the code: the focus starts on the first empty slot, a digit takes the place of
    /// the digit of the slot, ←, Backspace and Shift+Tab go back a slot, and Delete keeps it. Outside the slots, the
    /// window gives the keys to the first empty slot and keeps Tab there. A press on refused slots clears them.
    #[test]
    fn the_code_slots_take_the_keys_like_desktop() {
        use iced::keyboard::{key::Named, Key, Modifiers};
        let mut app = signed_out_app("code-slots");
        let id = app.login_window.expect("the sign-in window");
        let _ = app.update(LoginMsg::Email("alex@example.com".into()).into());
        let _ = app.update(LoginMsg::Done(Ok(Outcome::Requested("req_1".into()))).into());
        let key = |app: &mut BeeperGuiApp, k: Named, mods: Modifiers| {
            let _ = app.update(UiMessage::WindowKey(id, Key::Named(k), mods));
            (app.login.code.clone(), app.login.slot)
        };
        let typed = |app: &mut BeeperGuiApp, d: &str| {
            let _ = app.update(LoginMsg::Slot(d.into(), false).into());
            (app.login.code.clone(), app.login.slot)
        };
        let none = Modifiers::empty();
        assert_eq!((app.login.focus.get(), app.login.slot), (Some(Stop::Field), 0), "the first slot has the focus");
        for d in ["1", "2"] {
            typed(&mut app, d);
        }
        assert_eq!(typed(&mut app, "3"), ("123".into(), 3), "each digit moves the focus on");
        assert_eq!(typed(&mut app, "x"), ("123".into(), 3), "other text goes nowhere");
        assert_eq!(key(&mut app, Named::ArrowLeft, none), ("123".into(), 2));
        assert_eq!(typed(&mut app, "9"), ("129".into(), 3), "a digit takes the place of the 3");
        assert_eq!(key(&mut app, Named::Backspace, none), ("129".into(), 2), "an empty slot goes back");
        assert_eq!(key(&mut app, Named::Backspace, none), ("12".into(), 1), "and the next Backspace takes the 9");
        assert_eq!(key(&mut app, Named::Tab, Modifiers::SHIFT), ("12".into(), 0), "Shift+Tab goes to the slot before");
        assert_eq!(app.login.focus.get(), Some(Stop::Field), "inside the code");
        assert_eq!(key(&mut app, Named::Delete, none), ("2".into(), 0), "Delete pulls the 2 into the slot");

        app.login.focus.set(Some(Stop::Resend));
        assert_eq!(key(&mut app, Named::Tab, none), ("2".into(), 0));
        assert_eq!(app.login.focus.get(), Some(Stop::Resend), "the window keeps Tab before the last slot");
        assert_eq!(key(&mut app, Named::Backspace, none), ("2".into(), 0), "Backspace goes back from the first empty slot");
        assert_eq!(app.login.focus.get(), Some(Stop::Field), "and takes the focus into the code");
        app.login.focus.set(Some(Stop::Resend));
        let _ = app.update(UiMessage::WindowText(id, "7".into()));
        assert_eq!((app.login.code.as_str(), app.login.slot), ("27", 2), "a digit outside fills the first empty slot");
        assert_eq!(app.login.focus.get(), Some(Stop::Field));

        app.login.error = Some("Invalid code. Please try again.".into());
        let _ = app.update(LoginMsg::SlotsPressed.into());
        assert_eq!((app.login.code.as_str(), app.login.slot, app.login.error.is_none()), ("", 0, true));
    }

    fn shows(app: &BeeperGuiApp, text: &str) -> bool {
        simulator(app.view_login()).find(text).is_ok()
    }

    /// Desktop's texts of a refused code: the server's `errorMessage` under the heading, "Invalid code. Please try
    /// again." once the server counts two retries, and the message of a 5xx in the toast.
    #[tokio::test(flavor = "multi_thread")]
    async fn refused_codes_show_desktops_text() {
        let url = beeper_core::bridge_stub::serve(|_, path, body| match (path, body["response"].as_str()) {
            ("/user/login/response", Some("111111")) => (401, json!({ "errorMessage": "Wrong code", "retries": 1 })),
            ("/user/login/response", Some("222222")) => (401, json!({ "errorMessage": "Wrong code", "retries": 2 })),
            ("/user/login/response", _) => (503, json!({ "error": "Service unavailable" })),
            _ => (404, json!({ "errcode": "M_UNRECOGNIZED" })),
        })
        .await;
        let mut app = stub_sign_in("code-errors", &url);
        let _ = app.update(LoginMsg::Done(Ok(Outcome::Requested("req".into()))).into());
        let texts = |app: &BeeperGuiApp| (app.login.error.clone(), app.login.toast.clone());
        settle(&mut app, LoginMsg::Code("111111".into()).into()).await;
        assert_eq!(texts(&app), (Some("Wrong code".into()), None), "the server's message");
        assert!(shows(&app, "Wrong code"), "under the heading");
        settle(&mut app, LoginMsg::Code("222222".into()).into()).await;
        assert_eq!(texts(&app), (Some("Invalid code. Please try again.".into()), None), "after two retries");
        settle(&mut app, LoginMsg::Code("333333".into()).into()).await;
        assert_eq!(texts(&app), (None, Some("Service unavailable".into())), "a 5xx in the toast");
    }

    /// Desktop's `UsernameInput`: the spinner while a name waits for its debounced check, one server check for the
    /// last of quick changes, the local rules before the server, and the error of a failed check in the toast.
    #[tokio::test(flavor = "multi_thread")]
    async fn usernames_are_checked_on_the_server_like_desktop() {
        let asked = Arc::new(Mutex::new(Vec::new()));
        let log = asked.clone();
        let url = beeper_core::bridge_stub::serve(move |_, path, _| {
            let name = path.strip_prefix("/_matrix/client/v3/register/available?username=").expect("a name check");
            log.lock().unwrap().push(name.to_string());
            match name {
                "alex-rs" => (200, json!({ "available": true })),
                "taken" => (400, json!({ "errcode": "M_USER_IN_USE", "error": "User ID already taken." })),
                _ => (500, json!({ "errcode": "M_UNKNOWN", "error": "Internal server error" })),
            }
        })
        .await;
        let mut app = stub_sign_in("username-check", &url);
        let task = app.update(LoginMsg::Done(Ok(Outcome::Lead("lead".into(), vec!["alex-rs".into()]))).into());
        assert!(app.login.checking, "the suggestion is checked");
        assert!(app.login_act(Stop::Next).is_none(), "Next waits for the check");
        for m in task_messages(task).await {
            settle(&mut app, m).await;
        }
        assert_eq!((app.login.checking, app.login.available), (false, Some(true)));
        assert!(app.login_act(Stop::Next).is_some(), "a free name enables Next");

        let first = app.update(LoginMsg::Username("take".into()).into());
        let last = app.update(LoginMsg::Username("taken".into()).into());
        for m in [task_messages(first).await, task_messages(last).await].concat() {
            settle(&mut app, m).await;
        }
        assert_eq!(*asked.lock().unwrap(), ["alex-rs", "taken"], "only the last of quick changes is checked");
        assert!(shows(&app, "Username is not available. Try something else."));
        assert!(app.login_act(Stop::Next).is_none());

        settle(&mut app, LoginMsg::Username("ab".into()).into()).await;
        assert_eq!((app.login.checking, app.login.available), (false, None), "a short name is not checked");
        settle(&mut app, LoginMsg::Username("my-bot".into()).into()).await;
        assert_eq!(app.login.available, Some(false), "the rules refuse a name that ends with bot");
        assert_eq!(asked.lock().unwrap().len(), 2, "names that the rules refuse do not reach the server");

        settle(&mut app, LoginMsg::Username("boom".into()).into()).await;
        assert_eq!((app.login.toast.as_deref(), app.login.available), (Some("Internal server error"), None));
    }

    /// Desktop's hidden password login: the tenth Next with no email switches to a username and a password, Tab
    /// reaches the new controls, a refusal shows Desktop's text for its error code, and "Use magic code" goes back.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_tenth_empty_next_offers_the_password_login_like_desktop() {
        use iced::keyboard::{key::Named, Key, Modifiers};
        let url = beeper_core::bridge_stub::serve(|method, path, body| match (method, path) {
            ("POST", "/_matrix/client/v3/login") if body["password"] == "right" => {
                (200, json!({ "user_id": "@alex:beeper.com", "access_token": "token", "device_id": "DEV" }))
            }
            ("POST", "/_matrix/client/v3/login") => (403, json!({ "errcode": "M_FORBIDDEN", "error": "Invalid username or password" })),
            ("GET", "/whoami") => (200, json!({ "userInfo": { "username": "alex" }, "user": { "bridges": {} } })),
            _ => (404, json!({ "errcode": "M_UNRECOGNIZED" })),
        })
        .await;
        let mut app = stub_sign_in("password-login", &url);
        let _ = app.update(LoginMsg::Show(Screen::Email).into());
        for _ in 0..9 {
            let _ = app.update(LoginMsg::SubmitEmail.into());
        }
        assert_eq!(app.login.toast.as_deref(), Some("Please enter your email."));
        assert!(!shows(&app, "Login with password"), "nine Nexts keep the email login");
        let _ = app.update(LoginMsg::SubmitEmail.into());
        assert!(app.login.password_login && app.login.toast.is_none(), "the tenth switches to the password login");
        assert!(shows(&app, "Use magic code"));

        let id = app.login_window.expect("the sign-in window");
        let tabs: Vec<_> = (0..4)
            .map(|_| {
                let _ = app.update(UiMessage::WindowKey(id, Key::Named(Named::Tab), Modifiers::empty()));
                app.login.focus.get().expect("a focused control")
            })
            .collect();
        assert_eq!(tabs, [Stop::Password, Stop::Next, Stop::Toggle, Stop::Menu]);

        let _ = app.update(LoginMsg::User("alex".into()).into());
        let _ = app.update(LoginMsg::Password("wrong".into()).into());
        settle(&mut app, LoginMsg::SubmitEmail.into()).await;
        assert_eq!(app.login.toast.as_deref(), Some("Incorrect username or password. Please try again."));

        let _ = app.update(LoginMsg::TogglePassword.into());
        assert!(!app.login.password_login && app.login.user.is_empty() && app.login.password.is_empty());
        assert!(shows(&app, "Login with password"), "the link stays to switch again");
        let _ = app.update(LoginMsg::TogglePassword.into());
        let _ = app.update(LoginMsg::User("alex".into()).into());
        let _ = app.update(LoginMsg::Password("right".into()).into());
        let done = task_messages(app.update(LoginMsg::SubmitEmail.into())).await;
        assert!(matches!(done.as_slice(), [UiMessage::Login(LoginMsg::Done(Ok(Outcome::Session(..))))]));
        let _ = app.update(done.into_iter().next().expect("the session"));
        let st = app.store.read();
        assert!(st.signed_in() && st.beeper_state.user_id == "@alex:beeper.com", "the password signs in");
    }

    /// Desktop's Continue of Connect Chat Accounts selects the first read chat and shows the mobile apps toast a
    /// second later, whose links and close work. The tenth click on the heading does neither.
    #[tokio::test(flavor = "multi_thread")]
    async fn continue_selects_a_read_chat_and_shows_the_mobile_apps_toast_like_desktop() {
        let onboarding = |name: &str| {
            let mut app = crate::fuzz::demo_app(name);
            (app.login_window, app.main_window, app.login.screen) = (app.main_window, None, Screen::Accounts);
            app
        };
        let mut app = onboarding("onboarding-continue");
        let first_read = app.listed_chats(&app.store.read()).iter().find(|c| !c.is_unread()).map(|c| c.id.clone());
        assert!(first_read.is_some(), "a read demo chat");
        let task = app.update(LoginMsg::Continue.into());
        assert_eq!(app.pane.selected_chat_id, first_read, "Desktop's selectFirstReadThread");
        assert!(!app.mobile_apps_toast, "the toast comes a second later");
        let start = std::time::Instant::now();
        for m in task_messages(task).await {
            let _ = app.update(m);
        }
        assert!(app.mobile_apps_toast && start.elapsed() >= std::time::Duration::from_secs(1));
        // Clicks all over the toasts: the links open their app store, the rest of the toast the download page, and
        // the close button hides it.
        let size = iced::Size::new(600.0, 200.0);
        let mut sim = iced_test::Simulator::with_size(Default::default(), size, app.view_toast(true).expect("toasts"));
        for y in (0..200).step_by(4) {
            for x in (76..600).step_by(4) {
                sim.point_at(iced::Point::new(x as f32, y as f32));
                let _ = sim.simulate(iced_test::simulator::click());
            }
        }
        let msgs: Vec<UiMessage> = sim.into_messages().collect();
        let opens = |url: &str| msgs.iter().any(|m| matches!(m, UiMessage::OpenUrl(u) if u == url));
        let urls = ["https://www.beeper.com/download/ios", "https://www.beeper.com/download/android"];
        assert!(urls.into_iter().chain(["https://www.beeper.com/download"]).all(opens));
        let close = UiMessage::Toast(crate::toasts::Msg::MobileApps(false));
        assert!(msgs.iter().any(|m| matches!(m, UiMessage::Toast(crate::toasts::Msg::MobileApps(false)))));
        let _ = app.update(close);
        assert!(!app.mobile_apps_toast, "the close hides the toast");

        let mut app = onboarding("onboarding-headline");
        let mut task = iced::Task::none();
        for _ in 0..10 {
            task = app.update(LoginMsg::Headline.into());
        }
        assert!(app.main_window.is_some());
        assert_eq!(app.pane.selected_chat_id.as_deref(), Some(beeper_core::store::NOTE_CHAT_ID));
        assert!(task_messages(task).await.is_empty(), "no mobile apps toast");
    }

    /// Desktop's sign-in window becomes resizable on Connect Chat Accounts.
    #[tokio::test(flavor = "multi_thread")]
    async fn connect_chat_accounts_is_resizable_like_desktop() {
        let mut app = crate::fuzz::demo_app("onboarding-resizable");
        let id = app.main_window.take().expect("the main window");
        (app.login_window, app.login.screen) = (Some(id), Screen::Syncing);
        app.store.write().bridge_accounts.clear();
        let actions = task_actions(app.advance_onboarding()).await;
        assert_eq!(app.login.screen, Screen::Accounts);
        let resizable = |a: &iced_runtime::Action<UiMessage>| matches!(a, iced_runtime::Action::Window(iced_runtime::window::Action::SetResizable(w, true)) if *w == id);
        assert!(actions.iter().any(resizable), "the window becomes resizable");
    }
}
