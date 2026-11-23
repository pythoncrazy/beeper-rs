//! Desktop's account nicknames: the "Edit Account Nickname…" item of the account menus (`Zpe`) and the Account
//! Nickname window (`sft` with `RenameAccount`), which names an account in Beeper on all devices.

use super::*;
use crate::focus::{Keys, Look};
use iced::widget::column;

/// The size of Desktop's Account Nickname window.
pub(crate) const NICKNAME_SIZE: Size = Size::new(340.0, 380.0);
/// The name input of the Account Nickname window.
const NICKNAME_INPUT: &str = "account-nickname";
/// Desktop's `PillInput` is at most this wide.
const PILL_MAX: f32 = 220.0;
/// How long Desktop shows the error toast of a failed save.
const ERROR_TOAST: std::time::Duration = std::time::Duration::from_secs(3);

static EDIT: std::sync::LazyLock<svg::Handle> = std::sync::LazyLock::new(|| {
    svg::Handle::from_memory(
        br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20"><path fill="#000" d="M4.167 15.834h1.166l7.188-7.188-1.167-1.167-7.187 7.188zm11.916-8.396-3.541-3.5 1.166-1.167a1.6 1.6 0 0 1 1.177-.479 1.6 1.6 0 0 1 1.178.48l1.166 1.166q.48.48.5 1.156.02.677-.458 1.156zm-1.208 1.229L6.042 17.5H2.5V13.96l8.833-8.834z"/></svg>"##
            .as_slice(),
    )
});

#[derive(Clone)]
pub enum Msg {
    /// Desktop's `Zpe`: the Account Nickname window of an account.
    Open(String),
    Name(window::Id, String),
    Cancel(window::Id),
    /// The submit of the form, by Save or Enter.
    Save(window::Id),
    Saved(window::Id, Result<(), String>),
    /// The error toast of the window ends.
    HideError(window::Id, String),
}

/// An Account Nickname window: the account, the typed name, a save on its way, and the error toast.
pub struct NicknameDraft {
    pub id: window::Id,
    pub account: String,
    pub name: String,
    pub saving: bool,
    pub error: Option<String>,
}

/// Desktop's `Zpe` in the account menu `rj`: an account with a bridge login, other than the Beeper account.
pub(crate) fn can_nickname(st: &BeeperStore, account: &str) -> bool {
    account != "hungryserv" && st.bridge_accounts.iter().any(|b| b.account_id == account)
}

impl BeeperGuiApp {
    pub(crate) fn nickname(&mut self, msg: Msg) -> Task<UiMessage> {
        match msg {
            Msg::Open(account) => {
                self.list.chat_menu = None;
                if let Some(d) = self.nickname_windows.iter().find(|d| d.account == account) {
                    return self.raise(d.id);
                }
                let (id, open) = self.open_child(NICKNAME_SIZE);
                let name = self.store.read().custom_name(&account).unwrap_or_default().to_string();
                self.nickname_windows.push(NicknameDraft { id, account, name, saving: false, error: None });
                return open.chain(operation::focus(NICKNAME_INPUT)).chain(operation::select_all(NICKNAME_INPUT));
            }
            Msg::Name(id, name) => {
                if let Some(d) = self.nickname_windows.iter_mut().find(|d| d.id == id) {
                    d.name = name;
                }
            }
            Msg::Cancel(id) => return self.close_window(id),
            Msg::Save(id) => return self.save_nickname(id),
            Msg::Saved(id, Ok(())) => return self.close_window(id),
            Msg::Saved(id, Err(e)) => {
                let Some(d) = self.nickname_windows.iter_mut().find(|d| d.id == id) else { return Task::none() };
                let text = trf("Something went wrong: %s", &[&e]);
                (d.saving, d.error) = (false, Some(text.clone()));
                let wait = async { tokio::time::sleep(ERROR_TOAST).await };
                return Task::perform(wait, move |_| UiMessage::Nickname(Msg::HideError(id, text.clone())));
            }
            Msg::HideError(id, text) => {
                if let Some(d) = self.nickname_windows.iter_mut().find(|d| d.id == id) {
                    d.error = d.error.take().filter(|e| *e != text);
                }
            }
        }
        Task::none()
    }

    /// Desktop's `onSubmit` of `RenameAccount`: an unchanged name closes the window. Otherwise the window waits for
    /// `setBridgeAccountName` and closes, or shows the error. The demo keeps the name on this device.
    fn save_nickname(&mut self, id: window::Id) -> Task<UiMessage> {
        let Some(d) = self.nickname_windows.iter_mut().find(|d| d.id == id) else { return Task::none() };
        {
            let mut st = self.store.write();
            let Some(b) = st.bridge_accounts.iter().find(|b| b.account_id == d.account) else { return Task::none() };
            let (bridge, login) = (b.bridge_id.clone(), b.login_id.clone());
            if d.name.trim() == st.custom_name(&d.account).unwrap_or_default() {
                return self.close_window(id);
            }
            if !st.config.demo {
                d.saving = true;
                // Chromium takes the focus from a button that turns disabled.
                self.key_targets.remove(&id);
                let store = self.store.clone();
                let save = beeper_core::matrix::set_bridge_account_name(store, bridge, login, d.name.clone());
                return Task::perform(save, move |r| UiMessage::Nickname(Msg::Saved(id, r)));
            }
            let names = beeper_core::matrix::with_account_name(&st.account_overrides, &bridge, &login, d.name.trim());
            st.account_overrides = names.expect("the demo overrides are valid");
        }
        self.close_window(id)
    }

    /// Desktop's `RenameAccount`: the account's icon, the name pill with the default name as its placeholder, the
    /// user of the account, a hint, and Cancel and Save.
    pub(crate) fn view_nickname<'a>(&'a self, d: &'a NicknameDraft) -> El<'a> {
        let p = &self.pal;
        let st = self.store.read();
        let title = container(semi(tr("Account Nickname"), 14.0).line_height(lh(18.0)).color(p.strong))
            .center_x(Fill)
            .padding([8, 0]);
        let found = st.bridge_accounts.iter().find(|b| b.account_id == d.account);
        let account: El<'a> = match found {
            Some(b) => {
                let icon = svg(brand_svg(brand_key(&d.account))).width(64).height(64);
                let placeholder = new_chat::default_account_display_name(&st, &d.account, &rail_accounts(&st));
                let shown = if d.name.is_empty() { &placeholder } else { &d.name };
                let width = text_width(shown, 14.0, Weight::Semibold).max(text_width("0", 14.0, Weight::Semibold));
                let input = text_input(&placeholder, &d.name)
                    .id(NICKNAME_INPUT)
                    .on_input_maybe((!d.saving).then_some(move |t| UiMessage::Nickname(Msg::Name(d.id, t))))
                    .on_submit(UiMessage::Nickname(Msg::Save(d.id)))
                    .font(fw(Weight::Semibold))
                    .size(rem(14.0))
                    .line_height(lh(18.0))
                    .align_x(Center)
                    // Desktop's `field-sizing: content`, within the pill's 14 px and 26 px of padding.
                    .width((width + 1.0).min(PILL_MAX - 40.0))
                    .padding(0)
                    .style(input_style(CLEAR, p.strong, p.muted, p.accent, 0.0));
                let pill = row![input, tinted(&EDIT, 14.0, p.muted)].spacing(4).align_y(Center);
                let pill = container(pill).padding(Padding::new(6.0).left(14.0).right(8.0)).style(bordered_box(
                    p.card,
                    p.menu_border,
                    0.5,
                    999.0,
                ));
                let r = b.remote_profile.clone().unwrap_or_default();
                let full = r.name.clone().or_else(|| b.remote_name.clone());
                let id = r.phone.as_deref().map(fmt_phone).or_else(|| r.username.as_ref().map(|u| format!("@{u}")));
                let id = id.or(r.email.clone()).filter(|i| Some(i) != full.as_ref());
                let face = self.face(r.avatar.as_deref(), full.as_deref().unwrap_or_default(), 24.0, 0.0);
                let names = column![]
                    .push(full.map(|n| semi(n, 11.0).line_height(lh(14.0)).color(p.strong)))
                    .push(id.map(|i| txt(i, 10.0).line_height(lh(12.0)).color(p.sub)));
                let identity = container(row![face, names].spacing(6).align_y(Center))
                    .padding(Padding::new(4.0).right(10.0))
                    .style(bordered_box(CLEAR, p.border_secondary, 1.0, 999.0));
                column![icon, pill, Space::new().height(16), identity].align_x(Center).into()
            }
            None => txt(tr("Account not found"), 13.0).color(p.strong).into(),
        };
        let hint = found.map(|_| {
            txt(tr("Changes how this account appears in Beeper, on all your devices."), 10.0)
                .line_height(lh(12.0))
                .color(p.sub)
                .center()
                .width(Fill)
        });
        let card = column![container(account).center(Fill)].push(hint);
        let card = container(card).padding(12).height(Fill).style(fill_box(p.card, 14.0));
        let save = (!d.saving).then_some(UiMessage::Nickname(Msg::Save(d.id)));
        let cancel = (!d.saving).then_some(UiMessage::Nickname(Msg::Cancel(d.id)));
        // Desktop's `Button`s, which a save disables, and which take the focus while they work.
        let button = |key: &str, b: El<'a>, msg: Option<UiMessage>| match msg {
            Some(m) => self.target(Some(d.id), key, b, Keys::press(m), Look::button(p, 9999.0)),
            None => b,
        };
        let footer = row![
            button("cancel", main_button(p, trx("Cancel", "button label"), false, cancel.clone(), false), cancel),
            Space::new().width(Fill),
            button("save", main_button(p, trx("Save", "button label"), true, save.clone(), d.saving), save),
        ]
        .padding(Padding::new(16.0).right(12.0).bottom(10.0).left(12.0));
        // Desktop's toasts of this window (`k3` with `matchWindowName`).
        let error = d.error.as_ref().map(|e| {
            let dismiss = Some(UiMessage::Nickname(Msg::HideError(d.id, e.clone())));
            let toast = self.toast_card(&Toast { error: true, dismiss, ..Toast::new(e.clone()) }, None);
            container(toast).width(Fill).align_bottom(Fill).padding(8)
        });
        let body = container(column![title, card, footer]).padding(8).style(fill_box(p.window, 0.0));
        stack![body].push(error).into()
    }
}

/// Desktop's `Button` in its `md` size with `rounded`: the `primary` fill or the `secondary` tint, disabled at half
/// opacity, and a spinner over the hidden label while `loading`.
pub(crate) fn main_button<'a>(
    p: &Palette,
    label: &'a str,
    primary: bool,
    msg: Option<UiMessage>,
    loading: bool,
) -> El<'a> {
    let (bg, fg) = if primary { (p.main, Color::WHITE) } else { (p.main_muted, p.main_text) };
    let alpha = if msg.is_some() { 1.0 } else { 0.5 };
    let label = txt(label, 13.0).line_height(1.0).color(if loading { CLEAR } else { fg.scale_alpha(alpha) });
    let spin = loading.then(|| container(verify::spinner(16.0, fg.scale_alpha(alpha))).center(Fill));
    button(stack![container(label).center_y(Fill)].push(spin))
        .height(28)
        .padding([0, 12])
        .style(move |_: &Theme, _| button::Style {
            background: Some(bg.scale_alpha(alpha).into()),
            border: border::rounded(9999),
            ..Default::default()
        })
        .on_press_maybe(msg)
        .into()
}
