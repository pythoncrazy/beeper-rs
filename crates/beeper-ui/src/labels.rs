//! Desktop's labels: their windows, the Edit Labels switches, mute, Show in Inbox, and the undo of a change or a deletion.

use super::*;

#[derive(Clone)]
pub enum Msg {
    /// Opens the New label window with these chats picked.
    OpenCreate(Vec<String>),
    /// Desktop's `toggleLabel` for the chats of a menu: the clicked chat, or the selected chats.
    Toggle(String, Vec<String>, bool),
    /// Desktop's Edit Labels window of a chat.
    OpenEdit(String),
    /// A switch in the Edit Labels window: `toggleLabel` without the toast.
    Switch(String, String, bool),
    /// Puts back the name, color, icon and chats of a label.
    Restore(Box<Label>),
    /// Desktop's `restoreLabel`: makes a deleted label again, in its old place in the sidebar.
    RestoreDeleted(Box<Label>, Vec<SpaceOrder>),
    /// Desktop's Edit Label sheet (`Lst`) of Label Settings: the name, icon and color of a label.
    Edit(String),
    /// Desktop's Chats sheet (`Nst`) of Label Settings: the chats of a label.
    EditChats(String),
    Change(LabelEdit),
    Delete(String),
    /// Desktop's `setLabelInboxVisibility`.
    SetInbox(String, Option<&'static str>),
    /// Desktop's `toggleMuteLabel`.
    Mute(String, bool),
    /// Desktop's `CO`: opens the Label Settings window of a label.
    OpenSettings(String),
    Pane(window::Id, LabelPane),
}

impl BeeperGuiApp {
    pub(crate) fn labels(&mut self, msg: Msg) -> Task<UiMessage> {
        match msg {
            Msg::Toggle(label, chat, add) => self.toggle_label(label, chat, add, true),
            Msg::Switch(label, chat, add) => self.toggle_label(label, vec![chat], add, false),
            Msg::OpenEdit(chat) => {
                self.list.chat_menu = None;
                if let Some(&(id, _)) = self.labels_windows.iter().find(|w| w.1 == chat) {
                    return window::gain_focus(id);
                }
                let (id, open) = self.open_child(Size::new(303.0, 488.0));
                self.labels_windows.push((id, chat));
                return open;
            }
            Msg::RestoreDeleted(l, order) => {
                let mut st = self.store.write();
                let id = st.create_label(&l.name, l.color, l.icon.clone(), l.rooms.clone());
                if l.inbox.is_some() {
                    st.set_label_inbox(&id, l.inbox);
                }
                if l.muted {
                    st.set_label_muted(&id, true);
                }
                drop(st);
                let (old, new) = (format!("label:{}", l.id), format!("label:{id}"));
                let order =
                    order.into_iter().map(|o| if o.id == old { SpaceOrder { id: new.clone(), ..o } } else { o });
                self.patch_settings(|s| s.spaces_order = order.collect());
            }
            Msg::Restore(l) => {
                let id = self.label_id(l.id);
                self.store.write().update_label(&id, &l.name, l.color, l.icon, l.rooms);
            }
            Msg::OpenCreate(chats) => {
                self.list.chat_menu = None;
                if let Some(d) = &self.label_window {
                    return window::gain_focus(d.id);
                }
                let intro = !self.store.read().settings.labels_intro_seen;
                if intro {
                    self.patch_settings(|s| s.labels_intro_seen = true);
                }
                let (id, open) = self.open_child(LABEL_SETTINGS_SIZE);
                let step = if intro { LabelStep::Intro } else { LabelStep::Pick };
                self.label_window = Some(LabelDraft { chats, step, ..LabelDraft::new(id) });
                return if intro { open } else { open.chain(operation::focus("label-search")) };
            }
            Msg::Edit(label) => return self.open_label_sheet(label, LabelStep::Details),
            Msg::EditChats(label) => return self.open_label_sheet(label, LabelStep::Pick),
            Msg::Mute(id, muted) => {
                self.list.chat_menu = None;
                let id = self.label_id(id);
                let mut st = self.store.write();
                let name = st.labels.iter().find(|l| l.id == id).expect("a muted label").name.clone();
                st.set_label_muted(&id, muted);
                drop(st);
                let what = format!("{} \"{name}\"", if muted { "Mute" } else { "Unmute" });
                self.act(what, UiMessage::Labels(Msg::Mute(id, !muted)), None);
            }
            Msg::SetInbox(id, inbox) => {
                self.list.chat_menu = None;
                let id = self.label_id(id);
                let mut st = self.store.write();
                let old = st.labels.iter().find(|l| l.id == id).expect("a label").clone();
                st.set_label_inbox(&id, inbox);
                drop(st);
                let to = INBOX_OPTIONS.iter().find(|o| o.0 == inbox.unwrap_or("all")).expect("an inbox option").1;
                let what = format!("Set \"{}\" inbox visibility to {to}", old.name);
                self.act(what, UiMessage::Labels(Msg::SetInbox(id, old.inbox)), None);
            }
            Msg::OpenSettings(label) => {
                self.list.chat_menu = None;
                let label = self.label_id(label);
                if let Some(w) = self.label_settings.iter().find(|w| w.label == label) {
                    return window::gain_focus(w.id);
                }
                let (id, open) = self.open_child(LABEL_SETTINGS_SIZE);
                self.label_settings.push(LabelSettings { id, label, menu: false });
                return open;
            }
            Msg::Pane(id, e) => {
                let w = self.label_settings.iter_mut().find(|w| w.id == id).expect("an open Label Settings window");
                match e {
                    LabelPane::Menu(open) => w.menu = open,
                    LabelPane::Delete => {
                        (self.confirm, self.delete_target) = (Some(ASK_DELETE_LABEL), Some(w.label.clone()));
                        self.delete_in = Some(id);
                    }
                    LabelPane::Done => return window::close(id),
                }
            }
            Msg::Delete(id) => {
                self.confirm = None;
                let id = self.label_id(id);
                let mut st = self.store.write();
                let label = st.labels.iter().find(|l| l.id == id).expect("a deleted label").clone();
                let order = st.settings.spaces_order.clone();
                st.delete_label(&id);
                drop(st);
                let (what, text) = (format!("Delete label \"{}\"", label.name), format!("Deleted \"{}\"", label.name));
                self.act(what, UiMessage::Labels(Msg::RestoreDeleted(Box::new(label), order)), Some((text, None)));
                if let Some(d) = self.spaces_window.as_mut() {
                    d.order.retain(|o| *o != format!("label:{id}"));
                }
                if self.list().selected_account_filter.as_deref() == Some(&format!("label:{id}")) {
                    self.list.selected_account_filter = None;
                }
                let shown = self.label_settings.iter().filter(|w| w.label == id);
                return Task::batch(shown.map(|w| window::close(w.id)).collect::<Vec<_>>());
            }
            Msg::Change(edit) => return self.label_edit(edit),
        }
        self.settle()
    }
}

pub(crate) fn label_glyph(id: &str) -> Option<&'static svg::Handle> {
    LABEL_ICONS.iter().find(|i| i.0 == id).map(|i| &i.1)
}

/// The gradient of a label color. Desktop ignores unknown colors.
pub(crate) fn label_fill(color: Option<u8>) -> Option<Background> {
    let (top, bottom) = LABEL_COLORS.get(usize::from(color?)).copied()?;
    let g = Linear::new(Radians(std::f32::consts::PI)).add_stop(0.0, hex(top)).add_stop(1.0, hex(bottom));
    Some(Background::Gradient(Gradient::Linear(g)))
}

/// The number of rows of Desktop's label items (`kS`): the name heading, Mute or Unmute, and Label Settings.
pub(crate) fn label_rows(l: &Label) -> f32 {
    2.0 + f32::from(u8::from(!l.name.is_empty()))
}
