//! Desktop's Beeper Profile page of Settings (`ProfileSubView`), with its menu and prompts.

use crate::i18n::{tr, trf, trx};
use crate::settings;
use crate::style::{
    bold, bordered_box, btn_style, fill_box, fw, ic, icon, med, semi, shadow_box, txt, Bare, Palette, CLEAR,
};
use crate::{
    menu_item, menu_rule, outline_input, pill_style, plan_pill, portal, round_avatar, tinted, BeeperGuiApp, El,
    PendingMedia, SettingsTab, Toast, UiMessage, CHEVRON_DOWN,
};
use beeper_core::cloud::{Cloud, CloudError};
use beeper_core::models::{Plan, User};
use beeper_core::store::BeeperStore;
use iced::font::Weight;
use iced::widget::{
    button, column, container, hover, image, mouse_area, rich_text, row, span, text_input, Column, Space,
};
use iced::{Center, Color, Fill, Padding, Task, Theme};
use std::path::PathBuf;

/// The edits and the open prompts of the Beeper Profile page.
#[derive(Default)]
pub struct ProfileForm {
    /// The edited name and email. `None` shows the value of the profile.
    pub name: Option<String>,
    pub email: Option<String>,
    /// The picked avatar: the image, its MIME type and the round preview.
    avatar: Option<(Vec<u8>, String, image::Handle)>,
    saving: bool,
    pub menu: bool,
    /// Desktop's `DeleteAccountPrompt`: the typed username, whether the deletion runs, and its error.
    pub delete: Option<(String, bool, Option<String>)>,
    /// Desktop's `ChangeEmailModal`: the current, new and confirmed email, whether it saves, and its error.
    pub change_email: Option<([String; 3], bool, Option<String>)>,
    /// Desktop's `ShowRecoveryKeyModal`, and whether the key was copied.
    pub recovery_key: Option<bool>,
    /// The section that waits for the answer to "You have unsaved changes".
    pub leaving: Option<SettingsTab>,
}

#[derive(Clone)]
pub enum Msg {
    Name(String),
    Email(String),
    PickAvatar,
    AvatarPicked(Result<Vec<PathBuf>, String>),
    AvatarLoaded(Result<PendingMedia, String>),
    Save,
    Saved(Result<(), String>),
    Menu,
    RecoveryKey(bool),
    CopyRecoveryKey,
    /// Opens the delete prompt, or changes its text. `None` closes it.
    Delete(Option<String>),
    ConfirmDelete,
    Deleted(Result<(), String>),
    /// A field of the Change Email prompt (0 is the current email, 1 the new and 2 the confirmed one).
    EmailField(usize, String),
    SaveEmail,
    EmailSaved(Result<(), String>),
    CloseEmail,
    /// The answer to "You have unsaved changes": save (true) or discard.
    Leave(bool),
}

impl ProfileForm {
    /// Desktop's `hasChanges`: an edited field differs from the profile, or an avatar is picked.
    pub fn dirty(&self, me: &User) -> bool {
        let changed = |edit: &Option<String>, now: &Option<String>| {
            edit.as_deref().is_some_and(|e| e.trim() != now.as_deref().unwrap_or_default())
        };
        changed(&self.name, &me.full_name) || changed(&self.email, &me.email) || self.avatar.is_some()
    }
}

/// Desktop's `Kh`: emails compare without case and outer spaces.
fn same_email(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

impl BeeperGuiApp {
    /// A Beeper API call on a worker thread. The demo account answers at once without the network.
    fn cloud_call(
        &self,
        call: impl FnOnce(&Cloud, &str) -> Result<(), CloudError> + Send + 'static,
        done: fn(Result<(), String>) -> Msg,
    ) -> Task<UiMessage> {
        let st = self.store.read();
        if st.config.demo {
            return Task::done(UiMessage::Profile(done(Ok(()))));
        }
        let (cloud, token) = (self.cloud.clone(), st.beeper_state.access_token.clone());
        let work = async move {
            tokio::task::spawn_blocking(move || call(&cloud, &token).map_err(|e| e.message)).await.expect("API thread")
        };
        Task::perform(work, move |r| UiMessage::Profile(done(r)))
    }

    pub fn update_profile(&mut self, msg: Msg) -> Task<UiMessage> {
        let f = &mut self.profile;
        f.menu &= matches!(msg, Msg::Menu);
        match msg {
            Msg::Name(v) => f.name = Some(v),
            Msg::Email(v) => f.email = Some(v),
            Msg::PickAvatar => {
                let pick = portal::open_files(portal::PORTAL.into(), tr("Images").to_string(), portal::Pick::Image);
                return Task::perform(pick, |r| UiMessage::Profile(Msg::AvatarPicked(r)));
            }
            Msg::AvatarPicked(Ok(paths)) => {
                let Some(path) = paths.into_iter().next() else { return Task::none() };
                let load = async move {
                    tokio::task::spawn_blocking(move || portal::load_file(&path)).await.expect("file load thread")
                };
                return Task::perform(load, |r| UiMessage::Profile(Msg::AvatarLoaded(r)));
            }
            Msg::AvatarPicked(Err(e)) | Msg::AvatarLoaded(Err(e)) => self.fail(Toast::new(e)),
            Msg::AvatarLoaded(Ok(m)) => match round_avatar(&m.bytes) {
                Ok(h) => f.avatar = Some((m.bytes, m.mime, h)),
                Err(_) => self.fail(Toast::new(tr("Failed to upload avatar, please try again.").into())),
            },
            Msg::Save => {
                let st = self.store.read();
                let me = st.me();
                let name =
                    f.name.as_deref().map(str::trim).filter(|n| *n != me.full_name.as_deref().unwrap_or_default());
                let email = f.email.as_deref().map(str::trim).filter(|e| *e != me.email.as_deref().unwrap_or_default());
                if let Some(email) = email {
                    f.change_email = Some(([String::new(), email.to_string(), String::new()], false, None));
                }
                let (name, avatar) = (name.map(str::to_string), f.avatar.take().map(|(b, m, _)| (b, m)));
                drop(st);
                if name.is_none() && avatar.is_none() {
                    return Task::none();
                }
                f.saving = true;
                if self.store.read().config.demo {
                    return Task::done(UiMessage::Profile(Msg::Saved(Ok(()))));
                }
                let save = beeper_core::matrix::set_profile(self.store.clone(), name, avatar);
                return Task::perform(save, |r| UiMessage::Profile(Msg::Saved(r)));
            }
            Msg::Saved(r) => {
                (f.saving, f.name) = (false, None);
                if let Err(e) = r {
                    self.fail(Toast::new(e));
                }
            }
            Msg::Menu => f.menu = !f.menu,
            Msg::RecoveryKey(open) => f.recovery_key = open.then_some(false),
            Msg::CopyRecoveryKey => {
                f.recovery_key = Some(true);
                return iced::clipboard::write(self.store.read().beeper_state.secrets.recovery_code.clone());
            }
            Msg::Delete(text) => {
                let busy = f.delete.as_ref().is_some_and(|d| d.1);
                if !busy {
                    f.delete = text.map(|t| (t, false, None));
                }
            }
            Msg::ConfirmDelete => {
                let d = f.delete.as_mut().expect("the delete prompt is open");
                if d.1 || Some(&d.0) != self.store.read().me().username.as_ref() {
                    return Task::none();
                }
                (d.1, d.2) = (true, None);
                return self.cloud_call(|c, token| c.delete_account(token), Msg::Deleted);
            }
            Msg::Deleted(Ok(())) => {
                f.delete = None;
                return self.update(UiMessage::Logout);
            }
            Msg::Deleted(Err(e)) => {
                let d = f.delete.as_mut().expect("the delete prompt is open");
                (d.1, d.2) = (false, Some(trf("Error deleting your account: %s", &[&e])));
            }
            Msg::EmailField(i, v) => f.change_email.as_mut().expect("the Change Email prompt is open").0[i] = v,
            Msg::SaveEmail => {
                let current = self.store.read().me().email.clone().unwrap_or_default();
                let c = f.change_email.as_mut().expect("the Change Email prompt is open");
                let [now, new, again] = &c.0;
                if c.1 || !same_email(now, &current) || !same_email(new, again) || new.trim().is_empty() {
                    return Task::none();
                }
                let new = new.trim().to_string();
                (c.1, c.2) = (true, None);
                return self.cloud_call(move |c, token| c.set_email(token, &new), Msg::EmailSaved);
            }
            Msg::EmailSaved(Ok(())) => {
                (f.change_email, f.email) = (None, None);
                return self.refresh_cloud_accounts();
            }
            Msg::EmailSaved(Err(e)) => {
                let c = f.change_email.as_mut().expect("the Change Email prompt is open");
                (c.1, c.2) = (false, Some(e));
            }
            Msg::CloseEmail => f.change_email = None,
            Msg::Leave(save) => {
                let to = f.leaving.take().expect("a section waits");
                let task = match save {
                    true => self.update_profile(Msg::Save),
                    false => {
                        self.profile = ProfileForm::default();
                        Task::none()
                    }
                };
                return Task::batch([task, self.update(UiMessage::Settings(settings::Msg::SelectTab(to)))]);
            }
        }
        Task::none()
    }

    /// Desktop's `ProfileSubView`.
    pub fn profile_page<'a>(&'a self, st: &BeeperStore) -> Vec<El<'a>> {
        let p = &self.pal;
        let f = &self.profile;
        let me = st.me();
        let name = f.name.clone().unwrap_or_else(|| me.full_name.clone().unwrap_or_default());
        let email = f.email.clone().unwrap_or_else(|| me.email.clone().unwrap_or_default());
        let pill = (st.whoami.plan != Plan::Free).then(|| plan_pill(p, st.whoami.plan).0);
        let menu = button(tinted(&CHEVRON_DOWN, 16.0, Color::WHITE))
            .padding(4)
            .style(btn_style([p.accent; 3], [Color::WHITE; 3], 6.0))
            .on_press(UiMessage::Profile(Msg::Menu));
        let head = row![semi(tr("Beeper Profile"), 15.0).color(p.strong), Space::new().width(Fill)]
            .push(pill)
            .push(menu)
            .spacing(8)
            .align_y(Center);
        let face: El<'a> = match &f.avatar {
            Some((_, _, h)) => image(h.clone()).width(52).height(52).into(),
            None => self.face(me.img_url.as_deref(), &name, 52.0, 0.0),
        };
        let edit = container(txt(trx("Edit", "button label"), 13.0).color(Color::WHITE))
            .center(52)
            .style(fill_box(p.strong.scale_alpha(0.7), 26.0));
        let avatar = mouse_area(hover(face, edit)).on_press(UiMessage::Profile(Msg::PickAvatar));
        let link = match (st.whoami.can_subscribe, st.whoami.plan) {
            (true, _) => tr("Upgrade to Beeper Plus"),
            (false, Plan::PlusPlus) => tr("Manage Beeper Plus Plus Subscription"),
            (false, _) => tr("Manage Beeper Plus Subscription"),
        };
        let upsell =
            st.whoami.can_subscribe.then_some(UiMessage::Settings(settings::Msg::SelectTab(SettingsTab::BeeperPlus)));
        let link = button(semi(link, 11.0).color(p.accent)).bare(p.accent);
        let field = |label: &'static str, value: String, on: Option<fn(String) -> Msg>| -> El<'a> {
            let input = text_input("", &value)
                .on_input_maybe(on.map(|on| move |v| UiMessage::Profile(on(v))))
                .size(14)
                .font(fw(Weight::Semibold))
                .padding(Padding::new(8.0).right(30.0))
                .style(profile_input(p));
            column![container(semi(tr(label), 11.0).color(p.sub)).padding(Padding::ZERO.left(8.0)), input]
                .spacing(4)
                .into()
        };
        let form = column![
            column![avatar, container(link.on_press_maybe(upsell)).padding(Padding::ZERO.top(8.0))].align_x(Center),
            field("Name", name, Some(Msg::Name)),
            field("Email", email, Some(Msg::Email)),
            field("Username", me.username.clone().unwrap_or_default(), None),
        ]
        .spacing(8)
        .align_x(Center);
        let save = (f.dirty(me) || f.saving).then(|| {
            let save = button(semi(trx("Save", "button label"), 12.0).color(p.text))
                .padding([4, 10])
                .style(pill_style(p))
                .on_press_maybe((!f.saving).then_some(UiMessage::Profile(Msg::Save)));
            container(save).align_right(Fill).padding(Padding::ZERO.bottom(8.0))
        });
        let helper = |t: &'static str| container(txt(tr(t), 11.0).color(p.sub).center()).center_x(Fill);
        let card = column![form]
            .push(save)
            .push(helper("Changing this info will not affect your accounts on other chat networks."))
            .push(helper("Your username is how other people can contact you on Beeper."))
            .spacing(8)
            .padding(16);
        let card = container(card).width(Fill).style(bordered_box(p.prefs_group, p.prefs_border, 1.0, 8.0));
        vec![head.into(), card.into()]
    }

    /// The menu, the prompts and the modals of the Profile page, over the Settings window.
    pub fn profile_overlay<'a>(&'a self) -> Option<El<'a>> {
        let p = &self.pal;
        let f = &self.profile;
        let st = self.store.read();
        if f.menu {
            let item =
                |label: &'static str, msg| menu_item(p, None, tr(label).into(), "", Some(UiMessage::Profile(msg)));
            let key = !st.beeper_state.secrets.recovery_code.is_empty();
            let items = column![]
                .push(key.then(|| item("Show Recovery Key", Msg::RecoveryKey(true))))
                .push(key.then(|| menu_rule(p)))
                .push(menu_item(p, None, tr("Logout").into(), "", Some(UiMessage::Logout)))
                .push(item("Delete account", Msg::Delete(Some(String::new()))));
            let menu = container(items.padding([4, 0])).width(200).style(shadow_box(p.menu, p.menu_border, 6.0, 16.0));
            let menu = container(mouse_area(menu).on_press(UiMessage::Noop)).align_right(Fill).padding([64, 34]);
            return Some(
                mouse_area(container(menu).width(Fill).height(Fill)).on_press(UiMessage::Profile(Msg::Menu)).into(),
            );
        }
        if f.leaving.is_some() {
            let actions = vec![(trx("Save", "button label"), UiMessage::Profile(Msg::Leave(true)))];
            let body = tr("Would you like to save your changes before leaving?").into();
            let discard = (trx("Discard", "button label"), UiMessage::Profile(Msg::Leave(false)));
            return Some(self.dialog(tr("You have unsaved changes").into(), body, actions, discard));
        }
        let card = |content: El<'a>, width: f32, close: UiMessage| -> El<'a> {
            let card =
                container(content).width(width).padding(20).style(shadow_box(p.elevated, p.menu_border, 12.0, 24.0));
            mouse_area(
                container(mouse_area(card).on_press(UiMessage::Noop))
                    .center(Fill)
                    .style(fill_box(Color::from_rgba(0.0, 0.0, 0.0, 0.45), 0.0)),
            )
            .on_press(close)
            .into()
        };
        let buttons = |busy: bool, cancel: UiMessage, label: &'static str, ok: Option<UiMessage>, danger: bool| {
            let fill = if danger { p.danger } else { p.accent };
            let ok = button(med(label, 13.0).color(Color::WHITE))
                .padding([6, 14])
                .style(btn_style([fill; 3], [Color::WHITE; 3], 8.0))
                .on_press_maybe(ok.filter(|_| !busy));
            let cancel = button(med(trx("Cancel", "button label"), 13.0).color(p.text))
                .padding([6, 14])
                .style(pill_style(p))
                .on_press_maybe((!busy).then_some(cancel));
            row![Space::new().width(Fill), cancel, ok].spacing(8)
        };
        let input = |value: &str, hint: &str, on: Box<dyn Fn(String) -> UiMessage>| {
            text_input(hint, value).on_input(on).size(13).padding(8).style(outline_input(p))
        };
        if let Some((typed, busy, error)) = &f.delete {
            let username = st.me().username.clone().unwrap_or_default();
            let ok = (*typed == username).then_some(UiMessage::Profile(Msg::ConfirmDelete));
            let warning = tr("<strong>Warning: This is a permanent action.</strong> When you delete your account:");
            let (strong, rest) =
                warning.trim_start_matches("<strong>").split_once("</strong>").expect("a bold warning");
            let item = |t: &'static str| {
                row![ic(icon::CLOSE, 14.0).color(p.danger), txt(tr(t), 13.0).color(p.text)]
                    .spacing(8)
                    .align_y(Center)
                    .into()
            };
            let content = column![
                bold(tr("Delete account"), 17.0).color(p.strong),
                rich_text![span::<(), _>(strong.to_string()).font(fw(Weight::Bold)), span(rest.to_string())]
                    .size(13)
                    .color(p.text),
                Column::with_children([
                    item("Your account will be permanently deleted"),
                    item("Your encrypted message history and network connections will be deleted"),
                    item("Your account details will be removed from our servers"),
                ])
                .spacing(6),
                txt(trf("To confirm, type your username %s below.", &[&username]), 13.0).color(p.text),
                input(typed, "", Box::new(|v| UiMessage::Profile(Msg::Delete(Some(v)))))
                    .on_submit(UiMessage::Profile(Msg::ConfirmDelete)),
            ]
            .push(error.as_ref().map(|e| txt(e.clone(), 12.0).color(p.danger)))
            .push(buttons(*busy, UiMessage::Profile(Msg::Delete(None)), tr("Delete account"), ok, true))
            .spacing(12);
            return Some(card(content.into(), 360.0, UiMessage::Profile(Msg::Delete(None))));
        }
        if let Some(([now, new, again], busy, error)) = &f.change_email {
            let current = st.me().email.clone().unwrap_or_default();
            let valid = same_email(now, &current) && same_email(new, again) && !new.trim().is_empty();
            let field = |label: &'static str, value: &str, hint: &'static str, i: usize| -> El<'_> {
                column![
                    semi(tr(label), 11.0).color(p.sub),
                    input(value, tr(hint), Box::new(move |v| UiMessage::Profile(Msg::EmailField(i, v))))
                ]
                .spacing(4)
                .into()
            };
            let content = column![
                bold(tr("Change Email"), 17.0).color(p.strong),
                txt(tr("To change your email, please confirm your current email address first."), 13.0).color(p.sub),
                field("Current Email", now, "Enter your current email", 0),
                field("New Email", new, "Enter your new email", 1),
                field("Confirm New Email", again, "Confirm your new email", 2),
            ]
            .push(error.as_ref().map(|e| txt(e.clone(), 12.0).color(p.danger)))
            .push(buttons(
                *busy,
                UiMessage::Profile(Msg::CloseEmail),
                trx("Save", "button label"),
                valid.then_some(UiMessage::Profile(Msg::SaveEmail)),
                false,
            ))
            .spacing(12);
            return Some(card(content.into(), 350.0, UiMessage::Profile(Msg::CloseEmail)));
        }
        let copied = f.recovery_key?;
        let key = st.beeper_state.secrets.recovery_code.clone();
        let label = match copied && st.beeper_state.has_backed_up_code {
            true => tr("Copied"),
            false => tr("Copy to Clipboard"),
        };
        let copy = button(med(label, 13.0).color(Color::WHITE))
            .padding([6, 14])
            .style(btn_style([p.accent; 3], [Color::WHITE; 3], 8.0))
            .on_press(UiMessage::Profile(Msg::CopyRecoveryKey));
        let content = column![
            container(txt(key, 15.0).font(fw(Weight::Semibold)).color(p.strong).center())
                .padding(12)
                .width(Fill)
                .style(fill_box(p.chip, 8.0)),
            txt(format!("{}\n{}", tr("Do not share your recovery key."), tr("Put it in a safe place.")), 12.0)
                .color(p.sub)
                .center(),
            copy,
        ]
        .spacing(12)
        .align_x(Center);
        Some(card(content.into(), 350.0, UiMessage::Profile(Msg::RecoveryKey(false))))
    }
}

/// Desktop's `ProfileSubView` input: semibold text in an 8 px rounded box, gray when it is disabled.
fn profile_input(p: &Palette) -> impl Fn(&Theme, text_input::Status) -> text_input::Style + use<> {
    let (edge, text, sub, accent) = (p.chip, p.strong, p.sub, p.accent);
    move |_, status| text_input::Style {
        background: match status {
            text_input::Status::Disabled => edge.into(),
            _ => CLEAR.into(),
        },
        border: iced::Border { color: edge, width: 1.0, radius: 8.0.into() },
        icon: sub,
        placeholder: sub,
        value: if matches!(status, text_input::Status::Disabled) { sub } else { text },
        selection: accent.scale_alpha(0.45),
    }
}
