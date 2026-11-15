//! Desktop's device verification: the toast of a request from another device, the `VerificationModal` window, and the
//! onboarding screens of encrypted messaging after the sign-in.
use crate::emoji::EMOJI_FONT;
use crate::i18n::{tr, trx};
use crate::login::{actions, brand_gradient, heading, LoginMsg, Screen, DANGER, FIELD, HINT, INK, LINK};
use crate::style::{btn_style, fill_box, fw, ic, icon, input_style, semi, shadow_box, t, txt, Palette, CLEAR};
use crate::{BeeperGuiApp, El, Toast, UiMessage};
use beeper_core::matrix::E2eeSetup;
use beeper_core::store::SharedStore;
use iced::font::Weight;
use iced::widget::{
    button, column, container, mouse_area, rich_text, row, span, svg, text_input, tooltip, Column, Space,
};
use iced::{border, window, Background, Center, Color, Fill, Padding, Pixels, Size, Task, Theme};
use std::sync::LazyLock;

/// The size of Desktop's `VERIFICATION` modal window.
const SIZE: Size = Size::new(330.0, 480.0);

#[derive(Default)]
pub struct Verify {
    /// Desktop's `VERIFICATION` modal window.
    pub window: Option<window::Id>,
    /// The ID and the state of the verification that the UI saw last.
    seen: Option<(String, &'static str)>,
}

#[derive(Debug, Clone)]
pub enum VerifyMsg {
    /// Desktop's `startVerification` from Settings.
    Start,
    /// The Accept and Decline buttons of the request toast.
    Accept(String),
    Decline(String),
    /// "They match" and "They don't match".
    Match(bool),
    /// "Confirm using emojis instead" of the QR code.
    Emojis,
    /// A click outside the modal, which cancels the verification.
    Outside,
    /// The end of a Matrix call, with the text of the toast that tells its error.
    Done(&'static str, Result<(), String>),
}

impl From<VerifyMsg> for UiMessage {
    fn from(m: VerifyMsg) -> Self {
        UiMessage::Verify(m)
    }
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
fn call(
    what: &'static str,
    f: impl std::future::Future<Output = Result<(), String>> + Send + 'static,
) -> Task<UiMessage> {
    Task::perform(f, move |r| VerifyMsg::Done(what, r).into())
}

fn cancel(store: SharedStore, id: String) -> Task<UiMessage> {
    call("", async move { beeper_core::matrix::cancel_verification(&store, &id).await })
}

impl BeeperGuiApp {
    pub fn update_verify(&mut self, msg: VerifyMsg) -> Task<UiMessage> {
        let s = self.store.clone();
        match msg {
            VerifyMsg::Start => {
                let start = call("Failed to start verification", async move {
                    beeper_core::matrix::start_verification(&s, "device").await
                });
                return Task::batch([start, self.open_verify()]);
            }
            VerifyMsg::Accept(id) => {
                let accept = call("Failed to start verification", async move {
                    beeper_core::matrix::accept_verification(&s, &id).await
                });
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
            VerifyMsg::Done(_, Ok(())) => {}
            VerifyMsg::Done(what, Err(e)) => self.fail(Toast::new(if what.is_empty() { e } else { tr(what).into() })),
        }
        Task::none()
    }

    fn open_verify(&mut self) -> Task<UiMessage> {
        if self.verify.window.is_some() || self.main_window.is_none() {
            return Task::none();
        }
        let (id, open) = self.open_child(SIZE);
        self.verify.window = Some(id);
        open.chain(window::set_level(id, window::Level::AlwaysOnTop))
    }

    fn close_verify(&mut self) -> Task<UiMessage> {
        self.verify.window.take().map_or_else(Task::none, window::close)
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
        if self.login_window.is_some() {
            self.advance_onboarding();
        }
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
                let buttons = vec![
                    (trx("Accept", "button label"), VerifyMsg::Accept(id.clone()).into()),
                    (trx("Decline", "button label"), VerifyMsg::Decline(id).into()),
                ];
                let text = request_text(&name);
                self.show(Toast { buttons, ..Toast::new(text.clone()) });
                self.sticky_banner = Some(text);
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
        let title = t(title, 28.0, Weight::ExtraBold).line_height(Pixels(38.0)).color(p.strong).center();
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
                let glyph = txt(e.emoji.clone(), 28.0).line_height(1.0).font(EMOJI_FONT);
                let tip = container(txt(e.description.clone(), 12.0).color(p.strong))
                    .padding([4, 8])
                    .style(fill_box(p.elevated, 6.0));
                tooltip(glyph, tip, tooltip::Position::Bottom).into()
            });
            let glyphs = container(row(glyphs).spacing(4)).center_x(Fill).padding([70, 0]);
            let (before, after) = tr("Confirm the emojis above match the ones on %(deviceName)s")
                .split_once("%(deviceName)s")
                .expect("the hint names the device");
            let hint = rich_text![span::<(), _>(before), span(name).font(fw(Weight::Semibold)), span(after)]
                .size(13)
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
        let card = container(card)
            .width(Fill)
            .height(Fill)
            .padding(Padding::new(24.0).top(36.0))
            .style(fill_box(p.primary_container, 8.0));
        let card = mouse_area(card).on_press(UiMessage::Noop);
        mouse_area(container(card).padding(8).style(fill_box(p.window, 0.0))).on_press(VerifyMsg::Outside.into()).into()
    }
}

/// The variants of Desktop's `BrandButton` in `size: "small"`.
#[derive(Clone, Copy)]
pub(crate) enum Brand {
    Primary,
    Secondary,
    Muted,
}

/// Desktop's `BrandButton` with `size: "small"`: a full-width pill.
pub(crate) fn brand_btn<'a>(p: &Palette, label: &'a str, variant: Brand, msg: UiMessage) -> El<'a> {
    brand_btn_maybe(p, label, variant, Some(msg))
}

/// `brand_btn`, which is disabled without a message.
pub(crate) fn brand_btn_maybe<'a>(p: &Palette, label: &'a str, variant: Brand, msg: Option<UiMessage>) -> El<'a> {
    let (bg, fg, edge) = match (msg.is_some(), variant) {
        (false, _) => (Background::from(p.chip), p.on_secondary, CLEAR),
        (true, Brand::Primary) => (brand_gradient(), Color::WHITE, CLEAR),
        (true, Brand::Secondary) => (Background::from(CLEAR), p.accent, p.accent),
        (true, Brand::Muted) => (Background::from(p.chip), p.strong, CLEAR),
    };
    button(container(t(label, 13.0, Weight::Medium).color(fg)).center_x(Fill))
        .width(Fill)
        .padding([8, 10])
        .style(move |_: &Theme, _| button::Style {
            background: Some(bg),
            border: border::rounded(9999).color(edge).width(1),
            ..Default::default()
        })
        .on_press_maybe(msg)
        .into()
}

/// Desktop's `QRCode` of `data`: the bytes in error correction level L, 4 pixels for each module, and a
/// margin of 4 modules, like the `qrcode` package of Desktop.
pub(crate) fn qr_code<'a>(data: &[u8]) -> El<'a> {
    use qrcodegen::{QrCode, QrCodeEcc, QrSegment};
    let code =
        QrCode::encode_segments(&[QrSegment::make_bytes(data)], QrCodeEcc::Low).expect("a QR code holds the data");
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
        self.login.busy = true;
        Task::perform(beeper_core::matrix::e2ee_setup(self.store.clone()), |r| LoginMsg::E2ee(r).into())
    }

    /// Desktop's routing of `pre-verification`: verify with another device when one can do it, else with the
    /// recovery key.
    pub(crate) fn e2ee_checked(&mut self, r: Result<E2eeSetup, String>) -> Task<UiMessage> {
        let l = &mut self.login;
        l.busy = false;
        match r {
            Ok(s) if !s.cross_signing || s.verified => {
                self.advance_onboarding();
                return Task::none();
            }
            Ok(s) if s.devices.is_empty() => l.screen = Screen::RecoveryKey,
            Ok(s) => (l.screen, l.devices) = (Screen::Verify, s.devices),
            Err(e) => (l.screen, l.error) = (Screen::Verify, Some(e)),
        }
        match l.screen {
            Screen::Verify => call("Failed to start verification", {
                let s = self.store.clone();
                async move { beeper_core::matrix::start_verification(&s, "login").await }
            }),
            _ => Task::none(),
        }
    }

    /// "Enter Recovery Key": cancels the verification and asks for the recovery key.
    pub(crate) fn use_recovery_key(&mut self) -> Task<UiMessage> {
        (self.login.screen, self.login.waiting, self.login.error) = (Screen::RecoveryKey, false, None);
        let id = self
            .store
            .read()
            .app_verification()
            .filter(|v| !matches!(v.state, "done" | "cancelled"))
            .map(|v| v.id.clone());
        id.map_or_else(Task::none, |id| cancel(self.store.clone(), id))
    }

    /// Desktop's `advanceOnboarding`: the next screen once this device is verified.
    pub(crate) fn advance_onboarding(&mut self) {
        let (verified, syncing) = {
            let st = self.store.read();
            (st.beeper_state.verified, st.syncing)
        };
        let screen = self.login.screen;
        let checking = self.login.signing_in() || matches!(screen, Screen::Verify | Screen::RecoveryKey);
        self.login.screen = match (checking, screen, verified, syncing) {
            (true, Screen::Verify | Screen::RecoveryKey, false, _) => return,
            (true, _, _, true) => Screen::Syncing,
            (true, _, _, false) | (false, Screen::Syncing, _, false) => Screen::Accounts,
            _ => return,
        };
    }

    /// Desktop's `AwaitVerification`: the request to the other devices, then their QR code or their emojis.
    pub(crate) fn view_onboarding_verify(&self) -> El<'_> {
        let l = &self.login;
        let p = &crate::style::BEEPER_LIGHT;
        let st = self.store.read();
        let v = st.app_verification().filter(|v| v.purpose == "login");
        let emojis = v.and_then(|v| v.sas.clone().filter(|_| v.state == "sas_ready"));
        let qr = v.and_then(|v| v.qr.clone().filter(|_| v.state == "ready"));
        let can_emojis = v.is_some_and(|v| v.methods.contains(&"sas") && v.state == "ready");
        drop(st);
        let secondary = |label, msg: UiMessage| brand_btn(p, label, Brand::Secondary, msg);
        let use_key = || secondary(trx("Enter Recovery Key", "button label"), LoginMsg::UseRecoveryKey.into());
        let emojis_btn =
            || can_emojis.then(|| secondary(trx("Confirm using emojis", "button label"), VerifyMsg::Emojis.into()));
        let note = |s: &'static str| txt(tr(s), 13.0).color(INK).center().width(Fill);
        let (middle, bottom): (El<'_>, Vec<El<'_>>) = match (l.waiting, emojis, qr) {
            (true, _, _) => {
                (Space::new().into(), vec![note("Waiting for you to verify on your other device...").into()])
            }
            (false, Some(sas), _) => {
                let glyphs = sas.emojis.into_iter().map(|e| {
                    let glyph = txt(e.emoji, 28.0).line_height(1.0).font(EMOJI_FONT);
                    let tip = container(txt(e.description, 12.0).color(INK))
                        .padding([4, 8])
                        .style(fill_box(Color::WHITE, 6.0));
                    tooltip(glyph, tip, tooltip::Position::Bottom).into()
                });
                let buttons = vec![
                    container(note("Confirm the emojis match the ones shown on your other device."))
                        .padding(Padding::ZERO.bottom(8))
                        .into(),
                    brand_btn(p, trx("They match", "button label"), Brand::Primary, VerifyMsg::Match(true).into()),
                    secondary(trx("They don't match", "button label"), VerifyMsg::Match(false).into()),
                    use_key(),
                ];
                (container(row(glyphs).spacing(4)).center(Fill).into(), buttons)
            }
            (false, None, Some(qr)) => {
                let plate = container(qr_code(&qr.data)).padding(11).style(|_: &Theme| container::Style {
                    background: Some(brand_gradient()),
                    border: border::rounded(12),
                    ..Default::default()
                });
                let mut buttons: Vec<El<'_>> = vec![
                    note("Scan this QR Code using Beeper iOS or Android app.").into(),
                    note(trx("Or", "verification method divider")).into(),
                ];
                buttons.extend(emojis_btn());
                buttons.push(use_key());
                (container(plate).center(Fill).into(), buttons)
            }
            (false, None, None) => {
                let devices = self.login.devices.iter().take(5).map(|d| -> El<'_> {
                    let seen = d.last_seen_ms.map(|t| {
                        txt(tr("seen %(lastSeen)s").replace("%(lastSeen)s", &crate::ago(t)), 11.0).color(HINT)
                    });
                    column![txt(d.name.clone(), 13.0).color(INK)].push(seen).into()
                });
                let devices: El<'_> = match self.login.devices.is_empty() {
                    true => txt(tr("No devices found"), 13.0).color(HINT).into(),
                    false => Column::with_children(devices).spacing(6).into(),
                };
                let tip = container(devices).padding([8, 12]).style(shadow_box(Color::WHITE, FIELD, 8.0, 10.0));
                let another = row![semi(tr("another device"), 17.0).color(INK), ic(icon::INFO, 16.0).color(HINT)]
                    .spacing(4)
                    .align_y(Center);
                let title = column![
                    semi("Open Beeper", 17.0).color(INK),
                    row![
                        semi("on ", 17.0).color(INK),
                        tooltip(another, tip, tooltip::Position::Top),
                        semi(" and", 17.0).color(INK)
                    ]
                    .align_y(Center),
                    semi("follow instructions to verify", 17.0).color(INK),
                ]
                .align_x(Center)
                .width(Fill);
                let link = |s: &'static str, m: UiMessage| -> El<'_> {
                    button(txt(s, 13.0).color(LINK))
                        .padding(0)
                        .style(btn_style([CLEAR; 3], [LINK; 3], 0.0))
                        .on_press(m)
                        .into()
                };
                let mut buttons = vec![title.into()];
                match l.another_way {
                    false => buttons.push(
                        container(link(trx("Try another way", "button label"), LoginMsg::AnotherWay.into()))
                            .center_x(Fill)
                            .into(),
                    ),
                    true => {
                        buttons.push(note(trx("or", "verification method divider")).into());
                        buttons.extend(emojis_btn());
                        buttons.push(use_key());
                    }
                }
                (container(svg(VERIFY_ART.clone()).width(241).height(224)).center(Fill).into(), buttons)
            }
        };
        let error = l.error.as_deref().map(|e| txt(e, 12.0).color(DANGER).center().width(Fill));
        column![Space::new().height(34), heading(trx("Verify", "onboarding verification heading"), 36.0), middle]
            .push(error)
            .push(actions(bottom, 24.0))
            .into()
    }

    /// Desktop's `AwaitRecoveryCode`.
    pub(crate) fn view_onboarding_key(&self) -> El<'_> {
        let l = &self.login;
        let p = &crate::style::BEEPER_LIGHT;
        let input = text_input("", &l.recovery_key)
            .on_input(|v| LoginMsg::RecoveryKey(v).into())
            .on_submit(LoginMsg::SubmitRecoveryKey.into())
            .size(14)
            .padding([10, 12])
            .style(input_style(FIELD, INK, HINT, LINK, 12.0));
        let next = (!l.recovery_key.is_empty() && !l.busy).then(|| LoginMsg::SubmitRecoveryKey.into());
        let next = brand_btn_maybe(p, trx("Next", "button label"), Brand::Primary, next);
        let error = l.error.as_deref().map(|e| txt(e, 12.0).color(DANGER).center().width(Fill));
        column![
            Space::new().height(40),
            heading(tr("Enter your Recovery Key"), 28.0),
            container(input).padding([24, 18]),
            Space::new().height(Fill),
        ]
        .push(error)
        .push(actions(vec![next], 24.0))
        .into()
    }

    /// Desktop's `IndicateSyncing`, while the first sync of the chats runs.
    pub(crate) fn view_onboarding_syncing(&self) -> El<'_> {
        column![
            Space::new().height(34),
            heading(tr("Syncing your data"), 28.0),
            container(txt(tr("We're syncing your chats. This may take a while..."), 13.0).color(INK).center())
                .center(Fill)
                .padding(18),
        ]
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use beeper_core::models::{
        AppVerification, VerificationDevice, VerificationQr, VerificationSas, VerificationSasEmoji,
    };
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
    fn onboarding_verifies_with_another_device_or_the_recovery_key_like_desktop() {
        use beeper_core::matrix::DeviceInfo;
        let mut app = crate::fuzz::demo_app("verify-onboarding");
        app.login_window = Some(window::Id::unique());
        app.store.write().beeper_state.verified = false;
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
        shows(&app, &["Verify", "Open Beeper", "another device", "follow instructions to verify", "Try another way"]);
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

        app.store.write().beeper_state.verified = true;
        let _ = app.update(UiMessage::StoreChanged);
        assert_eq!(app.login.screen, Screen::Accounts, "a verified device goes on to the accounts");

        app.login.screen = Screen::Code;
        app.store.write().beeper_state.verified = false;
        let _ = app.e2ee_checked(Ok(setup(Vec::new())));
        assert_eq!(app.login.screen, Screen::RecoveryKey, "without another device the recovery key verifies");
        shows(&app, &["Enter your Recovery Key", "Next"]);
        let _ = app.update(LoginMsg::Recovered(Err("bad key".into())).into());
        shows(&app, &["Error: bad key"]);
    }
}
