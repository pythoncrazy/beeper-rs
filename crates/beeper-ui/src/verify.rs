//! Desktop's device verification: the toast of a request from another device, the `VerificationModal` window, and the
//! onboarding screens of encrypted messaging after the sign-in.
use crate::i18n::{tr, trn_named, trx};
use crate::login::{art, brand_gradient, Screen, H3};
use crate::prelude::*;
use iced::widget::column;
use iced::{border, window};

/// The size of Desktop's `VERIFICATION` modal window.
const SIZE: Size = Size::new(330.0, 480.0);

#[derive(Default)]
pub struct Verify {
    /// Desktop's `VERIFICATION` modal window.
    pub window: Option<window::Id>,
    /// The ID and the state of the verification that the UI saw last.
    seen: Option<(String, &'static str)>,
    /// Desktop's OS notification of an incoming request: the notification ID and the verification ID.
    pub note: Option<(u32, String)>,
}

#[derive(Debug, Clone)]
pub enum VerifyMsg {
    /// The Accept and Decline buttons of the request toast.
    Accept(String),
    Decline(String),
    /// "They match" and "They don't match".
    Match(bool),
    /// "Confirm using emojis instead" of the QR code.
    Emojis,
    /// A click outside the modal, which cancels the verification.
    Outside,
    /// The OS notification of the incoming request with the ID.
    Noted(String, Result<u32, String>),
    /// The end of a Matrix call, with the text of the toast that tells its error.
    Done(&'static str, Result<(), String>),
}

impl From<VerifyMsg> for UiMessage {
    fn from(m: VerifyMsg) -> Self {
        UiMessage::Verify(m)
    }
}

/// Closes the OS notification `id`.
fn close_note(id: u32) -> Task<UiMessage> {
    Task::future(crate::notify::close(vec![id])).discard()
}

/// The text of Desktop's toast of a request from the device `name`.
fn request_text(name: &str) -> String {
    tr("Verification request from %(deviceName)s").replace("%(deviceName)s", name)
}

/// The name of the other device, like Desktop's `getDeviceWithRetry`: the display name, else the device ID.
fn device_name(v: &beeper_core::models::AppVerification) -> String {
    let device = v.other_device.as_ref();
    match device.and_then(|d| d.name.clone().or_else(|| Some(d.id.clone()))) {
        Some(name) => name,
        None => trx("Unknown device", "device fallback label").into(),
    }
}

/// A Matrix call of the verification. A failure shows the toast `what`, or the error when `what` is empty.
fn call(what: &'static str, f: impl std::future::Future<Output = Result<(), String>> + Send + 'static) -> Task<UiMessage> {
    Task::perform(f, move |r| VerifyMsg::Done(what, r).into())
}

fn cancel(store: SharedStore, id: String) -> Task<UiMessage> {
    call("", async move { beeper_core::matrix::cancel_verification(&store, &id).await })
}

impl BeeperGuiApp {
    pub fn update_verify(&mut self, msg: VerifyMsg) -> Task<UiMessage> {
        let s = self.store.clone();
        match msg {
            VerifyMsg::Accept(id) => {
                let accept = call("Failed to start verification", async move { beeper_core::matrix::accept_verification(&s, &id).await });
                return Task::batch([accept, self.open_verify()]);
            }
            VerifyMsg::Decline(id) => return cancel(s, id),
            VerifyMsg::Emojis => {
                let id = s.read().app_verification().map(|v| v.id.clone()).expect("the QR code is of a verification");
                return call("", async move { beeper_core::matrix::start_sas(&s, &id).await });
            }
            VerifyMsg::Match(same) if self.login_window.is_some() && self.login.screen == Screen::Verify => {
                let id = s.read().app_verification().map(|v| v.id.clone()).expect("the emojis are of a verification");
                if !same {
                    return cancel(s, id);
                }
                if let Some(tx) = s.write().sas_confirm.take() {
                    tx.send(true).ok();
                }
                self.login.waiting = true;
            }
            VerifyMsg::Match(true) => {
                // An API client can answer the same emojis first.
                if let Some(tx) = s.write().sas_confirm.take() {
                    tx.send(true).ok();
                }
                let text = tr("Waiting for you to verify on your other device...");
                self.show(Toast::new(text.into()));
                self.sticky_banner = Some(text.into());
                return self.close_verify();
            }
            VerifyMsg::Match(false) | VerifyMsg::Outside => {
                let Some(id) = s.read().app_verification().map(|v| v.id.clone()) else { return self.close_verify() };
                self.show(Toast::new(tr("Verification cancelled").into()));
                return Task::batch([cancel(s, id), self.close_verify()]);
            }
            VerifyMsg::Noted(id, Ok(note)) => {
                let waits = s.read().app_verification().is_some_and(|v| v.id == id && v.state == "requested");
                match waits {
                    true => self.verify.note = Some((note, id)),
                    false => return close_note(note),
                }
            }
            VerifyMsg::Noted(_, Err(e)) => eprintln!("notification: {e}"),
            VerifyMsg::Done(_, Ok(())) => {}
            VerifyMsg::Done(what, Err(e)) => self.fail(Toast::new(if what.is_empty() { e } else { tr(what).into() })),
        }
        Task::none()
    }

    fn open_verify(&mut self) -> Task<UiMessage> {
        if self.verify.window.is_some() || self.main_window.is_none() {
            return Task::none();
        }
        let (id, open) = self.open_child(SIZE, false);
        self.verify.window = Some(id);
        open.chain(window::set_level(id, window::Level::AlwaysOnTop))
    }

    fn close_verify(&mut self) -> Task<UiMessage> {
        self.verify.window.take().map_or_else(Task::none, window::close)
    }

    /// Desktop's `sendVerificationNotification`: the OS notification of the request with Accept and Decline.
    fn notify_request(&self, id: String, text: String) -> Task<UiMessage> {
        let silent = self.store.read().settings.pref("messageSound") == 1;
        let buttons = vec![
            ("verificationAccept".into(), trx("Accept", "button label").to_string()),
            ("verificationDecline".into(), trx("Decline", "button label").to_string()),
        ];
        let toast = crate::notify::Toast { title: text, silent, buttons, ..Default::default() };
        Task::perform(crate::notify::show(toast), move |r| VerifyMsg::Noted(id, r).into())
    }

    /// Desktop's actions of the notification of a request: Accept and Decline answer it, and a click shows the app.
    pub(crate) fn verification_note_event(&mut self, action: Option<&str>) -> Task<UiMessage> {
        let (note, id) = self.verify.note.take().expect("the event is of the request notification");
        let answer = match action {
            Some("verificationAccept") => VerifyMsg::Accept(id.clone()),
            Some("verificationDecline") => VerifyMsg::Decline(id.clone()),
            Some(_) => return self.set_hidden(false),
            None => return Task::none(),
        };
        // Desktop's `acceptVerification` and `cancelVerification` remove the toast and the notification.
        let name = self.store.read().app_verification().filter(|v| v.id == id).map(device_name);
        if let Some(name) = name {
            self.drop_toast(&request_text(&name));
        }
        Task::batch([close_note(note), self.update_verify(answer)])
    }

    /// Removes the toast with `text`, like Desktop's `removeToast`.
    fn drop_toast(&mut self, text: &str) {
        if self.status_banner == text {
            self.status_banner.clear();
            self.sticky_banner = None;
        }
        self.older_toasts.retain(|t| t.0.text != text);
    }

    /// Desktop's handlers of `VERIFICATION_REQUESTED`, `QR_CODE_SCANNED`, `VERIFICATION_DONE` and
    /// `VERIFICATION_CANCELLED`, for a change
    /// of the tracked verification.
    pub fn verification_changed(&mut self) -> Task<UiMessage> {
        // Desktop's `clearVerificationNotification` once the request is answered or cancelled.
        let st = self.store.read();
        let waits = |id: &String| st.app_verification().is_some_and(|v| v.id == *id && v.state == "requested");
        let answered = self.verify.note.take_if(|n| !waits(&n.1));
        drop(st);
        let next = if self.login_window.is_some() { self.advance_onboarding() } else { Task::none() };
        Task::batch(answered.map(|n| close_note(n.0)).into_iter().chain([next, self.verification_toasts()]))
    }

    fn verification_toasts(&mut self) -> Task<UiMessage> {
        let st = self.store.read();
        let now = st.app_verification().map(|v| (v.id.clone(), v.state, v.direction, device_name(v)));
        drop(st);
        let seen = self.verify.seen.take();
        self.verify.seen = now.as_ref().map(|v| (v.0.clone(), v.1));
        let Some((id, state, direction, name)) = now.filter(|v| seen.as_ref() != Some(&(v.0.clone(), v.1))) else {
            return Task::none();
        };
        let waiting = tr("Waiting for you to verify on your other device...");
        match state {
            "requested" if direction == "incoming" => {
                let text = request_text(&name);
                let note = self.notify_request(id.clone(), text.clone());
                let buttons = vec![
                    (trx("Accept", "button label"), VerifyMsg::Accept(id.clone()).into()),
                    (trx("Decline", "button label"), VerifyMsg::Decline(id).into()),
                ];
                self.show(Toast { buttons, ..Toast::new(text.clone()) });
                self.sticky_banner = Some(text);
                return note;
            }
            // Desktop confirms the scan of its QR code by the other device at once.
            "qr_scanned" => {
                let s = self.store.clone();
                let confirm = call("", async move { beeper_core::matrix::confirm_qr(&s, &id).await });
                self.show(Toast::new(tr("Authentication confirmed, restoring the cryptographic state").into()));
                return Task::batch([confirm, self.close_verify()]);
            }
            "done" => {
                self.drop_toast(&request_text(&name));
                self.drop_toast(waiting);
                self.show(Toast::new(tr("Verification successful").into()));
                return self.close_verify();
            }
            "cancelled" => {
                self.login.waiting = false;
                self.drop_toast(&request_text(&name));
                self.drop_toast(waiting);
                return self.close_verify();
            }
            _ => {}
        }
        Task::none()
    }

    /// Desktop's `VerificationModal`: "Loading...", then the QR code for the other device to scan, or the emojis to
    /// compare with it.
    pub fn view_verify(&self) -> El<'_> {
        let p = &self.pal;
        let st = self.store.read();
        let v = st.app_verification();
        let emojis = v.and_then(|v| v.sas.as_ref().filter(|_| v.state == "sas_ready" && v.methods.contains(&"sas")));
        let qr = v.and_then(|v| v.qr.as_ref().filter(|_| emojis.is_none() && v.state == "ready"));
        let title = match (emojis, qr) {
            (Some(_), _) => tr("Emoji Verification"),
            (None, Some(_)) => tr("QR Verification"),
            (None, None) => trx("Loading...", "verification modal title"),
        };
        // `.VerificationModal-module__title` has a -0.75 px letter spacing.
        let title = t(title, 28.0, spaced(Weight::ExtraBold, -0.75 / rem(28.0))).line_height(lh(38.0)).color(p.strong).center();
        let code = qr.map(|qr| -> El<'_> {
            let emojis = v.is_some_and(|v| v.methods.contains(&"sas")).then(|| {
                let label = trx("Confirm using emojis instead", "button label");
                brand_btn(p, label, Brand::Secondary, VerifyMsg::Emojis.into())
            });
            let hint = txt(tr("Scan the QR code with your device to verify your session."), 13.0).color(p.strong);
            let content = column![hint.center().width(Fill)].push(emojis).spacing(16).align_x(Center);
            let code = qr_code(&qr.data);
            column![container(code).center_x(Fill).padding([24, 0]), content].into()
        });
        let body: Option<El<'_>> = emojis.map(|sas| {
            let name = device_name(v.expect("emojis come from a verification"));
            let glyphs = sas.emojis.iter().map(|e| {
                let glyph = crate::style::fixed(e.emoji.clone(), 28.0, iced::font::Weight::Normal).line_height(1.0).font(EMOJI_FONT);
                let tip = container(txt(e.description.clone(), 12.0).color(p.strong)).padding([4, 8]).style(fill_box(p.elevated, 6.0));
                tooltip(glyph, tip, tooltip::Position::Bottom).into()
            });
            let glyphs = container(row(glyphs).spacing(4)).center_x(Fill).padding([70, 0]);
            let (before, after) = tr("Confirm the emojis above match the ones on %(deviceName)s")
                .split_once("%(deviceName)s")
                .expect("the hint names the device");
            let hint = rich_text![span::<(), _>(before), span(name).font(fw(Weight::Semibold)), span(after)]
                .size(rem(13.0))
                .color(p.muted)
                .center()
                .width(Fill);
            let buttons = column![
                brand_btn(p, trx("They match", "button label"), Brand::Primary, VerifyMsg::Match(true).into()),
                brand_btn(p, trx("They don't match", "button label"), Brand::Muted, VerifyMsg::Match(false).into()),
            ]
            .spacing(8);
            let small = txt(tr("Proceeding will give the other device full access to your Beeper account"), 11.0)
                .color(p.muted)
                .center()
                .width(Fill);
            column![glyphs, column![hint, buttons, small].spacing(16).align_x(Center)].into()
        });
        drop(st);
        let card = column![title].push(code).push(body).align_x(Center).width(Fill);
        let card = container(card).width(Fill).height(Fill).padding(Padding::new(24.0).top(36.0)).style(fill_box(p.primary_container, 8.0));
        let card = mouse_area(card).on_press(UiMessage::Noop);
        // `VerificationModal` is `--left-pane-bg`.
        let frame = crate::ui_theme::window_bg(container(card).padding(8), p.backdrop, p.window);
        mouse_area(frame).on_press(VerifyMsg::Outside.into()).into()
    }
}

/// The variants of Desktop's `BrandButton` in `size: "small"`.
#[derive(Clone, Copy)]
pub(crate) enum Brand {
    Primary,
    Secondary,
    Muted,
    Danger,
}

/// Desktop's `BrandButton` with `size: "small"`: a full-width pill.
pub(crate) fn brand_btn<'a>(p: &Palette, label: &'a str, variant: Brand, msg: UiMessage) -> El<'a> {
    brand_btn_maybe(p, label, variant, Some(msg), false)
}

/// `brand_btn`, which is disabled without a message, and shows Desktop's dots while `loading`.
pub(crate) fn brand_btn_maybe<'a>(p: &Palette, label: &'a str, variant: Brand, msg: Option<UiMessage>, loading: bool) -> El<'a> {
    let label = t(label, 13.0, Weight::Medium).line_height(lh(14.95));
    crate::login::brand_button(p, label.into(), variant, msg, loading)
}

/// Desktop's `SpinnerIcon`, which turns once in 1.2 s.
pub(crate) fn spinner<'a>(size: f32, color: Color) -> El<'a> {
    let turn = crate::login::EPOCH.elapsed().as_secs_f32() / 1.2 * std::f32::consts::TAU;
    svg(SPINNER.clone())
        .width(size)
        .height(size)
        .rotation(iced::Rotation::Solid(iced::Radians(turn)))
        .style(move |_, _| svg::Style { color: Some(color) })
        .into()
}

static SPINNER: LazyLock<svg::Handle> = LazyLock::new(|| {
    svg::Handle::from_memory(br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="#000" d="M2.6290048,13.5685338 C2.70070755,13.9998397 3.10847664,14.2913557 3.53978255,14.2196529 C3.97108846,14.1479502 4.26260443,13.7401811 4.19090168,13.3088752 C4.11950255,12.8793956 4.08333333,12.4421541 4.08333333,12 C4.08333333,7.62774573 7.62774573,4.08333333 12,4.08333333 C16.3722543,4.08333333 19.9166667,7.62774573 19.9166667,12 C19.9166667,16.3722543 16.3722543,19.9166667 12,19.9166667 C11.5627746,19.9166667 11.2083333,20.2711079 11.2083333,20.7083333 C11.2083333,21.1455588 11.5627746,21.5 12,21.5 C17.2467051,21.5 21.5,17.2467051 21.5,12 C21.5,6.75329488 17.2467051,2.5 12,2.5 C6.75329488,2.5 2.5,6.75329488 2.5,12 C2.5,12.5292358 2.54335318,13.0533227 2.6290048,13.5685338 Z"/></svg>"##.as_slice())
});

/// Desktop's `ProgressCircle` of 250 px with a 10 px ring: `current` of `max` in the brand color from the top.
fn progress_circle<'a>(p: &Palette, current: usize, max: usize) -> El<'a> {
    let share = if max == 0 { 0.0 } else { (current as f32 / max as f32).clamp(0.0, 1.0) };
    let hex = |c: Color| {
        let [r, g, b, _] = c.into_rgba8();
        format!("#{r:02x}{g:02x}{b:02x}")
    };
    let len = std::f32::consts::TAU * 115.0;
    let ring = |color: Color, dash: String| {
        format!(
            r#"<circle cx="125" cy="125" r="115" fill="none" stroke="{}" stroke-width="10" stroke-linecap="round" {dash} transform="rotate(-90 125 125)"/>"#,
            hex(color)
        )
    };
    let progress = (share > 0.0).then(|| ring(p.accent, format!(r#"stroke-dasharray="{} {len}""#, share * len)));
    let svg_text = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 250 250">{}{}</svg>"#,
        ring(p.chip, String::new()),
        progress.unwrap_or_default()
    );
    svg(svg::Handle::from_memory(svg_text.into_bytes())).width(205).height(205).into()
}

/// Desktop's `QRCode` of `data`: the bytes in error correction level L, 4 pixels for each module, and a
/// margin of 4 modules, like the `qrcode` package of Desktop.
pub(crate) fn qr_code<'a>(data: &[u8]) -> El<'a> {
    use qrcodegen::{QrCode, QrCodeEcc, QrSegment};
    let code = QrCode::encode_segments(&[QrSegment::make_bytes(data)], QrCodeEcc::Low).expect("a QR code holds the data");
    let px = ((code.size() + 8) * 4) as f32;
    svg::Svg::new(crate::style::qr_svg(&code, 4)).width(px).height(px).into()
}

/// Desktop's `BeeperVerify` illustration of the "Verify" screen.
static VERIFY_ART: LazyLock<svg::Handle> =
    LazyLock::new(|| svg::Handle::from_memory(include_bytes!("../assets/beeper-verify.svg").as_slice()));

/// The onboarding screens of encrypted messaging, which Desktop shows after the sign-in.
impl BeeperGuiApp {
    /// Desktop's `pre-verification` stage: reads what encrypted messaging needs once Matrix is connected.
    pub(crate) fn check_e2ee(&mut self) -> Task<UiMessage> {
        // The screens of encrypted messaging have no keyboard stops, so the focus of the sign-in screens ends.
        (self.login.busy, self.login.focus) = (true, Default::default());
        Task::perform(beeper_core::matrix::e2ee_setup(self.store.clone()), |r| LoginMsg::E2ee(r).into())
    }

    /// Desktop's routing of `pre-verification`: verify with another device when one can do it, else with the
    /// recovery key.
    pub(crate) fn e2ee_checked(&mut self, r: Result<E2eeSetup, String>) -> Task<UiMessage> {
        let l = &mut self.login;
        l.busy = false;
        match r {
            Ok(s) if s.verified => return self.advance_onboarding(),
            Ok(s) if !s.cross_signing => l.screen = Screen::Preamble,
            Ok(s) if s.devices.is_empty() => l.screen = Screen::RecoveryKey,
            Ok(s) => (l.screen, l.devices) = (Screen::Verify, s.devices),
            Err(e) => (l.screen, l.error) = (Screen::Verify, Some(e)),
        }
        match l.screen {
            Screen::Verify => self.start_login_verification(),
            _ => Task::none(),
        }
    }

    /// Desktop's `AwaitVerification` asks the other devices to verify this device each time it shows.
    pub(crate) fn start_login_verification(&mut self) -> Task<UiMessage> {
        (self.login.another_way, self.login.waiting) = (false, false);
        let s = self.store.clone();
        call("Failed to start verification", async move { beeper_core::matrix::start_verification(&s, "login").await })
    }

    /// "Enter Recovery Key": cancels the verification and asks for the recovery key.
    pub(crate) fn use_recovery_key(&mut self) -> Task<UiMessage> {
        (self.login.screen, self.login.waiting, self.login.error) = (Screen::RecoveryKey, false, None);
        let id = self.store.read().app_verification().filter(|v| !matches!(v.state, "done" | "cancelled")).map(|v| v.id.clone());
        id.map_or_else(Task::none, |id| cancel(self.store.clone(), id))
    }

    /// Desktop's `advanceOnboarding`: the next screen once this device is verified. An account with a connected chat
    /// account skips Connect Chat Accounts, like Desktop's `completeSigninIn`.
    pub(crate) fn advance_onboarding(&mut self) -> Task<UiMessage> {
        // A store change can end the onboarding before the result of its Matrix call comes.
        if self.login_window.is_none() {
            return Task::none();
        }
        // Desktop's `IndicateSyncing` waits for the keys and the first sync of a verified device.
        let (verified, waiting, connected) = {
            let st = self.store.read();
            let s = &st.beeper_state;
            (s.verified, st.syncing || s.verified && s.stage() != "done", st.has_connected_network())
        };
        let screen = self.login.screen;
        let confirmed = screen == Screen::ConfirmKey && self.store.read().pending_recovery_key.is_none();
        // The sign-in screens wait for the result of their Matrix call, for example the check of encrypted messaging.
        let sign_in_done = self.login.signing_in() && !self.login.busy;
        let checking = sign_in_done || matches!(screen, Screen::Verify | Screen::RecoveryKey) || confirmed;
        match (checking, screen, verified, waiting) {
            (true, Screen::Verify | Screen::RecoveryKey, false, _) => Task::none(),
            (true, _, _, true) => {
                (self.login.screen, self.login.late) = (Screen::Syncing, false);
                let late = async { tokio::time::sleep(std::time::Duration::from_secs(60)).await };
                Task::perform(late, |()| LoginMsg::KeysLate.into())
            }
            (true, _, true, false) | (false, Screen::Syncing, true, false) if connected => {
                self.login = Login::default();
                self.enter_app(beeper_core::store::NOTE_CHAT_ID)
            }
            // Desktop's sign-in window becomes resizable on Connect Chat Accounts.
            (true, _, _, false) | (false, Screen::Syncing, _, false) => {
                self.login.screen = Screen::Accounts;
                window::set_resizable(self.login_window.expect("an open sign-in window, checked above"), true)
            }
            _ => Task::none(),
        }
    }

    /// Desktop's `AwaitVerification`: the request to the other devices, then their QR code or their emojis.
    pub(crate) fn view_onboarding_verify(&self) -> El<'_> {
        let l = &self.login;
        let p = &self.pal;
        let st = self.store.read();
        let v = st.app_verification().filter(|v| v.purpose == "login");
        let emojis = v.and_then(|v| v.sas.clone().filter(|_| v.state == "sas_ready"));
        let qr = v.and_then(|v| v.qr.clone().filter(|_| v.state == "ready"));
        let can_emojis = v.is_some_and(|v| v.methods.contains(&"sas") && v.state == "ready");
        drop(st);
        let secondary = |label, msg: UiMessage| brand_btn(p, label, Brand::Secondary, msg);
        let use_key = || secondary(trx("Enter Recovery Key", "button label"), LoginMsg::UseRecoveryKey.into());
        let emojis_btn = || can_emojis.then(|| secondary(trx("Confirm using emojis", "button label"), VerifyMsg::Emojis.into()));
        let note = |s: &'static str| txt(tr(s), 13.0).color(p.strong).center().width(Fill);
        let (middle, bottom): (El<'_>, Vec<El<'_>>) = match (l.waiting, emojis, qr) {
            (true, _, _) => {
                let wait = column![spinner(39.0, p.strong), note("Waiting for you to verify on your other device...")];
                (container(wait.spacing(16).align_x(Center)).center(Fill).into(), Vec::new())
            }
            (false, Some(sas), _) => {
                let glyphs = sas.emojis.into_iter().map(|e| {
                    let glyph = crate::style::fixed(e.emoji, 28.0, iced::font::Weight::Normal).line_height(1.0).font(EMOJI_FONT);
                    let tip = container(txt(e.description, 12.0).color(p.strong)).padding([4, 8]).style(fill_box(p.elevated, 6.0));
                    tooltip(glyph, tip, tooltip::Position::Bottom).into()
                });
                let buttons = vec![
                    container(note("Confirm the emojis match the ones shown on your other device."))
                        .padding(Padding::ZERO.bottom(8))
                        .into(),
                    brand_btn(p, trx("They match", "button label"), Brand::Primary, VerifyMsg::Match(true).into()),
                    secondary(trx("They don't match", "button label"), VerifyMsg::Match(false).into()),
                    use_key(),
                    lost_link(p),
                ];
                (container(row(glyphs).spacing(4)).center(Fill).into(), buttons)
            }
            (false, None, Some(qr)) => {
                let plate = container(qr_code(&qr.data)).padding(11).style(|_: &Theme| container::Style {
                    background: Some(brand_gradient(p)),
                    border: border::rounded(12),
                    ..Default::default()
                });
                let mut buttons: Vec<El<'_>> = vec![
                    note("Scan this QR Code using Beeper iOS or Android app.").into(),
                    note(trx("Or", "verification method divider")).into(),
                ];
                buttons.extend(emojis_btn());
                buttons.push(use_key());
                buttons.push(lost_link(p));
                (container(plate).center(Fill).into(), buttons)
            }
            (false, None, None) => {
                let devices = self.login.devices.iter().take(5).map(|d| -> El<'_> {
                    let seen = d
                        .last_seen_ms
                        .map(|t| txt(tr("seen %(lastSeen)s").replace("%(lastSeen)s", &crate::ago(t)), 11.0).color(p.on_secondary));
                    column![txt(d.name.clone(), 13.0).color(p.strong)].push(seen).into()
                });
                let devices: El<'_> = match self.login.devices.is_empty() {
                    true => txt(tr("No devices found"), 13.0).color(p.on_secondary).into(),
                    false => Column::with_children(devices).spacing(6).into(),
                };
                let tip = container(devices).padding([8, 12]).style(shadow_box(p.elevated, p.chip, 8.0, 10.0));
                let another = row![semi(tr("another device"), 17.0).color(p.strong), ic(icon::INFO, 16.0).color(p.on_secondary)]
                    .spacing(4)
                    .align_y(Center);
                let title = column![
                    semi("Open Beeper", 17.0).color(p.strong),
                    row![
                        semi("on ", 17.0).color(p.strong),
                        tooltip(another, tip, tooltip::Position::Top),
                        semi(" and", 17.0).color(p.strong)
                    ]
                    .align_y(Center),
                    semi("follow instructions to verify", 17.0).color(p.strong),
                ]
                .align_x(Center)
                .width(Fill);
                let link =
                    |s: &'static str, m: UiMessage| -> El<'_> { button(txt(s, 13.0).color(p.accent)).bare(p.accent).on_press(m).into() };
                let mut buttons = vec![title.into()];
                match l.another_way {
                    false => buttons
                        .push(container(link(trx("Try another way", "button label"), LoginMsg::AnotherWay.into())).center_x(Fill).into()),
                    true => {
                        buttons.push(note(trx("or", "verification method divider")).into());
                        buttons.extend(emojis_btn());
                        buttons.push(use_key());
                        buttons.push(lost_link(p));
                    }
                }
                (container(svg(VERIFY_ART.clone()).width(241).height(224)).center(Fill).into(), buttons)
            }
        };
        let error = l.error.as_deref().map(|e| txt(e, 12.0).color(p.text_danger).center().width(Fill));
        column![Space::new().height(34), heading(trx("Verify", "onboarding verification heading"), 36.0), middle]
            .push(error)
            .push(actions(bottom, 24.0))
            .into()
    }

    /// Desktop's `AwaitRecoveryCode`.
    pub(crate) fn view_onboarding_key(&self) -> El<'_> {
        let l = &self.login;
        let p = &self.pal;
        let next = (!l.key_input.text().trim().is_empty()).then(|| LoginMsg::SubmitRecoveryKey.into());
        let next = brand_btn_maybe(p, trx("Next", "button label"), Brand::Primary, next, l.busy);
        let error = l.error.as_deref().map(|e| txt(e, 12.0).color(p.text_danger).center().width(Fill));
        column![
            Space::new().height(40),
            heading(tr("Enter your Recovery Key"), H3),
            key_input(p, &l.key_input, l.error.is_some(), LOGIN_EDIT, LoginMsg::PasteKey.into()),
            Space::new().height(Fill),
        ]
        .push(error)
        .push(actions(vec![next, lost_link(p)], 24.0))
        .into()
    }

    /// Desktop's `IndicateSyncing`, while the keys and the first sync of the chats come: the stage, the progress of
    /// the sync, and after 60 s without the keys from the other device, the recovery key.
    pub(crate) fn view_onboarding_syncing(&self) -> El<'_> {
        let p = &self.pal;
        let st = self.store.read();
        let stage = st.beeper_state.stage();
        let (processed, total) = st.initial_sync;
        drop(st);
        let new = self.login.new_account();
        let title = match stage {
            _ if new => tr("Welcome to Beeper!"),
            "awaiting-secrets" => tr("Setting up encryption…"),
            "awaiting-key-backup" => tr("Restoring encryption keys…"),
            _ => tr("Syncing your data"),
        };
        let paragraph = |s: String| txt(s, 13.0).color(p.strong).center().width(Fill);
        let body: El<'_> = match (new, self.login.late && stage == "awaiting-secrets") {
            (true, _) => column![spinner(64.0, p.strong), paragraph(tr("We're getting your account ready...").into())]
                .spacing(24)
                .align_x(Center)
                .into(),
            (false, true) => {
                let text = "Your other device has not sent your private keys yet. You may want to verify with your recovery key instead.";
                let key = brand_btn(p, trx("Use your Recovery Key", "button label"), Brand::Primary, LoginMsg::UseRecoveryKey.into());
                column![paragraph(tr(text).into()), key].spacing(24).into()
            }
            _ if processed == 0 || total == 0 => paragraph(tr("We're syncing your chats. This may take a while...").into()).into(),
            _ => {
                let (one, many) = ("Synced %(processed)d of %(total)d chat.", "Synced %(processed)d of %(total)d chats.");
                paragraph(trn_named(one, many, total, &[("processed", &processed), ("total", &total)])).into()
            }
        };
        let circle = (!new).then(|| container(progress_circle(p, processed, total)).center_x(Fill));
        column![Space::new().height(34), heading(title, H3)].push(circle).push(container(body).center(Fill).padding(18)).into()
    }
}

/// Desktop's "Lost your recovery key?" link.
fn lost_link<'a>(p: &Palette) -> El<'a> {
    let label = txt(tr("Lost your recovery key?"), 13.0).color(p.accent);
    let link = button(label).bare(p.accent).on_press(LoginMsg::LostKey.into());
    container(link).center_x(Fill).into()
}

/// Desktop's display of a recovery key: 12 groups of 4 characters with dashes, 3 groups on each line.
pub(crate) fn format_recovery_key(key: &str) -> String {
    let chars: Vec<char> = beeper_core::matrix::recovery_key_chars(key).chars().take(48).collect();
    let groups: Vec<String> = chars.chunks(4).map(|g| g.iter().collect()).collect();
    groups.chunks(3).map(|line| line.join("-")).collect::<Vec<_>>().join("\n")
}

/// Desktop's `RecoveryCodeInput`: the key in 4 lines of 3 dash groups, and "Paste from Clipboard".
/// The widget ID of the recovery key fields.
pub(crate) const KEY_INPUT: &str = "recovery-key-input";

pub(crate) fn key_input<'a>(
    p: &Palette, content: &'a text_editor::Content, error: bool, edit: fn(text_editor::Action) -> UiMessage, on_paste: UiMessage,
) -> El<'a> {
    container(key_field(p, content, error, edit, on_paste)).padding([24, 18]).into()
}

/// [`key_input`] without the space of the onboarding around it.
pub(crate) fn key_field<'a>(
    p: &Palette, content: &'a text_editor::Content, error: bool, edit: fn(text_editor::Action) -> UiMessage, on_paste: UiMessage,
) -> El<'a> {
    let p = *p;
    let edge = if error { p.text_danger } else { CLEAR };
    let editor = text_editor(content).id(KEY_INPUT).on_action(edit).size(rem(17.0)).height(4.0 * 17.0 * 1.3 + 24.0).padding(12).style(
        move |_: &Theme, _| text_editor::Style {
            background: p.chip.into(),
            border: border::rounded(12).color(edge).width(1),
            placeholder: p.on_secondary,
            value: p.strong,
            selection: p.accent.scale_alpha(0.3),
        },
    );
    let paste = txt(trx("Paste from Clipboard", "button label"), 12.0).color(p.on_secondary);
    let paste = button(paste).padding(4).style(btn_style([CLEAR, p.chip, p.chip], [p.on_secondary; 3], 6.0));
    column![editor, paste.on_press(on_paste)].spacing(4).align_x(Center).into()
}

/// Desktop's `useAutoSubmitRecoveryCode`: the recovery key on the clipboard, unless it was tried.
pub(crate) fn clipboard_key(text: Option<String>, tried: &[String]) -> Option<String> {
    let key = beeper_core::matrix::recovery_key_chars(&text.unwrap_or_default());
    (valid_recovery_key(&key) && !tried.contains(&key)).then_some(key)
}

/// An edit of `RecoveryCodeInput`: the key keeps its format, and the caret stays after the same character.
pub(crate) fn edit_recovery_key(content: &mut text_editor::Content, action: text_editor::Action) {
    let edit = action.is_edit();
    content.perform(action);
    if !edit {
        return;
    }
    let (text, at) = (content.text(), content.cursor().position);
    let lines: Vec<&str> = text.split('\n').collect();
    let count = |s: &str| s.chars().filter(char::is_ascii_alphanumeric).count();
    let before = lines[..at.line].iter().map(|l| count(l)).sum::<usize>()
        + lines.get(at.line).map_or(0, |l| count(&l.chars().take(at.column).collect::<String>()));
    // Desktop's caret: after the same character, and after the dash or the line end that follows a full group.
    let formatted = format_recovery_key(&text);
    let caret = ((0..before.min(48)).map(|i| 1 + usize::from(i % 4 == 3 && i < 47)).sum::<usize>()).min(formatted.len());
    let line = formatted[..caret].matches('\n').count();
    let column = caret - formatted[..caret].rfind('\n').map_or(0, |i| i + 1);
    *content = text_editor::Content::with_text(&formatted);
    let position = text_editor::Position { line, column };
    content.move_to(text_editor::Cursor { position, selection: None });
}

/// The edits of the recovery key of the sign-in window.
const LOGIN_EDIT: fn(text_editor::Action) -> UiMessage = |a| LoginMsg::KeyEdit(a).into();

/// Desktop's `isValidRecoveryCode`: 48 letters and digits.
pub(crate) fn valid_recovery_key(key: &str) -> bool {
    beeper_core::matrix::recovery_key_chars(key).len() == 48
}

impl BeeperGuiApp {
    /// Desktop's `beginResetCrossSigning` on the "Save Your Recovery Key" screen.
    pub(crate) fn make_recovery_key(&mut self) -> Task<UiMessage> {
        let l = &mut self.login;
        (l.screen, l.new_key, l.copied, l.error, l.busy) = (Screen::NewKey, None, false, None, true);
        let s = self.store.clone();
        Task::perform(async move { beeper_core::matrix::new_recovery_key(&s).await }, |r| LoginMsg::MadeKey(r).into())
    }

    /// "Lost your recovery key?": cancels the verification and shows the reset warning.
    pub(crate) fn lost_key(&mut self) -> Task<UiMessage> {
        let cancel = self.use_recovery_key();
        self.login.screen = Screen::Reset;
        cancel
    }

    /// Desktop's `RecoveryCodePreamble`, for an account without cross-signing.
    pub(crate) fn view_onboarding_preamble(&self) -> El<'_> {
        let p = &self.pal;
        let text = tr("If you lose your device you'll need your Recovery Key to access your encrypted chat backup.");
        let save = brand_btn(p, trx("Save Key", "button label"), Brand::Primary, LoginMsg::NewKey.into());
        column![Space::new().height(40), heading(tr("Save Your Recovery Key"), H3), container(art(3, 240.0)).center(Fill),]
            .push(actions(vec![txt(text, 13.0).color(p.strong).center().width(Fill).into(), save], 24.0))
            .into()
    }

    /// Desktop's `AwaitNewRecoveryCode`: the new key to copy, then the key again to confirm that it was saved.
    pub(crate) fn view_onboarding_new_key(&self) -> El<'_> {
        let l = &self.login;
        let p = &self.pal;
        let warning = tr("WARNING: Beeper does not store your Recovery Key for you. If you lose your Recovery Key, you may lose access to your encrypted chat backup.");
        let warning = txt(warning, 11.0).color(p.text_danger).center().width(Fill).into();
        let error = l.error.as_deref().map(|e| txt(e, 12.0).color(p.text_danger).center().width(Fill));
        let (title, middle, next): (&str, El<'_>, Option<UiMessage>) = match l.screen {
            Screen::ConfirmKey => {
                let next = valid_recovery_key(&l.key_input.text()).then(|| LoginMsg::SubmitConfirmKey.into());
                (
                    tr("Confirm your Recovery Key"),
                    key_input(p, &l.key_input, l.error.is_some(), LOGIN_EDIT, LoginMsg::PasteKey.into()),
                    next,
                )
            }
            _ => {
                let key: El<'_> = match &l.new_key {
                    Some(key) => {
                        let copy = if l.copied { tr("Copied!") } else { tr("Copy to Clipboard") };
                        let body =
                            column![txt(format_recovery_key(key), 17.0).color(p.strong).center(), txt(copy, 12.0).color(p.on_secondary),]
                                .spacing(12)
                                .align_x(Center);
                        button(container(body).center_x(Fill).padding(16))
                            .padding(0)
                            .style(btn_style([p.chip; 3], [p.strong; 3], 12.0))
                            .on_press(LoginMsg::CopyKey.into())
                            .into()
                    }
                    None => Space::new().height(120).into(),
                };
                let next = l.new_key.is_some().then(|| LoginMsg::KeyNext.into());
                (tr("Save Your Recovery Key"), container(key).padding([24, 18]).into(), next)
            }
        };
        let next = brand_btn_maybe(p, trx("Next", "button label"), Brand::Primary, next, false);
        column![Space::new().height(40), heading(title, H3), middle, Space::new().height(Fill)]
            .push(error)
            .push(actions(vec![warning, next], 24.0))
            .into()
    }

    /// Desktop's `ConfirmResetRecoveryCode`.
    pub(crate) fn view_onboarding_reset(&self) -> El<'_> {
        let p = &self.pal;
        let bullet =
            |s: &'static str| -> El<'_> { row![txt("•", 13.0).color(p.strong), txt(tr(s), 13.0).color(p.strong)].spacing(8).into() };
        let warning = column![
            semi(trx("Warning", "recovery key reset warning heading"), 15.0).color(p.text_danger),
            bullet("All of your data is encrypted with your Recovery Key. All your accounts will be disconnected. Your chat history and settings will be lost."),
            bullet("Only use this option if you no longer have access to your other devices. You’ll need to re-verify all your other devices after reset."),
        ]
        .spacing(12)
        .padding([0, 24]);
        let reset = brand_btn(p, trx("Reset Recovery Key", "button label"), Brand::Danger, LoginMsg::ConfirmReset.into());
        let back = brand_btn(p, trx("Cancel", "button label"), Brand::Muted, LoginMsg::Show(Screen::Verify).into());
        let title = t(tr("Reset your Recovery Key"), H3, crate::style::display(Weight::Bold, H3)).color(p.text_danger).center().width(Fill);
        column![Space::new().height(40), container(title).padding([0, 8]), container(warning).center(Fill)]
            .push(actions(vec![reset, back], 24.0))
            .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use beeper_core::models::{AppVerification, VerificationDevice, VerificationQr, VerificationSas, VerificationSasEmoji};
    use iced_test::simulator::simulator;

    fn request(device: Option<VerificationDevice>) -> AppVerification {
        AppVerification {
            id: "flow".into(),
            purpose: "device",
            direction: "incoming",
            state: "requested",
            methods: Vec::new(),
            available_actions: vec!["accept", "cancel"],
            other_user_id: Some("@alex:beeper.com".into()),
            other_device: device,
            qr: None,
            sas: None,
            error: None,
        }
    }

    #[test]
    fn device_name_prefers_the_display_name_then_the_device_id() {
        let mut v = request(None);
        assert_eq!(device_name(&v), "Unknown device");
        v.other_device = Some(VerificationDevice { id: "DEV".into(), name: None });
        assert_eq!(device_name(&v), "DEV");
        v.other_device = Some(VerificationDevice { id: "DEV".into(), name: Some("Pixel".into()) });
        assert_eq!(device_name(&v), "Pixel");
    }

    #[test]
    fn request_toast_modal_and_end_follow_the_verification_like_desktop() {
        let mut app = crate::fuzz::demo_app("verify-request");
        let set = |app: &BeeperGuiApp, change: &dyn Fn(&mut AppVerification)| {
            let mut st = app.store.write();
            change(st.verification.get_or_insert_with(|| request(None)));
        };
        set(&app, &|v| v.other_device = Some(VerificationDevice { id: "DEV".into(), name: Some("Pixel".into()) }));
        let _ = app.verification_changed();
        let shown = app.shown_toast().expect("the request shows a toast");
        assert_eq!(shown.text, "Verification request from Pixel");
        let buttons: Vec<&str> = shown.buttons.iter().map(|b| b.0).collect();
        assert_eq!(buttons, ["Accept", "Decline"]);
        assert_eq!(app.sticky_banner.as_deref(), Some("Verification request from Pixel"), "the toast stays");
        let _ = app.verification_changed();
        assert_eq!(app.status_banner, "Verification request from Pixel", "an unchanged state shows nothing new");

        app.verify.window = Some(window::Id::unique());
        assert!(simulator(app.view_verify()).find("Loading...").is_ok(), "the modal waits for the emojis");
        let emojis = ["🐶", "🐱", "🦁", "🐎", "🦄", "🐷", "🐘"]
            .map(|e| VerificationSasEmoji { emoji: e.into(), description: "Dog".into() })
            .to_vec();
        set(&app, &|v| {
            (v.state, v.methods) = ("sas_ready", vec!["sas"]);
            v.sas = Some(VerificationSas { emojis: emojis.clone(), decimals: "1 2 3".into() });
        });
        let _ = app.verification_changed();
        let mut sim = simulator(app.view_verify());
        for text in [
            "Emoji Verification",
            "🐶",
            "They match",
            "They don't match",
            "Proceeding will give the other device full access to your Beeper account",
        ] {
            assert!(sim.find(text).is_ok(), "the modal shows {text}");
        }
        let _ = sim.click("They match");
        assert!(sim.into_messages().any(|m| matches!(m, UiMessage::Verify(VerifyMsg::Match(true)))));

        set(&app, &|v| v.state = "done");
        let _ = app.verification_changed();
        assert_eq!(app.status_banner, "Verification successful");
        assert!(app.verify.window.is_none(), "the end closes the modal");

        set(&app, &|v| (v.id, v.state) = ("other".into(), "requested"));
        let _ = app.verification_changed();
        set(&app, &|v| v.state = "cancelled");
        let _ = app.verification_changed();
        assert!(app.older_toasts.iter().all(|t| t.0.text != "Verification request from Pixel"));
        assert_ne!(app.status_banner, "Verification request from Pixel", "a cancel removes the request toast");
    }

    #[test]
    fn request_notification_accepts_declines_and_closes_like_desktop() {
        let mut app = crate::fuzz::demo_app("verify-notification");
        let device = Some(VerificationDevice { id: "DEV".into(), name: Some("Pixel".into()) });
        app.store.write().verification = Some(request(device));
        let _ = app.verification_changed();
        let _ = app.update(VerifyMsg::Noted("flow".into(), Ok(7)).into());
        assert_eq!(app.verify.note, Some((7, "flow".into())));
        let _ = app.update(UiMessage::Notify(crate::notify::Msg::Event(7, Some("verificationAccept".into()))));
        assert!(app.verify.note.is_none() && app.verify.window.is_some(), "Accept opens the modal");
        assert_ne!(app.status_banner, "Verification request from Pixel", "Accept removes the toast");

        app.verify.note = Some((8, "flow".into()));
        app.store.write().verification.as_mut().expect("a verification").state = "cancelled";
        let _ = app.verification_changed();
        assert!(app.verify.note.is_none(), "a cancel closes the notification");
        let _ = app.update(VerifyMsg::Noted("flow".into(), Ok(9)).into());
        assert!(app.verify.note.is_none(), "a late notification of an ended request closes");
    }

    #[test]
    fn modal_shows_the_qr_code_and_a_scan_confirms_it_like_desktop() {
        let mut app = crate::fuzz::demo_app("verify-qr");
        let mut v = request(Some(VerificationDevice { id: "DEV".into(), name: None }));
        (v.direction, v.state, v.methods) = ("outgoing", "ready", vec!["qr", "sas"]);
        v.qr = Some(VerificationQr { data: b"MATRIX\x02\x02".to_vec() });
        app.store.write().verification = Some(v);
        let _ = app.verification_changed();
        app.verify.window = Some(window::Id::unique());
        let mut sim = simulator(app.view_verify());
        for text in ["QR Verification", "Scan the QR code with your device to verify your session."] {
            assert!(sim.find(text).is_ok(), "the modal shows {text}");
        }
        let _ = sim.click("Confirm using emojis instead");
        assert!(sim.into_messages().any(|m| matches!(m, UiMessage::Verify(VerifyMsg::Emojis))));
        app.store.write().verification.as_mut().expect("a verification").methods = vec!["qr"];
        assert!(simulator(app.view_verify()).find("Confirm using emojis instead").is_err(), "no emojis without SAS");

        app.store.write().verification.as_mut().expect("a verification").state = "qr_scanned";
        let _ = app.verification_changed();
        assert_eq!(app.status_banner, "Authentication confirmed, restoring the cryptographic state");
        assert!(app.verify.window.is_none(), "a scan closes the modal");
    }

    #[test]
    fn indicate_syncing_waits_for_the_keys_and_counts_the_chats_like_desktop() {
        let mut app = crate::fuzz::demo_app("verify-syncing");
        app.login_window = Some(window::Id::unique());
        (app.login.screen, app.store.write().beeper_state.secrets.megolm_backup_key) = (Screen::RecoveryKey, false);
        let _ = app.update(LoginMsg::Recovered(Ok(())).into());
        assert_eq!(app.login.screen, Screen::Syncing, "a verified device waits for its keys");
        let shows = |app: &BeeperGuiApp, texts: &[&str]| {
            let mut sim = simulator(app.view_login());
            for text in texts {
                assert!(sim.find(*text).is_ok(), "the screen shows {text}");
            }
        };
        assert_eq!(app.store.read().beeper_state.stage(), "awaiting-secrets");
        shows(&app, &["We're syncing your chats. This may take a while..."]);
        let _ = app.update(LoginMsg::KeysLate.into());
        shows(&app, &["Use your Recovery Key"]);
        let _ = app.update(LoginMsg::UseRecoveryKey.into());
        assert_eq!(app.login.screen, Screen::RecoveryKey, "the recovery key gives the keys");

        app.login.screen = Screen::Syncing;
        (app.store.write().beeper_state.secrets.megolm_backup_key, app.store.write().initial_sync) = (true, (3, 40));
        shows(&app, &["Synced 3 of 40 chats."]);
        let _ = app.update(UiMessage::StoreChanged);
        assert!(app.login_window.is_none(), "the keys and the sync end the wait");
    }

    /// Desktop's `o1` Enter hotkey of the recovery key screens runs in the key box (mousetrap's `stopCallback` is
    /// off) and prevents its newline: it submits the key as it is, compares the confirmed key, or resets.
    #[test]
    fn enter_submits_the_recovery_key_screens_like_desktop() {
        use iced::keyboard::{key::Named, Key, Modifiers};
        let mut app = crate::fuzz::demo_app("recovery-enter");
        let login = window::Id::unique();
        app.login_window = Some(login);
        let enter = |app: &mut BeeperGuiApp| _ = app.update(UiMessage::WindowKey(login, Key::Named(Named::Enter), Modifiers::empty()));
        app.login.screen = Screen::RecoveryKey;
        let _ = app.update(LoginMsg::KeyEdit(text_editor::Action::Edit(text_editor::Edit::Insert('a'))).into());
        let _ = app.update(LoginMsg::KeyEdit(text_editor::Action::Edit(text_editor::Edit::Enter)).into());
        assert_eq!(app.login.key_input.text(), "a", "no newline goes in");
        enter(&mut app);
        assert!(app.login.busy, "a partial key is submitted too");
        assert_eq!(app.login.tried_keys, ["a"]);
        enter(&mut app);
        assert_eq!(app.login.tried_keys.len(), 1, "not again while it is checked");
        let _ = app.update(LoginMsg::Recovered(Err("Invalid recovery key".into())).into());
        // The demo account needs no more onboarding, which closes the window.
        (app.login_window, app.login.screen) = (Some(login), Screen::ConfirmKey);
        assert!(!app.login.busy, "the check ended");
        app.store.write().pending_recovery_key = Some("EsTG bqMm dUDy dKth xAyD 45dS HhyC WKc9 CF6B xsFi J5FQ t8u2".into());
        let _ = app.update(LoginMsg::Pasted(Some("EsTG".into())).into());
        enter(&mut app);
        let error = "The Recovery Key you entered doesn't match your Recovery Key.";
        assert_eq!(app.login.error.as_deref(), Some(error), "the confirmation compares a partial key");
        app.login.screen = Screen::Reset;
        enter(&mut app);
        assert!(app.login.busy, "Enter resets like Reset Recovery Key");
    }

    #[test]
    fn onboarding_verifies_with_another_device_or_the_recovery_key_like_desktop() {
        use beeper_core::matrix::DeviceInfo;
        let mut app = crate::fuzz::demo_app("verify-onboarding");
        app.login_window = Some(window::Id::unique());
        app.store.write().beeper_state.verified = false;
        app.login.screen = Screen::Code;
        app.login.focus.set(Some(crate::login::Stop::Resend));
        let _ = app.check_e2ee();
        app.login.screen = Screen::RecoveryKey;
        let _ = app.login_text(" ");
        assert!(app.login.focus.get().is_none(), "the keys of the encryption screens find no stale stop of the sign-in");
        let phone = DeviceInfo {
            id: "PHONE".into(),
            name: "Pixel".into(),
            model: None,
            kind: None,
            last_seen_ms: None,
            verified: true,
            current: false,
        };
        let setup = |devices: Vec<DeviceInfo>| E2eeSetup { cross_signing: true, verified: false, devices };
        let _ = app.e2ee_checked(Ok(setup(vec![phone])));
        assert_eq!(app.login.screen, Screen::Verify);
        let shows = |app: &BeeperGuiApp, texts: &[&str]| {
            let mut sim = simulator(app.view_login());
            for text in texts {
                assert!(sim.find(*text).is_ok(), "the screen shows {text}");
            }
        };
        shows(&app, &["Open Beeper", "another device", "follow instructions to verify", "Try another way"]);
        let _ = app.update(LoginMsg::AnotherWay.into());
        shows(&app, &["or", "Enter Recovery Key"]);

        let mut v = request(None);
        (v.purpose, v.direction, v.state, v.methods) = ("login", "outgoing", "ready", vec!["qr", "sas"]);
        v.qr = Some(VerificationQr { data: b"MATRIX\x02\x02".to_vec() });
        app.store.write().verification = Some(v);
        shows(&app, &["Scan this QR Code using Beeper iOS or Android app.", "Or", "Confirm using emojis"]);
        let emojis = vec![VerificationSasEmoji { emoji: "🐶".into(), description: "Dog".into() }];
        let sas = VerificationSas { emojis, decimals: "1 2 3".into() };
        app.store.write().verification.as_mut().expect("a verification").sas = Some(sas);
        app.store.write().verification.as_mut().expect("a verification").state = "sas_ready";
        shows(&app, &["🐶", "Confirm the emojis match the ones shown on your other device.", "They match"]);
        let _ = app.update(VerifyMsg::Match(true).into());
        shows(&app, &["Waiting for you to verify on your other device..."]);

        let accounts = std::mem::take(&mut app.store.write().bridge_accounts);
        app.store.write().beeper_state.verified = true;
        let _ = app.update(UiMessage::StoreChanged);
        assert_eq!(app.login.screen, Screen::Accounts, "a verified device goes on to the accounts");
        shows(&app, &["WhatsApp", "Discord", "GroupMe", "10 free accounts remaining."]);
        assert!(simulator(app.view_login()).find("Continue").is_err(), "Continue needs a connected chat account");
        let _ = app.update(LoginMsg::Query("sig".into()).into());
        shows(&app, &["Signal"]);
        assert!(simulator(app.view_login()).find("WhatsApp").is_err(), "search filters the networks");
        let _ = app.update(LoginMsg::Query("zzz".into()).into());
        shows(&app, &["No networks found"]);

        app.login.screen = Screen::Code;
        app.store.write().beeper_state.verified = false;
        let _ = app.e2ee_checked(Ok(setup(Vec::new())));
        assert_eq!(app.login.screen, Screen::RecoveryKey, "without another device the recovery key verifies");
        shows(&app, &["Next"]);
        let _ = app.update(LoginMsg::Recovered(Err("bad key".into())).into());
        shows(&app, &["Error: bad key"]);

        app.store.write().bridge_accounts = accounts;
        app.store.write().beeper_state.verified = true;
        let _ = app.update(LoginMsg::Recovered(Ok(())).into());
        assert!(app.login_window.is_none(), "a connected chat account skips Connect Chat Accounts");
    }

    #[test]
    fn a_recovery_key_that_ends_the_onboarding_before_its_result_comes_does_not_stop_the_app() {
        let mut app = crate::fuzz::demo_app("verify-late-result");
        let main = app.main_window.take();
        (app.login_window, app.login.screen, app.login.busy) = (main, Screen::RecoveryKey, true);
        let _ = app.update(UiMessage::StoreChanged);
        assert!(app.login_window.is_none() && app.main_window.is_some(), "the keys of the recovery key end it");
        let _ = app.update(LoginMsg::Recovered(Ok(())).into());
        assert!(app.main_window.is_some(), "the late result changes nothing");
    }

    #[test]
    fn a_restart_before_the_end_of_the_onboarding_continues_it_like_desktop() {
        use beeper_core::config::CentralConfig;
        use beeper_core::store::{BeeperStore, StoreLock};
        let config = CentralConfig::new(beeper_core::testing::temp_dir("verify-restart"), "127.0.0.1".into(), 23391, None);
        let session = beeper_core::models::MatrixSession {
            user_id: "@me:beeper.com".into(),
            access_token: "t".into(),
            device_id: "D".into(),
            jwt: None,
        };
        let mut st = BeeperStore::from_config(config.clone());
        st.sign_in(session, "http://127.0.0.1:1".into(), &Default::default(), Vec::new());
        drop(st);
        let boot = || BeeperGuiApp::boot(StoreLock::shared(BeeperStore::from_config(config.clone()))).0;
        let app = boot();
        assert!(app.main_window.is_none(), "an unverified device does not show the chats");
        assert!(app.login_window.is_some() && app.login.busy, "the onboarding checks encrypted messaging again");
        assert_eq!(app.login.screen, Screen::Email);

        let mut st = BeeperStore::from_config(config.clone());
        let s = &mut st.beeper_state;
        assert_eq!(s.stage(), "unknown", "Desktop waits for the state of encrypted messaging");
        s.initialized = true;
        assert_eq!(s.stage(), "pre-verification");
        (s.verified, s.key_backup, s.first_sync_done) = (true, true, true);
        assert_eq!(s.stage(), "awaiting-secrets", "a verified device waits for its keys");
        let k = &mut s.secrets;
        (k.master_key, k.self_signing_key, k.user_signing_key, k.megolm_backup_key) = (true, true, true, true);
        assert_eq!(s.stage(), "done");
        st.persist_state_to_sqlite();
        drop(st);
        let app = boot();
        assert!(app.main_window.is_some() && app.login_window.is_none(), "a done onboarding shows the chats");
    }

    #[test]
    fn a_device_that_loses_its_keys_shows_the_onboarding_again_like_desktop() {
        type Drop = fn(&mut beeper_core::models::BeeperStateRecord);
        let drops: [(&str, Drop); 4] = [
            ("syncing", |s| s.first_sync_done = false),
            ("awaiting-secrets", |s| s.secrets.megolm_backup_key = false),
            ("awaiting-key-backup", |s| s.key_backup = false),
            ("pre-verification", |s| s.verified = false),
        ];
        let [.., mut app] = drops.map(|(stage, drop)| {
            let mut app = crate::fuzz::demo_app(&format!("verify-in-session-{stage}"));
            let main = app.main_window.expect("the demo shows the chats");
            drop(&mut app.store.write().beeper_state);
            assert_eq!(app.store.read().beeper_state.stage(), stage);
            let _ = app.update(UiMessage::StoreChanged);
            let back = stage != "syncing";
            assert_eq!((app.main_window.is_none(), app.login_window == Some(main)), (back, back), "{stage}");
            app
        });
        assert!(app.login.busy && app.login.screen == Screen::Email, "the onboarding checks encrypted messaging");
        let _ = app.update(UiMessage::StoreChanged);
        assert_eq!(app.login.screen, Screen::Email, "the onboarding waits for the check");
        let _ = app.e2ee_checked(Ok(E2eeSetup { cross_signing: true, verified: false, devices: Vec::new() }));
        assert_eq!(app.login.screen, Screen::RecoveryKey);
        app.store.write().beeper_state.verified = true;
        let _ = app.update(LoginMsg::Recovered(Ok(())).into());
        assert!(app.main_window.is_some() && app.login_window.is_none(), "a done stage shows the chats again");
    }

    #[test]
    fn recovery_keys_show_in_groups_and_need_48_characters() {
        let key = "EsTG bqMm dUDy dKth xAyD 45dS HhyC WKc9 CF6B xsFi J5FQ t8u2";
        assert_eq!(format_recovery_key(key), "EsTG-bqMm-dUDy\ndKth-xAyD-45dS\nHhyC-WKc9-CF6B\nxsFi-J5FQ-t8u2");
        assert!(valid_recovery_key(&format_recovery_key(key)), "the dashes of Desktop's format do not count");
        assert!(!valid_recovery_key("EsTG-bqMm"));
    }

    #[test]
    fn a_new_account_saves_and_confirms_its_recovery_key_like_desktop() {
        let mut app = crate::fuzz::demo_app("verify-new-key");
        app.store.write().bridge_accounts.clear();
        app.login_window = Some(window::Id::unique());
        let none = E2eeSetup { cross_signing: false, verified: false, devices: Vec::new() };
        let _ = app.e2ee_checked(Ok(none));
        assert_eq!(app.login.screen, Screen::Preamble, "an account without cross-signing makes a recovery key");
        let shows = |app: &BeeperGuiApp, texts: &[&str]| {
            let mut sim = simulator(app.view_login());
            for text in texts {
                assert!(sim.find(*text).is_ok(), "the screen shows {text}");
            }
        };
        shows(&app, &["Save Key"]);
        // "Reset Recovery Key" deletes the bridges first, and stays on the warning when that fails.
        app.login.screen = Screen::Reset;
        let _ = app.update(LoginMsg::ConfirmReset.into());
        assert!(app.login.busy);
        let _ = app.update(LoginMsg::BridgesDeleted(Err("offline".into())).into());
        assert_eq!(app.login.screen, Screen::Reset);
        assert_eq!(app.login.toast.as_deref(), Some("Failed to reset. Please check your connection and try again."));
        let _ = app.update(LoginMsg::BridgesDeleted(Ok(())).into());
        assert_eq!(app.login.screen, Screen::NewKey, "then it makes a new key");
        let _ = app.update(LoginMsg::NewKey.into());
        assert_eq!(app.login.screen, Screen::NewKey);
        let key = "EsTG bqMm dUDy dKth xAyD 45dS HhyC WKc9 CF6B xsFi J5FQ t8u2";
        app.store.write().pending_recovery_key = Some(key.into());
        app.store.write().beeper_state.verified = true;
        let _ = app.update(LoginMsg::MadeKey(Ok(key.into())).into());
        let _ = app.update(UiMessage::StoreChanged);
        assert_eq!(app.login.screen, Screen::NewKey, "the key stays until it is confirmed");
        shows(&app, &["Copy to Clipboard", "EsTG-bqMm-dUDy\ndKth-xAyD-45dS\nHhyC-WKc9-CF6B\nxsFi-J5FQ-t8u2"]);
        let _ = app.update(LoginMsg::KeyNext.into());
        assert_eq!(app.login.screen, Screen::ConfirmKey);
        let _ = app.update(LoginMsg::Pasted(Some("x".repeat(48))).into());
        let _ = app.update(LoginMsg::SubmitConfirmKey.into());
        shows(&app, &["The Recovery Key you entered doesn't match your Recovery Key."]);
        let _ = app.update(LoginMsg::Pasted(Some(key.into())).into());
        let _ = app.update(LoginMsg::SubmitConfirmKey.into());
        assert_eq!(app.login.screen, Screen::Accounts);
        assert!(app.store.read().beeper_state.has_backed_up_code);

        app.login.screen = Screen::Verify;
        let _ = app.update(LoginMsg::LostKey.into());
        shows(&app, &["Reset your Recovery Key", "Warning", "Reset Recovery Key", "Cancel"]);
    }

    #[test]
    fn recovery_key_input_formats_the_key_and_keeps_the_caret_like_desktop() {
        use text_editor::{Action, Edit};
        let mut input = text_editor::Content::new();
        for c in "EsTGbqMmdUDy-dKth".chars() {
            edit_recovery_key(&mut input, Action::Edit(Edit::Insert(c)));
        }
        assert_eq!(input.text(), "EsTG-bqMm-dUDy\ndKth", "the 13th character starts the second line");
        assert_eq!(input.cursor().position, text_editor::Position { line: 1, column: 4 });
        edit_recovery_key(&mut input, Action::Edit(Edit::Backspace));
        edit_recovery_key(&mut input, Action::Edit(Edit::Backspace));
        assert_eq!(input.text(), "EsTG-bqMm-dUDy\ndK");
        let mut app = crate::fuzz::demo_app("verify-key-focus");
        (app.login.screen, app.store.write().beeper_state.verified) = (Screen::RecoveryKey, false);
        let key = "EsTG bqMm dUDy dKth xAyD 45dS HhyC WKc9 CF6B xsFi J5FQ t8u2";
        let _ = app.update(LoginMsg::KeyOnFocus(Some("not a key".into())).into());
        assert!(!app.login.busy, "a clipboard without a key does nothing");
        let _ = app.update(LoginMsg::KeyOnFocus(Some(key.into())).into());
        assert!(app.login.busy, "a recovery key on the clipboard is submitted when the window gets the focus");
        assert_eq!(app.login.key_input.text(), format_recovery_key(key));
        let _ = app.update(LoginMsg::Recovered(Err("bad key".into())).into());
        let _ = app.update(LoginMsg::KeyOnFocus(Some(key.into())).into());
        assert!(!app.login.busy, "a key that failed is not submitted again");
    }
}
