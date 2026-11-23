//! Desktop's New Chat: its drafts, the contact and user search, the networks of a contact, and the start of a chat or group.

use super::*;
use iced::widget::{column, Container};
use std::sync::LazyLock;

/// The plus of Desktop's `PlusIcon` on the avatar of the `GroupTitleBar`.
static PLUS: LazyLock<svg::Handle> = LazyLock::new(|| {
    svg::Handle::from_memory(
        br#"<svg width="7" height="8" viewBox="0 0 7 8" xmlns="http://www.w3.org/2000/svg"><path d="M6.82227 2.78906V4.27246H0V2.78906H6.82227ZM4.21094 0V7.24609H2.61816V0H4.21094Z" fill="white"/></svg>"#
            .as_slice(),
    )
});

#[derive(Clone)]
pub enum Msg {
    Open,
    Select(u32),
    Delete(u32),
    /// A click on a contact in the New Chat view, by contact ID: Desktop shows the networks of the contact.
    ToggleContact(String),
    /// The user that `resolve_identifier` found for a slot of the networks of a contact or a typed identifier.
    ContactNetwork(String, usize, Result<Option<serde_json::Value>, String>),
    /// Desktop's lookup of a typed phone number, email or username, after the user stops typing.
    LookupIdentifier(String),
    /// The users of an account's `search_users` for a New Chat query.
    SearchedUsers(String, String, Result<Vec<User>, String>),
    ToggleResults,
    /// A click on a network of the expanded contact: the account and its user.
    PickUser(String, Box<User>),
    /// Desktop's `createChatFromDraft` for the open draft.
    Start,
    /// The bridge made a direct chat for a New Chat draft or for Reply Privately with this quote.
    DmCreated(Option<u32>, String, Option<String>, Result<String, String>),
    /// The bridge made the group of a New Chat draft, by draft and account.
    GroupCreated(u32, String, Result<String, String>),
    /// Desktop's New Group row: the open draft becomes a group draft.
    NewGroup,
    /// The group name of the open draft.
    GroupTitle(String),
    /// A click on the avatar of Desktop's `GroupTitleBar`, which asks for an image.
    PickAvatar,
    /// The image file that the user picked for the group avatar of a draft.
    AvatarPicked(u32, Result<Vec<std::path::PathBuf>, String>),
    AvatarLoaded(u32, Result<PendingMedia, String>),
    /// The Encrypted toggle of Desktop's "Room encryption" step of a new Matrix group.
    Encrypted(bool),
    Query(String),
    ToggleExpanded,
    ToggleChats,
    ToggleJoinMatrix,
    /// Join or Enter in the Join a Matrix room view.
    JoinMatrixRoom,
    /// The end of a join of the Join a Matrix room view of a draft, and whether the room had no routing servers.
    MatrixRoomJoined(Option<u32>, Result<String, String>, bool),
    ContactsLoaded(Vec<User>),
    /// The account filter of the New Chat view.
    Account(Option<String>),
    ToggleAccounts,
}

impl BeeperGuiApp {
    pub(crate) fn new_chat(&mut self, msg: Msg) -> Task<UiMessage> {
        match msg {
            Msg::Open => {
                self.list.chat_menu = None;
                let id = self.new_chats.iter().map(|d| d.id).max().map_or(0, |n| n + 1);
                let draft = NewChat { id, users: Vec::new(), group: false, title: String::new(), photo: None };
                self.new_chats.insert(0, draft);
                return self.update(UiMessage::NewChat(Msg::Select(id)));
            }
            Msg::Select(id) => {
                if self.creating_chat != Some(id) {
                    self.leave_draft();
                    self.group_encrypted = true;
                }
                self.creating_chat = Some(id);
                self.show_command_palette = false;
                self.new_chat_query.clear();
                (self.new_chat_expanded, self.new_chat_chats_expanded) = (false, false);
                (self.new_chat_account, self.new_chat_accounts_open) = (None, false);
                (self.contact_networks, self.new_chat_lookup, self.new_chat_search) = (None, None, None);
                self.new_chat_results_expanded = false;
                self.join_matrix_mode = false;
                return operation::focus("new-chat-input");
            }
            Msg::ToggleContact(id) => {
                if self.contact_networks.take().is_some_and(|(c, _)| c == id) {
                    return Task::none();
                }
                let st = self.store.read();
                let c = st.contacts.iter().find(|u| u.id == id).expect("a listed contact").clone();
                let accounts: Vec<String> =
                    rail_accounts(&st).into_iter().filter(|a| self.can(a, "create_dm")).collect();
                drop(st);
                let ids = (c.phone_number.as_deref(), c.email.as_deref(), c.username.as_deref());
                let known = c.mxid.as_ref().zip(beeper_core::cloud::contact_account(&id)).map(|(mxid, account)| {
                    let user = User {
                        id: mxid.clone(),
                        full_name: c.full_name.clone(),
                        img_url: c.img_url.clone(),
                        ..Default::default()
                    };
                    (account, user)
                });
                let (nets, tasks) = self.resolve_identifiers(&id, &accounts, ids, known);
                self.contact_networks = Some((id, nets));
                return tasks;
            }
            Msg::LookupIdentifier(q) => {
                let q = q.trim();
                if q != self.new_chat_query.trim() || q.is_empty() {
                    return Task::none();
                }
                let only =
                    self.new_chats.iter().find(|d| Some(d.id) == self.creating_chat).and_then(|d| d.users.first());
                let only = only.map(|(a, _)| a.clone()).or_else(|| self.new_chat_account.clone());
                let search = self.search_users(q, only);
                let phone = typed_phone(q).then(|| fmt_phone(&phone_digits(q)));
                let email = (q.contains('@') && !q.starts_with('@')).then_some(q);
                let named = q.as_bytes().windows(2).any(|w| w[0] == b'@' && w[1].is_ascii_alphanumeric());
                let username = named.then_some(q);
                if phone.is_none() && email.is_none() && username.is_none() {
                    return search;
                }
                let st = self.store.read();
                let only =
                    self.new_chats.iter().find(|d| Some(d.id) == self.creating_chat).and_then(|d| d.users.first());
                let only = only.map(|(a, _)| a.clone()).or_else(|| self.new_chat_account.clone());
                let listed = rail_accounts(&st).into_iter().filter(|a| self.can(a, "create_dm"));
                let accounts: Vec<String> = listed.filter(|a| only.as_ref().is_none_or(|o| o == a)).collect();
                drop(st);
                let key = format!("lookup:{q}");
                let (nets, tasks) =
                    self.resolve_identifiers(&key, &accounts, (phone.as_deref(), email, username), None);
                self.new_chat_lookup = Some((key, nets));
                return Task::batch([search, tasks]);
            }
            Msg::SearchedUsers(q, account, r) => {
                let Some((_, pending, found)) = self.new_chat_search.as_mut().filter(|(s, ..)| *s == q) else {
                    return Task::none();
                };
                *pending -= 1;
                let users = r.inspect_err(|e| eprintln!("search users failed for {account}: {e}")).unwrap_or_default();
                found.extend(users.into_iter().map(|u| (account.clone(), u)));
            }
            Msg::ToggleResults => self.new_chat_results_expanded = !self.new_chat_results_expanded,
            Msg::ContactNetwork(id, slot, r) => {
                let open = [&mut self.contact_networks, &mut self.new_chat_lookup].into_iter().flatten();
                let Some((_, nets)) = open.into_iter().find(|(c, _)| *c == id) else {
                    return Task::none();
                };
                let found = r.inspect_err(|e| eprintln!("resolve identifier failed: {e}")).ok().flatten();
                nets[slot].1 = Some(found.map(|v| beeper_core::matrix::bridge_user(&v)));
            }
            Msg::PickUser(account, user) => {
                let n = self.creating_chat.expect("an open draft");
                let group = self.can_group(&account);
                let max = self
                    .caps
                    .get(&account)
                    .and_then(|c| c["group_creation"]["group"]["participants"]["max_length"].as_u64());
                let draft = self.new_chats.iter_mut().find(|d| d.id == n).expect("the open draft");
                if !group {
                    draft.users = vec![(account, *user)];
                } else if draft.users.iter().any(|(_, u)| u.id == user.id) {
                    return Task::none();
                } else if let Some(max) = max.filter(|&m| m > 0 && draft.users.len() as u64 >= m) {
                    // Desktop's `C$t` toast.
                    let max = usize::try_from(max).expect("a participant limit fits in usize");
                    let (one, many) =
                        ("%1$s groups can have up to %2$d person", "%1$s groups can have up to %2$d people");
                    self.show(Toast::new(trn(one, many, max, &[&network_label(&account), &max])));
                    return Task::none();
                } else {
                    draft.users.push((account, *user));
                }
                draft.group |= draft.users.len() > 1;
                self.new_chat_query.clear();
                return operation::focus("new-chat-input");
            }
            Msg::NewGroup => {
                let n = self.creating_chat.expect("an open draft");
                self.new_chats.iter_mut().find(|d| d.id == n).expect("the open draft").group = true;
                return operation::focus("new-chat-input");
            }
            Msg::GroupTitle(t) => {
                let n = self.creating_chat.expect("an open draft");
                self.new_chats.iter_mut().find(|d| d.id == n).expect("the open draft").title = t;
            }
            // Desktop's Start (`D`), and Start of the "Room encryption" step (`R`), which Matrix groups of more than
            // one person ask first.
            Msg::Start => {
                let Some(n) = self.creating_chat.filter(|_| !self.creating_thread()) else { return Task::none() };
                let draft = self.new_chats.iter().find(|d| d.id == n).expect("the open draft");
                let Some((account, user)) = draft.users.first().cloned() else { return Task::none() };
                if draft.group {
                    let asked = self.confirm.take_if(|d| *d == ASK_ROOM_ENCRYPTION).is_some();
                    let bridge = self.bridge(&account);
                    let matrix = matches!(bridge, beeper_core::matrix::ChatBridge::Matrix(_));
                    if matrix && draft.users.len() > 1 && !asked {
                        self.confirm = Some(ASK_ROOM_ENCRYPTION);
                        return Task::none();
                    }
                    let users = draft.users.iter().map(|(_, u)| u.id.clone()).collect();
                    // Desktop's `createChatFromDraft` sends the name and the avatar only when the bridge allows a name.
                    let named = self.caps[&account]["group_creation"]["group"]["name"]["allowed"] == true;
                    let title = if named { draft.title.clone() } else { String::new() };
                    let photo =
                        draft.photo.as_ref().filter(|_| named).map(|(bytes, mime, _)| (bytes.clone(), mime.clone()));
                    self.status_banner = tr("Creating group chat...").into();
                    self.sticky_banner = Some(self.status_banner.clone());
                    let store = self.store.clone();
                    // Desktop's `createGroupThread`: an `unencrypted_group` when the Matrix step turned encryption
                    // off, then `setRoomAvatar`.
                    let kind = if asked && !self.group_encrypted { "unencrypted_group" } else { "group" };
                    let create = async move {
                        let room = bridge.create_group(kind.into(), title, users).await?;
                        if let Some((bytes, mime)) = photo {
                            beeper_core::matrix::set_room_avatar(store, room.clone(), bytes, mime).await?;
                        }
                        Ok(room)
                    };
                    return Task::perform(create, move |r| {
                        UiMessage::NewChat(Msg::GroupCreated(n, account.clone(), r))
                    });
                }
                return self.open_dm(account, user, Some(n), None);
            }
            Msg::PickAvatar => {
                let n = self.creating_chat.expect("an open draft");
                let pick = portal::open_files(portal::PORTAL.into(), tr("Open").to_string(), portal::Pick::Image);
                return Task::perform(pick, move |r| UiMessage::NewChat(Msg::AvatarPicked(n, r)));
            }
            Msg::AvatarPicked(n, Ok(paths)) => {
                let Some(path) = paths.into_iter().next() else { return Task::none() };
                return Task::perform(portal::load_off_thread(path), move |r| {
                    UiMessage::NewChat(Msg::AvatarLoaded(n, r))
                });
            }
            Msg::AvatarPicked(_, Err(e)) | Msg::AvatarLoaded(_, Err(e)) => self.fail(Toast::new(e)),
            Msg::AvatarLoaded(n, Ok(m)) => match round_avatar(&m.bytes) {
                Ok(h) => {
                    if let Some(d) = self.new_chats.iter_mut().find(|d| d.id == n) {
                        d.photo = Some((m.bytes, m.mime, h));
                    }
                }
                Err(_) => self.fail(Toast::new(tr("Failed to upload image").into())),
            },
            Msg::Encrypted(on) => self.group_encrypted = on,
            Msg::DmCreated(n, account, quote, r) => self.chat_created(n, &account, quote, r, "creating thread"),
            Msg::GroupCreated(n, account, r) => self.chat_created(Some(n), &account, None, r, "creating group chat"),
            Msg::Delete(id) => {
                // The second click of a double click on the close button hits the draft that the first click removed.
                let Some(i) = self.new_chats.iter().position(|d| d.id == id) else { return Task::none() };
                self.new_chats.remove(i);
                self.list.chat_menu = None;
                if self.creating_chat != Some(id) {
                    return Task::none();
                }
                self.creating_chat = None;
                let st = self.store.read();
                let rest = self.pins_and_rows(&st).1.into_iter().map(|c| UiMessage::SelectChat(c.id.clone()));
                let items: Vec<_> =
                    self.new_chats.iter().map(|d| UiMessage::NewChat(Msg::Select(d.id))).chain(rest).collect();
                drop(st);
                if let Some(next) = items.get(i).or(i.checked_sub(1).and_then(|j| items.get(j))) {
                    return self.update(next.clone());
                }
            }
            Msg::Query(q) => {
                (self.new_chat_query, self.join_room.1) = (q.clone(), None);
                (self.new_chat_lookup, self.new_chat_search) = (None, None);
                let lookup = UiMessage::NewChat(Msg::LookupIdentifier(q));
                return self.timers.lookup.after(std::time::Duration::from_millis(300), lookup);
            }
            Msg::ToggleExpanded => self.new_chat_expanded = !self.new_chat_expanded,
            Msg::ToggleChats => self.new_chat_chats_expanded = !self.new_chat_chats_expanded,
            Msg::JoinMatrixRoom => return self.join_matrix_room(),
            Msg::MatrixRoomJoined(draft, r, unrouted) => return self.matrix_room_joined(draft, r, unrouted),
            Msg::ToggleJoinMatrix => {
                self.join_matrix_mode = !self.join_matrix_mode;
                self.new_chat_query.clear();
                return operation::focus("new-chat-input");
            }
            Msg::Account(account) => {
                (self.new_chat_account, self.new_chat_accounts_open) = (account, false);
            }
            Msg::ToggleAccounts => self.new_chat_accounts_open = !self.new_chat_accounts_open,
            Msg::ContactsLoaded(contacts) => {
                if !contacts.is_empty() {
                    self.store.write().set_contacts(contacts);
                }
            }
        }
        self.settle()
    }
}

/// Whether typed text looks like a phone number: 7 to 15 digits with phone punctuation.
pub(crate) fn typed_phone(q: &str) -> bool {
    let digits = q.chars().filter(char::is_ascii_digit).count();
    (7..=15).contains(&digits) && q.chars().all(|c| c.is_ascii_digit() || " +-().".contains(c))
}

/// Desktop's `SD`: a person once per bridge, by phone number, username, email or ID.
pub(crate) fn person_key(account_id: &str, u: &User) -> String {
    let id = u.phone_number.as_deref().map(phone_digits);
    let id = id.or_else(|| u.username.clone()).or_else(|| u.email.clone()).unwrap_or_else(|| u.id.clone());
    format!("{}:{id}", brand_key(account_id))
}

/// Desktop's `pb`: the name that sorts a person.
pub(crate) fn person_name(u: &User) -> &str {
    [&u.full_name, &u.custom_nickname, &u.username, &u.phone_number, &u.email]
        .into_iter()
        .find_map(|s| s.as_deref())
        .unwrap_or(&u.id)
}

/// Desktop's `Chats` category of New Chat: the people of the chats of the accounts, once per bridge and
/// identifier from their most recent chat, that match the query, by name.
pub(crate) fn chat_people(st: &BeeperStore, accounts: &[String], only: Option<&str>, q: &str) -> Vec<(String, User)> {
    let mut found: HashMap<String, (&Chat, &User)> = HashMap::new();
    let chats = st.chats.iter().filter(|c| accounts.contains(&c.account_id) && only.is_none_or(|a| c.account_id == a));
    for c in chats {
        for u in c.participants.items.iter().filter(|u| u.is_self != Some(true)) {
            let key = person_key(&c.account_id, u);
            if found.get(&key).is_none_or(|(old, _)| c.last_activity > old.last_activity) {
                found.insert(key, (c, u));
            }
        }
    }
    let fields =
        |u: &User| [&u.full_name, &u.custom_nickname, &u.username, &u.phone_number, &u.email].map(Clone::clone);
    let matches =
        |u: &User| fields(u).into_iter().flatten().chain([u.id.clone()]).any(|f| f.to_lowercase().contains(q));
    let mut people: Vec<(String, User)> =
        found.into_values().filter(|(_, u)| matches(u)).map(|(c, u)| (c.account_id.clone(), u.clone())).collect();
    let rank = |u: &User| {
        let n = person_name(u).to_lowercase();
        (n.is_empty(), !n.starts_with(|c: char| c.is_ascii_alphabetic()), n)
    };
    people.sort_by_cached_key(|(_, u)| rank(u));
    people
}

/// Desktop's `accountDisplayName`: the nickname of the account (`customName`), or its default name.
pub(crate) fn account_display_name(st: &BeeperStore, account_id: &str, accounts: &[String]) -> String {
    let custom = st.custom_name(account_id).map(str::to_string);
    custom.unwrap_or_else(|| default_account_display_name(st, account_id, accounts))
}

/// Desktop's `defaultAccountDisplayName`: the network, after the user when another account has the same network.
pub(crate) fn default_account_display_name(st: &BeeperStore, account_id: &str, accounts: &[String]) -> String {
    let n = network_label(account_id);
    let user = default_account_user(st, account_id);
    match accounts.iter().filter(|a| network_label(a) == n).count() > 1 && !user.is_empty() {
        true => format!("{user} · {n}"),
        false => n.into(),
    }
}

/// Desktop's `createThreadWithPhoneNumber` identifier: the digits and `+` of a phone number.
pub(crate) fn phone_digits(phone: &str) -> String {
    phone.chars().filter(|c| c.is_ascii_digit() || *c == '+').collect()
}

/// Desktop's draft chat: a chat in the sidebar that the New Chat view makes, with the people it picked.
pub struct NewChat {
    pub id: u32,
    /// The picked people, each with the account to reach them on.
    pub users: Vec<(String, User)>,
    /// Desktop's draft `type: "group"`: from New Group, or more than one person.
    pub group: bool,
    /// The group name of Desktop's `GroupTitleBar`.
    pub title: String,
    /// The group avatar of Desktop's `GroupTitleBar` (`imgURL`): the image, its MIME type and its round picture.
    pub photo: Option<(Vec<u8>, String, crate::Frames)>,
}

impl BeeperGuiApp {
    /// Desktop's `setSelectedChatTarget` with `isDraftEmpty`: leaving a New Chat draft without people, a group name,
    /// a group avatar, a room to join or composer text removes it.
    pub(crate) fn leave_draft(&mut self) {
        let Some(n) = self.creating_chat.take() else { return };
        let joining = self.join_matrix_mode && !self.new_chat_query.trim().is_empty();
        let typed = !self.pane.composer.text().trim().is_empty() || joining;
        if !typed {
            let empty = |d: &NewChat| d.users.is_empty() && d.title.trim().is_empty() && d.photo.is_none();
            self.new_chats.retain(|d| d.id != n || !empty(d));
        }
    }

    pub(crate) fn view_new_chat<'a>(&'a self) -> El<'a> {
        let p = &self.pal;
        if self.join_matrix_mode {
            return self.view_join_matrix();
        }
        let st = self.store.read();
        let draft = self.new_chats.iter().find(|d| Some(d.id) == self.creating_chat);
        let users = draft.map_or(&[][..], |d| &d.users);
        let group = draft.is_some_and(|d| d.group);
        let account = users.first().map(|(a, _)| a.as_str());
        let all = rail_accounts(&st);
        let listed: Vec<String> = match group {
            true => all.iter().filter(|a| self.can_group(a)).cloned().collect(),
            false => all.iter().filter(|a| self.can(a, "create_dm")).cloned().collect(),
        };
        let picked = self.new_chat_account.as_deref();
        let inputs = self.view_new_chat_inputs(&st, users, &listed);
        let col = self.view_new_chat_results(&st, &all, &listed, group, account.or(picked));
        // Desktop's `T$t`.
        let caps = account.map(|a| &self.caps[a]["group_creation"]["group"]);
        let ready = match (group, caps) {
            (false, _) => users.len() == 1,
            (true, None) => false,
            (true, Some(g)) => {
                let fewest = g["participants"]["min_length"]
                    .as_u64()
                    .unwrap_or(u64::from(g["participants"]["required"] == true));
                let most = g["participants"]["max_length"].as_u64().filter(|&m| m > 0).unwrap_or(u64::MAX);
                let named = g["name"]["required"] != true || draft.is_some_and(|d| !d.title.trim().is_empty());
                let pictured = g["avatar"]["required"] != true || draft.is_some_and(|d| d.photo.is_some());
                (fewest..=most).contains(&(users.len() as u64)) && named && pictured
            }
        };
        let start = if group { tr("Start group chat") } else { tr("Start chat") };
        let body = column![
            inputs,
            scrollable(col).direction(bar()).height(Fill).style(scroll_style(p.scroll)),
            new_chat_footer(p, start, "ENTER", ready.then_some(UiMessage::NewChat(new_chat::Msg::Start)))
        ];
        let panel = container(body).width(Fill).height(Fill).style(fill_box(p.card, 18.0));
        let named = caps.is_some_and(|g| g["name"]["allowed"] == true);
        let title = draft.filter(|_| group && named).map(|d| self.view_group_title(d));
        let dropdown = self.new_chat_accounts_open.then(|| self.view_new_chat_accounts(&st, &listed));
        stack![column![].push(title).push(panel).spacing(8)].push(dropdown).into()
    }

    /// Desktop's "Join a Matrix room" view of New Chat: the room address, its error, and Join.
    fn view_join_matrix(&self) -> El<'_> {
        let p = &self.pal;
        let back = button(
            row![tinted(&SYS_ARROW_BACK, 18.0, p.strong), txt(tr("Back"), 12.0).color(p.strong)]
                .spacing(4)
                .align_y(Center),
        )
        .padding([4, 6])
        .style(ghost(p, 6.0))
        .on_press(UiMessage::NewChat(new_chat::Msg::ToggleJoinMatrix));
        let head =
            container(row![back, txt(tr("Join a Matrix room"), 13.0).color(p.strong)].spacing(12).align_y(Center))
                .width(Fill)
                .padding([8, 13])
                .style(fill_box(p.chip, 0.0));
        let (joining, error) = (self.join_room.0, self.join_room.1.as_deref());
        // `.joinInputs .joinInput input`: 32 px tall on the secondary container, body-md text, radius 8.
        let input = text_input(tr("Room address or Matrix link"), &self.new_chat_query)
            .id("new-chat-input")
            .on_input(|q| UiMessage::NewChat(new_chat::Msg::Query(q)))
            .size(rem(14.0))
            .line_height(Pixels(rem(18.0)))
            .padding([7, 10])
            .style(input_style(p.chip, p.strong, p.sub, p.accent, 8.0));
        let input = if joining { input } else { input.on_submit(UiMessage::NewChat(new_chat::Msg::JoinMatrixRoom)) };
        // `.joinError`: under the input, 4 px above and 10 px below.
        let error = error
            .map(|e| container(txt(e.to_string(), 12.0).color(p.text_danger)).padding(Padding::ZERO.top(4).bottom(10)));
        let link = container(tinted(&MAT_LINK, 18.0, p.sub)).padding(Padding::ZERO.top(7));
        let inputs = row![link, column![input].push(error).width(Fill)]
            .spacing(8)
            .padding(Padding::new(6.0).left(13.0).right(8.0));
        let ready = !joining && !self.new_chat_query.trim().is_empty();
        let label = if joining { tr("Joining...") } else { tr("Join") };
        let join =
            new_chat_footer(p, label, "ENTER", ready.then_some(UiMessage::NewChat(new_chat::Msg::JoinMatrixRoom)));
        let body = column![head, inputs].push(Space::new().height(Fill)).push(join);
        container(body).width(Fill).height(Fill).style(fill_box(p.card, 18.0)).into()
    }

    /// The search field of New Chat after the pills of the picked people, and the account picker while it has none.
    fn view_new_chat_inputs(&self, st: &BeeperStore, users: &[(String, User)], listed: &[String]) -> El<'_> {
        let p = &self.pal;
        let picked = self.new_chat_account.as_deref();
        let label = picked.map_or_else(|| tr("All accounts").into(), |a| account_option(st, a, listed));
        let all_acc = button(
            row![]
                .push(picked.map(network_logo))
                .push(txt(label, 12.0).color(p.strong))
                .push(ic(icon::EXPAND_MORE, 16.0).color(p.strong))
                .spacing(4)
                .align_y(Center),
        )
        .height(24)
        .padding(Padding { top: 2.0, right: 4.0, bottom: 2.0, left: 6.0 })
        .style(btn_style([p.chip; 3], [p.strong; 3], 12.0))
        .on_press(UiMessage::NewChat(new_chat::Msg::ToggleAccounts));
        let shade = Color { a: 0.1, ..p.strong };
        let pills = users.iter().map(|(_, u)| -> El<'_> {
            let name = u.full_name.as_deref().unwrap_or(&u.id);
            let pill =
                row![self.face(u.img_url.as_deref(), name, 16.0, 0.0), semi(name.to_string(), 13.0).color(p.strong)];
            container(pill.spacing(4).align_y(Center))
                .padding(Padding::new(2.0).right(8.0))
                .style(fill_box(shade, 20.0))
                .into()
        });
        let placeholder = if users.is_empty() { tr("Search contacts") } else { "" };
        let field = text_input(placeholder, &self.new_chat_query)
            .id("new-chat-input")
            .on_input(|a0| UiMessage::NewChat(new_chat::Msg::Query(a0)))
            .size(rem(13.0))
            .padding([8, 0])
            .style(input_style(CLEAR, p.strong, p.sub, p.accent, 0.0));
        let inputs = Row::with_children(pills).push(field).spacing(4).align_y(Center);
        row![ic(icon::SEARCH, 16.0).color(p.sub), inputs]
            .push(users.is_empty().then_some(all_acc))
            .spacing(8)
            .align_y(Center)
            .padding(Padding::new(6.0).top(10.0).left(13.0).right(8.0))
            .into()
    }

    /// The menu of the account picker of New Chat: All accounts, then the accounts that can start the chat.
    fn view_new_chat_accounts(&self, st: &BeeperStore, listed: &[String]) -> El<'_> {
        let p = &self.pal;
        let picked = self.new_chat_account.as_deref();
        let options = std::iter::once(None).chain(listed.iter().map(|a| Some(a.as_str()))).map(|a| -> El<'_> {
            let label = a.map_or_else(|| tr("All accounts").into(), |a| account_option(st, a, listed));
            let check = (a == picked).then(|| ic(icon::CHECK, 16.0).color(p.strong));
            let line = row![]
                .push(a.map(network_logo))
                .push(ell(&label, 160.0, 13.0, Weight::Normal).color(p.strong).width(Fill))
                .push(check)
                .spacing(8)
                .align_y(Center);
            button(line)
                .width(Fill)
                .height(22)
                .padding([0, 8])
                .style(btn_style([CLEAR, p.chip, p.chip], [p.strong; 3], 4.0))
                .on_press(UiMessage::NewChat(new_chat::Msg::Account(a.map(str::to_string))))
                .into()
        });
        let menu = scrollable(Column::with_children(options.collect::<Vec<_>>())).height(Shrink);
        let menu =
            container(menu).width(216).max_height(300).padding(5).style(shadow_box(p.menu, p.menu_border, 6.0, 22.0));
        stack![
            mouse_area(Space::new().width(Fill).height(Fill))
                .on_press(UiMessage::NewChat(new_chat::Msg::ToggleAccounts)),
            container(menu).align_right(Fill).padding(Padding::new(8.0).top(38.0)),
        ]
        .into()
    }

    /// The results of New Chat: New Group and Join a Matrix room, the looked-up user, the people of the chats, the
    /// users that the accounts found, and the contacts.
    fn view_new_chat_results(
        &self,
        st: &BeeperStore,
        all: &[String],
        listed: &[String],
        group: bool,
        account: Option<&str>,
    ) -> Column<'_, UiMessage> {
        let p = &self.pal;
        let q = self.new_chat_query.trim().to_lowercase();
        let mut col = Column::new().padding(Padding::ZERO.top(3.0));
        if q.is_empty() && !group {
            let (group_btn, join_btn) = self.new_chat_actions(listed);
            col = col.push(group_btn).push(join_btn);
        }
        let lookups: Vec<(&String, &User)> = self
            .new_chat_lookup
            .iter()
            .filter(|(k, nets)| {
                k.strip_prefix("lookup:") == Some(self.new_chat_query.trim()) && nets.iter().all(|n| n.1.is_some())
            })
            .flat_map(|(_, nets)| nets.iter().filter_map(|(a, n)| Some((a, n.as_ref()?.as_ref()?))))
            .collect();
        for (a, u) in &lookups {
            col = col.push(self.view_person(a, u));
        }
        let people = if q.is_empty() { Vec::new() } else { chat_people(st, listed, account, &q) };
        if !people.is_empty() {
            col = col.push(new_chat_heading(p, tr("Chats")));
            let limit = if self.new_chat_chats_expanded { people.len() } else { 10.min(people.len()) };
            for (a, u) in people.iter().take(limit) {
                col = col.push(self.view_person(a, u));
            }
            if people.len() > 10 {
                let label = if self.new_chat_chats_expanded { tr("Show less chats") } else { tr("Show more chats") };
                let more = Some(UiMessage::NewChat(new_chat::Msg::ToggleChats));
                col = col.push(new_chat_item(p, txt(label, 13.0).color(p.text).into(), more));
            }
        }
        let search = self.new_chat_search.as_ref().filter(|(s, ..)| !q.is_empty() && *s == self.new_chat_query.trim());
        if let Some((_, pending, found)) = search {
            col = self.push_connections(col, *pending, found);
        }
        let any = listed.iter().any(|a| self.can(a, "contact_list") || self.can(a, "any_phone"));
        let contacts: Vec<&User> = st
            .contacts
            .iter()
            .filter(|_| any)
            .filter(|u| u.full_name.as_deref().unwrap_or_default().to_lowercase().contains(&q))
            .collect();
        if lookups.is_empty()
            && people.is_empty()
            && search.is_none_or(|(_, n, f)| *n == 0 && f.is_empty())
            && contacts.is_empty()
        {
            // Desktop's "No results found" is only for a query, in a DM draft.
            match group {
                true => col = col.push(new_chat_heading(p, tr("Start typing to search for people to add"))),
                false if !q.is_empty() => col = col.push(new_chat_heading(p, tr("No results found"))),
                false => {}
            }
        }
        if !contacts.is_empty() {
            col = self.push_contacts(col, st, &contacts, all, account, q.is_empty());
        }
        col
    }

    /// The New Group and Join a Matrix room buttons at the top of an empty search.
    fn new_chat_actions(&self, listed: &[String]) -> (Option<El<'_>>, El<'_>) {
        let p = &self.pal;
        let action = |icon: El<'static>, label, bg, msg| {
            button(row![icon, semi(label, 13.0).color(p.strong)].spacing(7).align_y(Center))
                .width(Fill)
                .height(36)
                .padding([0, 11])
                .style(btn_style([bg, p.chip, p.selected], [p.strong; 3], 6.0))
                .on_press(UiMessage::NewChat(msg))
        };
        let group_btn =
            action(ic(icon::GROUP, 20.0).color(p.accent).into(), tr("New Group"), p.chip, new_chat::Msg::NewGroup);
        let join_btn =
            action(tinted(&MAT_LINK, 20.0, p.accent), tr("Join a Matrix room"), CLEAR, new_chat::Msg::ToggleJoinMatrix);
        let group_btn = listed.iter().any(|a| self.can_group(a)).then(|| container(group_btn).padding([0, 8]).into());
        (group_btn, container(join_btn).padding([0, 8]).into())
    }

    /// Desktop's `CommandBarUser`: a person of the New Chat results, with their handles and network.
    fn view_person(&self, a: &str, u: &User) -> El<'_> {
        let p = &self.pal;
        // `.search-participant .small` is 12 px.
        let small = |s: String| crate::style::fixed(s, 12.0, Weight::Normal).color(Color { a: 0.7, ..p.strong });
        let bare = [&u.full_name, &u.username, &u.phone_number, &u.email].iter().all(|f| f.is_none());
        let line = row![self.face(u.img_url.as_deref(), person_name(u), 28.0, 0.0)]
            .push(u.full_name.clone().or_else(|| bare.then(|| u.id.clone())).map(|n| med(n, 13.0).color(p.strong)))
            .push(u.username.as_ref().map(|n| small(format!("@{}", n.trim_start_matches('@')))))
            .push(u.phone_number.as_deref().map(|n| small(fmt_phone(n))))
            .push(u.email.clone().map(small))
            .push(Space::new().width(Fill))
            .push(svg(crate::style::brand_svg(brand_key(a))).width(16).height(16))
            .spacing(6)
            .align_y(Center);
        new_chat_item(
            p,
            line.into(),
            Some(UiMessage::NewChat(new_chat::Msg::PickUser(a.to_string(), Box::new(u.clone())))),
        )
    }

    /// The Connections that the accounts found for the query, in the order of their names.
    fn push_connections<'a>(
        &'a self,
        col: Column<'a, UiMessage>,
        pending: usize,
        found: &People,
    ) -> Column<'a, UiMessage> {
        let p = &self.pal;
        let mut col = col.push(new_chat_heading(p, tr("Connections")));
        let mut seen = HashSet::new();
        let mut results: Vec<&(String, User)> = found.iter().filter(|(a, u)| seen.insert(person_key(a, u))).collect();
        results.sort_by_cached_key(|(_, u)| {
            let n = person_name(u).to_lowercase();
            (n.is_empty(), n)
        });
        let limit = if self.new_chat_results_expanded { results.len() } else { 10.min(results.len()) };
        for (a, u) in results.iter().take(limit).filter(|_| pending == 0) {
            col = col.push(self.view_person(a, u));
        }
        if pending > 0 {
            col = col.push(new_chat_item(p, txt(tr("Searching..."), 13.0).color(p.sub).into(), None));
        } else if results.len() > 10 {
            let label = if self.new_chat_results_expanded { tr("Show less results") } else { tr("Show more results") };
            let more = Some(UiMessage::NewChat(new_chat::Msg::ToggleResults));
            col = col.push(new_chat_item(p, txt(label, 13.0).color(p.text).into(), more));
        }
        col
    }

    /// The contacts, under the headings of their initials, with the networks of the contact that a click opened.
    fn push_contacts<'a>(
        &'a self,
        mut col: Column<'a, UiMessage>,
        st: &BeeperStore,
        contacts: &[&User],
        all: &[String],
        account: Option<&str>,
        blank: bool,
    ) -> Column<'a, UiMessage> {
        let p = &self.pal;
        col = col.push(new_chat_heading(p, tr("In your contacts")));
        let limit = if self.new_chat_expanded || !blank { contacts.len() } else { 10.min(contacts.len()) };
        let mut last_cat = None;
        for u in contacts.iter().take(limit) {
            let name = u.full_name.as_deref().unwrap_or(&u.id);
            let ch = name.chars().next().unwrap_or(' ').to_ascii_uppercase();
            let cat = if ch.is_ascii_alphabetic() { ch.to_string() } else { tr("Others").to_string() };
            if last_cat.as_deref() != Some(cat.as_str()) {
                col = col.push(new_chat_heading(p, &cat));
                last_cat = Some(cat);
            }
            let nets = self.contact_networks.as_ref().filter(|(c, _)| *c == u.id).map(|(_, n)| n);
            let line = row![
                self.face(u.img_url.as_deref(), name, 28.0, 0.0),
                semi(name.to_string(), 13.0).color(p.strong),
                Space::new().width(Fill),
                match nets {
                    Some(_) => tinted(&MAT_EXPAND_LESS, 20.0, p.strong),
                    None => ic(icon::EXPAND_MORE, 20.0).color(p.strong).into(),
                }
            ]
            .spacing(8)
            .align_y(Center);
            let open = Some(UiMessage::NewChat(new_chat::Msg::ToggleContact(u.id.clone())));
            col = col.push(new_chat_item(p, line.into(), open));
            if let Some(nets) = nets {
                col = self.push_contact_networks(col, st, nets, all, account);
            }
        }
        if blank && contacts.len() > 10 {
            let label = if self.new_chat_expanded { tr("Show less contacts") } else { tr("Show more contacts") };
            col = col.push(
                container(
                    button(txt(label, 13.0).color(p.text))
                        .height(36)
                        .padding([0, 6])
                        .width(Fill)
                        .style(ghost(p, 8.0))
                        .on_press(UiMessage::NewChat(new_chat::Msg::ToggleExpanded)),
                )
                .padding([0, 6]),
            );
        }
        col
    }

    /// The networks of an opened contact: skeletons while they resolve, then each account that has the contact.
    fn push_contact_networks<'a>(
        &'a self,
        mut col: Column<'a, UiMessage>,
        st: &BeeperStore,
        nets: &Networks,
        all: &[String],
        account: Option<&str>,
    ) -> Column<'a, UiMessage> {
        let p = &self.pal;
        if nets.iter().any(|(_, n)| n.is_none()) {
            let bone = |w: f32, r: f32| container(Space::new().width(w).height(if r > 8.0 { w } else { 16.0 }));
            let skeleton =
                row![bone(22.0, 11.0).style(fill_box(p.chip, 11.0)), bone(100.0, 5.0).style(fill_box(p.chip, 5.0))];
            return col.push(new_chat_item(p, skeleton.spacing(8).align_y(Center).into(), None));
        }
        let found: Vec<(&String, &User)> = nets
            .iter()
            .filter_map(|(a, n)| Some((a, n.as_ref()?.as_ref()?)))
            .filter(|(a, _)| account.is_none_or(|b| *a == b))
            .collect();
        if found.is_empty() {
            let none = semi(tr("No networks found"), 13.0).color(p.strong).width(Fill).align_x(Center);
            col = col.push(new_chat_item(p, none.into(), None));
        }
        for (a, user) in found {
            let icon = container(svg(crate::style::brand_svg(brand_key(a))).width(20).height(20)).center(28);
            let line = row![icon, med(account_display_name(st, a, all), 13.0).color(p.strong)];
            let pick = UiMessage::NewChat(new_chat::Msg::PickUser(a.clone(), Box::new(user.clone())));
            col = col.push(new_chat_item(p, line.spacing(8).align_y(Center).into(), Some(pick)));
        }
        col
    }

    /// Desktop's `GroupTitleBar` of a group draft: its avatar picker and its name.
    fn view_group_title<'a>(&'a self, d: &'a NewChat) -> Container<'a, UiMessage> {
        let p = &self.pal;
        // Desktop's `ThreadAvatar` of the `GroupTitleBar`: the picked image, the members, or an empty circle.
        let face: El<'_> = match (&d.photo, d.users.is_empty()) {
            (Some((.., f)), _) => self.emoji(f, |h| image(h).width(28).height(28)),
            (None, false) => {
                let members: Vec<User> = d.users.iter().map(|(_, u)| u.clone()).collect();
                self.people_avatar(None, "", true, &members, 28.0, 0.0)
            }
            (None, true) => container(Space::new().width(28).height(28)).style(fill_box(p.chip, 14.0)).into(),
        };
        // Desktop's `PlusIcon`: a 16px badge with a white border, 5px past the right of the avatar.
        let plus = container(svg(PLUS.clone()).width(7).height(8)).center(16).style(|_| container::Style {
            background: Some(p.accent.into()),
            border: iced::border::rounded(8).color(Color::WHITE).width(2),
            ..Default::default()
        });
        let picker = stack![face, container(plus).padding(Padding::ZERO.top(12).left(17))].width(33).height(28);
        let picker = mouse_area(picker)
            .on_press(UiMessage::NewChat(new_chat::Msg::PickAvatar))
            .interaction(mouse::Interaction::Pointer);
        let name = text_input(tr("New Group Chat"), &d.title)
            .on_input(|a0| UiMessage::NewChat(new_chat::Msg::GroupTitle(a0)))
            .width(200)
            .size(rem(13.0))
            .padding(0)
            .style(input_style(CLEAR, p.strong, p.sub, p.accent, 0.0));
        let bg = if self.dark { Color::BLACK } else { Color::WHITE };
        // The badge sticks out 5px of the 8px gap.
        container(row![picker, name].spacing(3).align_y(Center)).center_x(Fill).center_y(38).style(move |_| {
            container::Style {
                background: Some(bg.into()),
                border: iced::border::rounded(10),
                shadow: iced::Shadow {
                    color: Color::from_rgba(0.0, 0.0, 0.0, 0.12),
                    blur_radius: 10.0,
                    ..Default::default()
                },
                ..Default::default()
            }
        })
    }
}

/// The 16 px icon of an account's network.
fn network_logo<'a>(a: &str) -> El<'a> {
    svg(crate::style::brand_svg(brand_key(a))).width(16).height(16).into()
}

/// The footer of New Chat: its Start or Join pill, with the key that presses it, faded while it is off.
fn new_chat_footer<'a>(
    p: &Palette,
    label: &'static str,
    key: &'static str,
    press: Option<UiMessage>,
) -> Row<'a, UiMessage> {
    let a = if press.is_some() { 1.0 } else { 0.4 };
    let label = row![med(label, 13.0), txt(key, 10.0).color(Color { a: 0.8 * a, ..Color::WHITE })];
    let pill = button(label.spacing(4).align_y(Center))
        .padding([4, 8])
        .style(btn_style([Color { a, ..p.accent }; 3], [Color { a, ..Color::WHITE }; 3], 20.0))
        .on_press_maybe(press);
    row![Space::new().width(Fill), pill].padding(Padding::new(8.0).left(13.0)).align_y(Center)
}

/// A heading of the New Chat results.
fn new_chat_heading<'a>(p: &Palette, label: &str) -> Container<'a, UiMessage> {
    container(txt(label.to_string(), 12.0).color(p.sub)).padding(Padding::new(4.0).left(13.0).right(13.0).bottom(2.0))
}

/// A row of the New Chat results.
fn new_chat_item<'a>(p: &Palette, content: El<'a>, press: Option<UiMessage>) -> El<'a> {
    let b = button(content).height(36).padding([0, 6]).width(Fill).style(ghost(p, 8.0));
    container(b.on_press_maybe(press)).padding([0, 6]).into()
}
