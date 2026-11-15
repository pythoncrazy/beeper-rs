//! Desktop's `ENTER_RECOVERY_CODE` modal: verifies this device with the recovery key from the main window.
use crate::i18n::{tr, trx};
use crate::login::{actions, heading};
use crate::style::{fill_box, txt};
use crate::verify::{brand_btn_maybe, clipboard_key, edit_recovery_key, format_recovery_key, key_input, Brand};
use crate::{BeeperGuiApp, El, UiMessage};
use iced::keyboard::{key::Named, Key};
use iced::widget::{column, container, text_editor, Space};
use iced::{window, Fill, Size, Task};

/// The size of Desktop's modal windows.
const SIZE: Size = Size::new(500.0, 450.0);

#[derive(Default)]
pub struct EnterKey {
    pub window: Option<window::Id>,
    input: text_editor::Content,
    pub busy: bool,
    error: Option<String>,
    /// The keys that were tried, which a paste on focus does not submit again.
    tried: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum KeyMsg {
    Open,
    Edit(text_editor::Action),
    Paste,
    Pasted(Option<String>),
    /// The clipboard when the window gets the focus, like Desktop's `useAutoSubmitRecoveryCode`.
    OnFocus(Option<String>),
    Submit,
    Done(Result<(), String>),
}

impl From<KeyMsg> for UiMessage {
    fn from(m: KeyMsg) -> Self {
        UiMessage::EnterKey(m)
    }
}

impl BeeperGuiApp {
    pub(crate) fn update_enter_key(&mut self, msg: KeyMsg) -> Task<UiMessage> {
        let k = &mut self.enter_key;
        match msg {
            KeyMsg::Open if k.window.is_some() => return k.window.map_or_else(Task::none, window::gain_focus),
            KeyMsg::Open => {
                let (id, open) = self.open_child(SIZE);
                self.enter_key = EnterKey { window: Some(id), ..EnterKey::default() };
                return open;
            }
            KeyMsg::Edit(action) => {
                edit_recovery_key(&mut k.input, action);
                k.error = None;
            }
            KeyMsg::Paste => return iced::clipboard::read().map(|t| KeyMsg::Pasted(t).into()),
            KeyMsg::Pasted(text) => {
                k.input = text_editor::Content::with_text(&format_recovery_key(&text.unwrap_or_default()));
                k.error = None;
            }
            KeyMsg::OnFocus(text) => {
                if let Some(key) = clipboard_key(text, &k.tried).filter(|_| !k.busy) {
                    k.input = text_editor::Content::with_text(&format_recovery_key(&key));
                    return self.update_enter_key(KeyMsg::Submit);
                }
            }
            KeyMsg::Submit if k.busy => {}
            KeyMsg::Submit => {
                (k.busy, k.error) = (true, None);
                let key = beeper_core::matrix::recovery_key_chars(&k.input.text());
                k.tried.push(key.clone());
                return Task::perform(beeper_core::matrix::recover(self.store.clone(), key), |r| {
                    KeyMsg::Done(r).into()
                });
            }
            KeyMsg::Done(Ok(())) => return k.window.take().map_or_else(Task::none, window::close),
            KeyMsg::Done(Err(e)) => {
                (k.busy, k.error) = (false, Some(tr("Error: %(message)s").replace("%(message)s", &e)))
            }
        }
        Task::none()
    }

    /// The keys of the modal: Esc closes it.
    pub(crate) fn enter_key_key(&mut self, key: Key) -> Task<UiMessage> {
        match key {
            Key::Named(Named::Escape) => self.enter_key.window.map_or_else(Task::none, window::close),
            _ => Task::none(),
        }
    }

    /// The title of the modal and its window: "Verifying..." while the key is checked.
    pub(crate) fn enter_key_title(&self) -> &'static str {
        match self.enter_key.busy {
            true => tr("Verifying..."),
            false => tr("Enter your Recovery Key"),
        }
    }

    /// Desktop's `EnterRecoveryCode`: the title, the `RecoveryCodeInput`, "Next", and the error.
    pub fn view_enter_key(&self) -> El<'_> {
        let k = &self.enter_key;
        let p = &self.pal;
        let next = (!k.input.text().trim().is_empty()).then(|| KeyMsg::Submit.into());
        let next = brand_btn_maybe(p, trx("Next", "button label"), Brand::Primary, next, k.busy);
        let error = k.error.as_deref().map(|e| txt(e, 12.0).color(p.text_danger).center().width(Fill));
        let input = key_input(p, &k.input, k.error.is_some(), |a| KeyMsg::Edit(a).into(), KeyMsg::Paste.into());
        let body = column![Space::new().height(40), heading(self.enter_key_title(), 28.0), input]
            .push(Space::new().height(Fill))
            .push(error)
            .push(actions(vec![next], 24.0));
        container(body).width(Fill).height(Fill).style(fill_box(p.primary_container, 0.0)).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced_test::simulator::simulator;

    #[test]
    fn the_modal_verifies_with_the_recovery_key_like_desktop() {
        let mut app = crate::fuzz::demo_app("enter-key");
        let _ = app.update(KeyMsg::Open.into());
        assert!(app.enter_key.window.is_some(), "the modal opens");
        {
            let mut sim = simulator(app.view_enter_key());
            assert!(sim.find("Paste from Clipboard").is_ok() && sim.find("Next").is_ok());
        }
        let key = "EsTG bqMm dUDy dKth xAyD 45dS HhyC WKc9 CF6B xsFi J5FQ t8u2";
        let _ = app.update(KeyMsg::OnFocus(Some("not a key".into())).into());
        assert!(!app.enter_key.busy, "only a recovery key on the clipboard is submitted");
        let _ = app.update(KeyMsg::OnFocus(Some(key.into())).into());
        assert!(app.enter_key.busy, "a copied key is submitted on focus");
        assert_eq!(app.enter_key_title(), "Verifying...");
        let _ = app.update(KeyMsg::Done(Err("Invalid recovery key".into())).into());
        assert!(simulator(app.view_enter_key()).find("Error: Invalid recovery key").is_ok());
        let _ = app.update(KeyMsg::OnFocus(Some(key.into())).into());
        assert!(!app.enter_key.busy, "a key that failed is not submitted again");
        let _ = app.update(KeyMsg::Done(Ok(())).into());
        assert!(app.enter_key.window.is_none(), "a verified device closes the modal");
    }
}
