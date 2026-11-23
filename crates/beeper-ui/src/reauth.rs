//! Desktop's `ReauthRequiredModal`: a sign-in code from the email of the account gives a new Beeper JWT, which
//! answers the user-interactive auth of the homeserver (`updateAuthToken`).
use crate::i18n::{tr, trx};
use crate::login::{actions, field, heading};
use crate::style::{fill_box, txt};
use crate::verify::{brand_btn_maybe, Brand};
use crate::{BeeperGuiApp, El, UiMessage};
use beeper_core::cloud::{Cloud, CodeResult};
use iced::keyboard::{key::Named, Key};
use iced::widget::{column, container, operation, Space};
use iced::{window, Fill, Padding, Size, Task};
use std::sync::Arc;

/// The size of Desktop's modal windows.
const SIZE: Size = Size::new(500.0, 450.0);
const CODE_INPUT: &str = "reauth-code";

#[derive(Default)]
pub struct Reauth {
    pub window: Option<window::Id>,
    /// The sign-in request of the Beeper cloud, which emailed the code.
    request: Option<String>,
    code: String,
    pub busy: bool,
    error: Option<String>,
    /// The step that needed the auth, which runs again with the new Beeper JWT.
    retry: Option<UiMessage>,
}

#[derive(Debug, Clone)]
pub enum ReauthMsg {
    /// The sign-in request after the Beeper cloud emailed the code.
    Sent(Result<String, &'static str>),
    Code(String),
    Submit,
    /// The new Beeper JWT.
    Done(Result<String, String>),
}

impl From<ReauthMsg> for UiMessage {
    fn from(m: ReauthMsg) -> Self {
        UiMessage::Reauth(m)
    }
}

/// Runs a blocking call of the Beeper cloud on a worker thread.
async fn cloud<T: Send + 'static>(cloud: Arc<Cloud>, f: impl FnOnce(&Cloud) -> T + Send + 'static) -> T {
    tokio::task::spawn_blocking(move || f(&cloud)).await.expect("Beeper cloud worker thread")
}

impl BeeperGuiApp {
    /// Opens Desktop's `ReauthRequiredModal`, and asks the Beeper cloud to email a sign-in code. The code runs `retry`.
    pub(crate) fn open_reauth(&mut self, retry: UiMessage) -> Task<UiMessage> {
        if let Some(id) = self.reauth.window {
            return self.raise(id);
        }
        let (id, open) = self.open_child(SIZE);
        self.reauth = Reauth { window: Some(id), retry: Some(retry), ..Reauth::default() };
        let open = open.chain(operation::focus(CODE_INPUT));
        // Like Desktop, an account without an email gets no code.
        let Some(email) = self.store.read().whoami.email.clone() else { return open };
        let send = cloud(self.cloud.clone(), move |c| {
            let session = "Couldn't create authentication session. Please try again or contact support.";
            let (request, _) = c.start_login().map_err(|_| session)?;
            let failed = "Something went wrong requesting an email code, try again or contact support.";
            c.send_login_email(&request, &email).map_err(|_| failed)?;
            Ok(request)
        });
        Task::batch([open, Task::perform(send, |r| ReauthMsg::Sent(r).into())])
    }

    pub(crate) fn update_reauth(&mut self, msg: ReauthMsg) -> Task<UiMessage> {
        let r = &mut self.reauth;
        match msg {
            ReauthMsg::Sent(sent) => match sent {
                Ok(request) => r.request = Some(request),
                Err(e) => r.error = Some(tr(e).into()),
            },
            ReauthMsg::Code(code) => (r.code, r.error) = (code, None),
            ReauthMsg::Submit if r.busy => {}
            ReauthMsg::Submit => {
                let Some(request) = r.request.clone() else {
                    r.error = Some(tr("Could not find session. Please close this modal and try again.").into());
                    return Task::none();
                };
                (r.busy, r.error) = (true, None);
                let code = r.code.clone();
                let check = cloud(self.cloud.clone(), move |c| match c.send_login_code(&request, &code) {
                    Ok(CodeResult::Token(jwt)) => Ok(jwt),
                    Ok(CodeResult::Registration { .. }) => {
                        Err(tr("Something went wrong. Please try again or contact support.").into())
                    }
                    Err(e) => Err(e.message),
                });
                return Task::perform(check, |r| ReauthMsg::Done(r).into());
            }
            ReauthMsg::Done(Err(e)) => (r.busy, r.error) = (false, Some(e)),
            // Desktop's `updateAuthToken`, then the step that needed the auth again.
            ReauthMsg::Done(Ok(jwt)) => {
                self.store.write().ui_auth_token = Some(jwt);
                let close = self.reauth.window.take().map_or_else(Task::none, window::close);
                let retry = self.reauth.retry.take().expect("the modal has a step to run again");
                return Task::batch([close, self.update(retry)]);
            }
        }
        Task::none()
    }

    /// The keys of the modal: Esc closes it, and Enter submits the code.
    pub(crate) fn reauth_key(&mut self, key: Key) -> Task<UiMessage> {
        match key {
            Key::Named(Named::Escape) => self.reauth.window.map_or_else(Task::none, window::close),
            Key::Named(Named::Enter) => self.update_reauth(ReauthMsg::Submit),
            _ => Task::none(),
        }
    }

    /// Desktop's `ReauthRequiredModal`: "Check your email", the code field and "Next".
    pub fn view_reauth(&self) -> El<'_> {
        let r = &self.reauth;
        let p = &self.pal;
        let email = self.store.read().whoami.email.clone().unwrap_or_default();
        let hint = tr("This action requires re-authentication. Please check %(email)s for your sign in code.");
        let hint = txt(hint.replace("%(email)s", &email), 13.0).color(p.strong).center().width(Fill);
        let input = field(
            p,
            CODE_INPUT,
            trx("6-digit code", "input placeholder"),
            &r.code,
            (!r.busy).then_some(ReauthMsg::Code),
        );
        let next =
            brand_btn_maybe(p, trx("Next", "button label"), Brand::Primary, Some(ReauthMsg::Submit.into()), r.busy);
        let error = r.error.as_deref().map(|e| txt(e, 12.0).color(p.text_danger).center().width(Fill).into());
        let body = column![Space::new().height(40), heading(tr("Check your email"), 36.0)]
            .push(container(hint).padding(Padding::new(16.0).left(24.0).right(24.0)))
            .push(Space::new().height(Fill))
            .push(actions([input, next].into_iter().chain(error).collect(), 24.0));
        container(body).width(Fill).height(Fill).style(fill_box(p.primary_container, 0.0)).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::login::{LoginMsg, Screen};
    use beeper_core::matrix::REAUTH;
    use iced_test::simulator::simulator;

    #[test]
    fn a_refused_auth_asks_for_an_email_code_then_makes_the_key_again_like_desktop() {
        let mut app = crate::fuzz::demo_app("reauth");
        app.login.screen = Screen::NewKey;
        let _ = app.update(LoginMsg::MadeKey(Err(REAUTH.into())).into());
        assert!(app.reauth.window.is_some(), "the modal opens");
        assert_eq!(app.login.error.as_deref(), Some(REAUTH), "the error stays when the modal closes");
        let shows = |app: &BeeperGuiApp, texts: &[&str]| {
            let mut sim = simulator(app.view_reauth());
            for text in texts {
                assert!(sim.find(*text).is_ok(), "the modal shows {text}");
            }
        };
        shows(&app, &["6-digit code", "Next"]);
        let _ = app.update(ReauthMsg::Submit.into());
        shows(&app, &["Could not find session. Please close this modal and try again."]);
        let _ = app.update(ReauthMsg::Sent(Ok("request".into())).into());
        let _ = app.update(ReauthMsg::Code("123456".into()).into());
        let _ = app.update(ReauthMsg::Submit.into());
        assert!(app.reauth.busy, "the code is checked");
        let _ = app.update(ReauthMsg::Done(Err("Invalid code. Please try again.".into())).into());
        shows(&app, &["Invalid code. Please try again."]);
        let _ = app.update(ReauthMsg::Done(Ok("new.jwt".into())).into());
        assert!(app.reauth.window.is_none(), "the modal closes");
        assert_eq!(app.store.read().ui_auth_token.as_deref(), Some("new.jwt"), "Desktop's updateAuthToken");
        assert!(app.login.busy && app.login.error.is_none(), "the key is made again");
    }
}
