//! The child windows of the app: each is one of [`Win`] in [`BeeperGuiApp::children`], which gives its title, its view,
//! its keys and what it forgets as it closes. The chat, space and "Media in %s" windows stay apart, because their panes
//! swap with the main window's.

use super::*;
use crate::merge_editor::MergeDraft;
use crate::modals::Modal;
use crate::nickname::NicknameDraft;

/// A child window and what it shows.
pub(crate) enum Win {
    /// Desktop's `ChatThemeBrowserWindow` of a chat.
    Theme(String),
    /// The Edit Labels window of a chat.
    Labels(String),
    /// The poll votes window of a chat and its poll message.
    Vote(String, String),
    Poll(PollDraft),
    Label(LabelDraft),
    LabelSettings(LabelSettings),
    Spaces(SpacesDraft),
    /// Desktop's merged chat editor (`D3`).
    Merge(MergeDraft),
    Group(GroupDraft),
    /// Desktop's Account Nickname window of an account.
    Nickname(NicknameDraft),
    /// Desktop's Blocked users window.
    Blocked,
    /// Desktop's How It Works window of the Chat Info behavior menu.
    How,
    /// Desktop's `AddAccountsBrowserWindow`.
    Networks,
    /// Desktop's `VERIFICATION` modal window.
    Verify,
    /// Desktop's `ReauthRequiredModal`.
    Reauth,
    /// Desktop's Enter your Recovery Key modal.
    EnterKey,
    /// Desktop's windows that open once each, like its modal windows.
    Modal(Modal),
}

/// What a kind of child window shows, by which [`Children::get`] and the like find the windows of that kind.
pub(crate) trait Kind: Sized + 'static {
    fn of(w: &Win) -> Option<&Self>;
    fn of_mut(w: &mut Win) -> Option<&mut Self>;
}

macro_rules! kinds {
    ($($win:ident($kind:ty)),*) => {$(
        impl Kind for $kind {
            fn of(w: &Win) -> Option<&Self> { if let Win::$win(d) = w { Some(d) } else { None } }
            fn of_mut(w: &mut Win) -> Option<&mut Self> { if let Win::$win(d) = w { Some(d) } else { None } }
        }
    )*};
}

kinds!(
    Poll(PollDraft),
    Label(LabelDraft),
    LabelSettings(LabelSettings),
    Spaces(SpacesDraft),
    Merge(MergeDraft),
    Group(GroupDraft),
    Nickname(NicknameDraft)
);

/// The child windows and what they show, in the order that they opened.
#[derive(Default)]
pub(crate) struct Children(Vec<(window::Id, Win)>);

impl Children {
    /// What child window `id` shows.
    pub(crate) fn at(&self, id: window::Id) -> Option<&Win> {
        self.0.iter().find(|w| w.0 == id).map(|w| &w.1)
    }

    /// The first open child window that `is` picks.
    pub(crate) fn find(&self, is: impl Fn(&Win) -> bool) -> Option<window::Id> {
        self.0.iter().find(|w| is(&w.1)).map(|w| w.0)
    }

    pub(crate) fn push(&mut self, id: window::Id, w: Win) {
        self.0.push((id, w));
    }

    /// Forgets the child window that `is` picks before it closes, and gives its ID.
    pub(crate) fn take(&mut self, is: impl Fn(&Win) -> bool) -> Option<window::Id> {
        let i = self.0.iter().position(|w| is(&w.1))?;
        Some(self.0.remove(i).0)
    }

    /// What the first open window of kind `T` shows.
    pub(crate) fn get<T: Kind>(&self) -> Option<&T> {
        self.0.iter().find_map(|w| T::of(&w.1))
    }

    pub(crate) fn get_mut<T: Kind>(&mut self) -> Option<&mut T> {
        self.0.iter_mut().find_map(|w| T::of_mut(&mut w.1))
    }

    /// What the open windows of kind `T` show.
    pub(crate) fn all<T: Kind>(&self) -> impl Iterator<Item = &T> {
        self.0.iter().filter_map(|w| T::of(&w.1))
    }

    pub(crate) fn all_mut<T: Kind>(&mut self) -> impl Iterator<Item = &mut T> {
        self.0.iter_mut().filter_map(|w| T::of_mut(&mut w.1))
    }

    /// Forgets child window `id`, and gives what it showed.
    pub(crate) fn remove(&mut self, id: window::Id) -> Option<Win> {
        let i = self.0.iter().position(|w| w.0 == id)?;
        Some(self.0.remove(i).1)
    }

    /// The open child windows.
    pub(crate) fn iter(&self) -> impl Iterator<Item = &(window::Id, Win)> {
        self.0.iter()
    }
}

impl BeeperGuiApp {
    /// Raises the child window that `is` picks, else opens a window with `open` that shows `win`.
    pub(crate) fn open_once(
        &mut self, is: impl Fn(&Win) -> bool, open: impl FnOnce(&mut Self) -> (window::Id, Task<UiMessage>),
        win: impl FnOnce(window::Id) -> Win,
    ) -> Task<UiMessage> {
        if let Some(id) = self.children.find(is) {
            return self.raise(id);
        }
        let (id, open) = open(self);
        self.children.push(id, win(id));
        open
    }

    /// The title of child window `w`.
    pub(crate) fn child_title(&self, w: &Win) -> String {
        match w {
            Win::Theme(_) => tr("Chat Theme"),
            Win::Labels(_) => tr("Edit Labels"),
            Win::LabelSettings(_) => tr("Label Settings"),
            Win::Poll(_) => tr("New Poll"),
            Win::Label(d) => match (d.label.is_some(), d.step) {
                (false, _) => tr("New Label"),
                (true, LabelStep::Details) => tr("Edit Label"),
                (true, _) => tr("Chats"),
            },
            Win::Spaces(_) => tr("Edit Spaces"),
            // Desktop's `D3` titles, which it does not translate.
            Win::Merge(d) => match d.union {
                Some(_) => "Edit merged chat",
                None => "Merge Chats",
            },
            Win::Nickname(_) => tr("Account Nickname"),
            Win::Group(d) => tr(if d.personal { "Edit Chat for Me" } else { "Edit Group" }),
            Win::Vote(_, mid) => {
                let st = self.store.read();
                let total = st.loaded(mid).and_then(|m| m.poll.as_ref()).map_or(0, PollData::voters);
                return trn("%d vote", "%d votes", total, &[&total]);
            }
            Win::Blocked => tr("Blocked users"),
            // Desktop's `AddAccountsBrowserWindow` title, which it does not translate.
            Win::Networks => "Add an Account",
            Win::How => tr("How It Works"),
            Win::Verify => "Beeper",
            Win::Reauth => tr("Authentication Required"),
            Win::EnterKey => self.enter_key_title(),
            Win::Modal(m) => m.title(),
        }
        .into()
    }

    pub(crate) fn view_child<'a>(&'a self, id: window::Id, w: &'a Win) -> El<'a> {
        match w {
            Win::Theme(chat) => self.view_chat_theme(chat),
            Win::Labels(chat) => self.view_edit_labels(chat),
            Win::Vote(chat, mid) => self.view_poll_votes(chat, mid),
            Win::Poll(d) => self.view_poll_creator(d),
            Win::Label(d) => self.view_create_label(d),
            Win::LabelSettings(w) => self.view_label_settings(w),
            Win::Merge(d) => self.view_merge_editor(d),
            Win::Spaces(d) => self.view_edit_spaces(d),
            Win::Group(d) => self.view_edit_group(d),
            Win::Nickname(d) => self.view_nickname(d),
            Win::Blocked => self.view_blocked_users(id),
            Win::How => self.view_how_it_works(),
            Win::Networks => self.view_add_accounts(),
            Win::Verify => self.view_verify(),
            Win::Reauth => self.view_reauth(),
            Win::EnterKey => self.view_enter_key(),
            Win::Modal(m) => self.view_modal(*m),
        }
    }

    /// Forgets child window `id`, which closed, and what it showed.
    pub(crate) fn child_closed(&mut self, id: window::Id) {
        let Some(w) = self.children.remove(id) else { return };
        match w {
            Win::Networks => self.networks.menu = None,
            // Desktop's `ReauthRequiredModal` closed without a code ends the logout of a device.
            Win::Reauth => self.devices_busy = false,
            Win::Modal(m) => self.modal_closed(m),
            _ => {}
        }
    }

    /// The keys of child window `id` that closes on Escape or confirms on Ctrl+Enter, or `None` for the keys of the
    /// app.
    pub(crate) fn child_key(&mut self, id: window::Id, key: &Key, mods: Modifiers) -> Option<Task<UiMessage>> {
        let escape = *key == Key::Named(Named::Escape);
        let close = |app: &Self, closes: bool| if closes { app.close_window(id) } else { Task::none() };
        let confirm = *key == Key::Named(Named::Enter) && mods.command();
        Some(match self.children.at(id)? {
            Win::LabelSettings(_) | Win::How => close(self, escape || confirm),
            Win::Theme(_)
            | Win::Labels(_)
            | Win::Vote(..)
            | Win::Spaces(_)
            | Win::Merge(_)
            | Win::Group(_)
            | Win::Blocked
            | Win::Nickname(_) => close(self, escape),
            Win::Label(_) if confirm => self.label_edit(LabelEdit::Advance),
            Win::Label(_) => close(self, escape),
            Win::Poll(_) if escape => self.poll_edit(id, PollEdit::Close),
            Win::Poll(_) if confirm => self.poll_edit(id, PollEdit::Send),
            Win::Poll(_) => Task::none(),
            // Desktop's `onEscapePressed` closes the Add an Account window, after the menu of a network.
            Win::Networks => {
                let closes = escape && self.networks.menu.take().is_none();
                close(self, closes)
            }
            Win::Reauth => self.reauth_key(id, key),
            &Win::Modal(m) => close(self, escape && m.escape_closes()),
            Win::EnterKey => close(self, escape),
            Win::Verify => return None,
        })
    }
}
