//! Desktop's `beeper://` links (`deep-link-CFIhWY4D.js` `Ld`): from the launch arguments, and from a later launch.
use crate::chat_list;
use crate::composer;
use crate::encode_uri_component;
use crate::i18n::tr;
use crate::prelude::*;
use beeper_core::codec::url_decode;

pub const SCHEME: &str = "beeper://";

/// The argument that carries the `XDG_ACTIVATION_TOKEN` of a later launch to the running app, so that the app can
/// raise its window with it on Wayland.
pub const ACTIVATION_TOKEN_ARG: &str = "--xdg-activation-token=";

/// The networks whose Desktop platform name (`platform.name` without `bridge-`, Desktop's `Uae`) is not the
/// beeper-rs bridge ID.
const PLATFORMS: [(&str, &str); 7] = [
    ("discord", "discordgo"),
    ("slack", "slackgo"),
    ("tumblr", "tumblrdms"),
    ("facebook", "facebookgo"),
    ("instagram", "instagramgo"),
    ("imessage", "imessagecloud"),
    ("bluesky", "local-bluesky"),
];

fn platform(n: &NetworkSpec) -> &'static str {
    PLATFORMS.iter().find(|p| p.0 == n.bridge_id).map_or(n.bridge_id, |p| p.1)
}

/// The link of a chat, Desktop's "Copy Chat Link".
pub fn chat_link(account_id: &str, chat_id: &str) -> String {
    format!("{SCHEME}select-thread/{}/{}", platform(account_network(account_id)), encode_uri_component(chat_id))
}

/// Desktop's "Copy chat deep link" command: the platform name with its `bridge-` and the chat ID as they are, then
/// the account.
pub fn chat_deep_link(account_id: &str, chat_id: &str) -> String {
    format!("{SCHEME}select-thread/bridge-{}/{chat_id}?accountID={account_id}", platform(account_network(account_id)))
}

/// Desktop's `decodeURIComponent`, which keeps `+`.
fn decode_uri_component(s: &str) -> Option<String> {
    url_decode(&s.replace('+', "%2B"))
}

/// Desktop's `ag`: the path parts and the query of a link, or `None` when a part does not decode. The parts of
/// `join-matrix-room` stay encoded.
fn parse(link: &str) -> Option<(Vec<String>, HashMap<String, String>)> {
    let rest = link.strip_prefix(SCHEME).expect("a beeper:// link");
    let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
    let query = query.split('&').filter(|p| !p.is_empty()).map(|p| p.split_once('=').unwrap_or((p, "")));
    let query = query.map(|(k, v)| Some((url_decode(k)?, url_decode(v)?))).collect::<Option<_>>()?;
    let parts = path.split('/').map(str::to_string);
    let parts = match path.split('/').next() == Some("join-matrix-room") {
        true => parts.collect(),
        false => parts.map(|p| decode_uri_component(&p)).collect::<Option<_>>()?,
    };
    Some((parts, query))
}

/// Desktop's `registerLinuxConfig` desktop entry, as Beeper-RS. Its name is the install ID, which the window and the
/// launcher badge use.
#[cfg(target_os = "linux")]
fn desktop_entry(exe: &str, icon: &str) -> String {
    use beeper_core::config::{INSTALL_ID, INSTALL_NAME};
    format!(
        "[Desktop Entry]\nName={INSTALL_NAME}\nExec={} %u\nType=Application\nTerminal=false\n\
         Comment=The ultimate messaging app\nCategories=Network;Chat;\nMimeType=x-scheme-handler/beeper;\n\
         StartupWMClass={INSTALL_ID}\nStartupNotify=true\nIcon={icon}\n",
        exec_program(exe)
    )
}

/// The program of the `Exec` key: as it is, like Desktop's, unless it has a character that the desktop entry spec
/// reserves, which needs quotes. xdg-open without a desktop that it knows (`open_generic`, as under sway or i3) runs
/// the first word of `Exec` as it is, so it cannot start a quoted program.
#[cfg(target_os = "linux")]
fn exec_program(exe: &str) -> String {
    let reserved = |c: char| c.is_whitespace() || "\"'\\><~|&;$*?#()`".contains(c);
    match exe.contains(reserved) {
        true => format!("\"{exe}\""),
        false => exe.to_string(),
    }
}

/// Removes the entry that older builds installed in `apps` under Desktop's name, which hid Desktop's own entry. Only
/// an entry with the beeper-rs `icon` goes, so Desktop's entry stays.
#[cfg(target_os = "linux")]
fn drop_old_entry(apps: &std::path::Path, icon: &std::path::Path) -> Result<(), String> {
    let old = apps.join(format!("{}.desktop", beeper_core::config::APP_BUNDLE_ID));
    match std::fs::read_to_string(&old) {
        Ok(e) if e.contains(&format!("\nIcon={}\n", icon.display())) => {
            std::fs::remove_file(&old).map_err(|e| format!("remove {}: {e}", old.display()))
        }
        _ => Ok(()),
    }
}

/// Desktop's `registerLinuxConfig`: installs the desktop entry and the icon, and makes the entry the default handler
/// of `beeper://` links.
#[cfg(target_os = "linux")]
pub fn register() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let quotable = exe.to_str().filter(|e| !e.contains(['"', '`', '$', '\\']));
    let exe = quotable.ok_or(format!("the program path {} has characters that Exec cannot quote", exe.display()))?;
    let home = std::path::PathBuf::from(std::env::var_os("HOME").ok_or("HOME is not set")?);
    let data = std::env::var_os("XDG_DATA_HOME").map_or_else(|| home.join(".local/share"), Into::into);
    let name = format!("{}.desktop", beeper_core::config::INSTALL_ID);
    let (entry, icon) = (data.join("applications").join(&name), data.join("icons/beeper-rs.png"));
    drop_old_entry(&data.join("applications"), &icon)?;
    let write = |path: &std::path::Path, bytes: &[u8]| {
        let dir = path.parent().expect("a file in a folder");
        let done = std::fs::create_dir_all(dir).and_then(|()| std::fs::write(path, bytes));
        done.map_err(|e| format!("write {}: {e}", path.display()))
    };
    write(&icon, include_bytes!("../assets/icon.png"))?;
    write(&entry, desktop_entry(exe, &icon.to_string_lossy()).as_bytes())?;
    let mime = std::process::Command::new("xdg-mime").args(["default", &name, "x-scheme-handler/beeper"]).status();
    match mime.map_err(|e| format!("xdg-mime: {e}"))?.success() {
        true => Ok(()),
        false => Err("xdg-mime did not set the beeper:// handler".into()),
    }
}

impl BeeperGuiApp {
    /// Desktop's `second-instance` event: opens the last argument as a link, and shows the main window.
    pub(crate) fn second_instance(&mut self, mut args: Vec<String>) -> Task<UiMessage> {
        // The later launch passes its launcher's activation token, which raises the window on Wayland.
        let token = args.iter().position(|a| a.starts_with(ACTIVATION_TOKEN_ARG)).map(|i| args.remove(i));
        if let Some(token) = token {
            self.activation_token = Some(token[ACTIVATION_TOKEN_ARG.len()..].to_owned()).filter(|t| !t.is_empty());
        }
        let link = args.last().map_or_else(Task::none, |l| self.open_link(l));
        Task::batch([link, self.show_main()])
    }

    /// Shows the main window, or the sign-in window while signed out, also after a close hid it.
    fn show_main(&mut self) -> Task<UiMessage> {
        self.set_hidden(false)
    }

    /// Desktop's `handleDeepLink`.
    pub(crate) fn open_link(&mut self, link: &str) -> Task<UiMessage> {
        if link == SCHEME || !link.starts_with(SCHEME) {
            return Task::none();
        }
        let show = self.show_main();
        Task::batch([show, self.follow(link)])
    }

    /// Desktop's `joinMatrixRoomFromInput` (`Wh`) of the Join a Matrix room view: the address is checked first, a
    /// known room opens, and other rooms are joined through the servers of the link.
    pub(crate) fn join_matrix_room(&mut self) -> Task<UiMessage> {
        use beeper_core::matrix::{room_address, BadAddress};
        let (room, vias) = match room_address(&self.new_chat_query) {
            Ok(target) => target,
            Err(reason) => {
                self.join_room.1 = Some(tr(match reason {
                    BadAddress::Empty => "Enter a Matrix room address or link.",
                    BadAddress::MixedSigil => "This starts with both # and !, so it isn't a valid Matrix address. Use a room address like #room:server, or paste a Matrix link for a room ID.",
                    BadAddress::Invalid => "Enter a valid Matrix room address like #room:server, or paste a Matrix room link.",
                }).into());
                return Task::none();
            }
        };
        let draft = self.creating_chat;
        if self.store.read().resolve_chat_idx(&room).is_some() {
            return self.open_joined_room(draft, &room);
        }
        let unrouted = room.starts_with('!') && vias.is_empty();
        self.join_room = (true, None);
        let join = beeper_core::matrix::join_via(self.store.clone(), room, vias);
        Task::perform(join, move |r| UiMessage::NewChat(new_chat::Msg::MatrixRoomJoined(draft, r, unrouted)))
    }

    /// The end of a join of the Join a Matrix room view of the `draft`: Desktop opens the room and drops the draft,
    /// or shows why it could not in the view, or in a toast once the user has left the view.
    pub(crate) fn matrix_room_joined(&mut self, draft: Option<u32>, r: Result<String, String>, unrouted: bool) -> Task<UiMessage> {
        self.join_room.0 = false;
        let here = self.join_matrix_mode && self.creating_chat == draft;
        let error = match r {
            Ok(id) if here && self.store.read().resolve_chat_idx(&id).is_some() => return self.open_joined_room(draft, &id),
            Ok(_) if here => "You joined the room, but Beeper couldn't open it yet. Try again.",
            Ok(_) => "The Matrix room was joined, but it was not opened.",
            Err(e) => {
                eprintln!("Failed to join Matrix room: {e}");
                match unrouted {
                    true => "Couldn't join this Matrix room without routing information. Try a link with routing information, or check your access.",
                    false => "Couldn't join this Matrix room. Check the address and your access, then try again.",
                }
            }
        };
        match here {
            true => self.join_room.1 = Some(tr(error).into()),
            false => self.show(Toast::new(tr(error).into())),
        }
        Task::none()
    }

    fn open_joined_room(&mut self, draft: Option<u32>, room: &str) -> Task<UiMessage> {
        self.new_chats.retain(|d| Some(d.id) != draft);
        self.new_chat_query.clear();
        self.select_chat(room);
        Task::none()
    }

    /// Desktop's `joinMatrixRoom` result for a link.
    pub(crate) fn room_joined(&mut self, r: Result<String, String>) {
        let text = match r {
            Ok(id) if self.store.read().resolve_chat_idx(&id).is_some() => return self.select_chat(&id),
            Ok(_) => tr("You joined the room, but Beeper couldn't open it yet. Try again."),
            Err(e) => {
                eprintln!("Failed to join Matrix room: {e}");
                tr("Couldn't join this Matrix room. Check the address and your access, then try again.")
            }
        };
        self.show(Toast::new(text.into()));
    }

    fn follow(&mut self, link: &str) -> Task<UiMessage> {
        let toast = |app: &mut Self, text: String| {
            app.show(Toast::new(text));
            Task::none()
        };
        let Some((parts, query)) = parse(link) else { return toast(self, "Invalid deep link".into()) };
        let (text, attachment) = (query.get("text").cloned(), query.get("attachmentPath").cloned());
        let part = |i: usize| parts.get(i).map(String::as_str);
        match part(0).expect("split gives a part") {
            "refresh-user" if self.main_window.is_some() && !self.store.read().config.demo => {
                return self.refresh_cloud_accounts();
            }
            "refresh-user" => return Task::none(),
            "fill-textarea" => {
                if let (Some(t), Some(_)) = (text, &self.pane.selected_chat_id) {
                    self.pane.composer = composer_with(&t);
                }
                return iced::widget::operation::focus("composer-input");
            }
            "join-matrix-room" => {
                let Some(room) = part(1).filter(|r| !r.is_empty()) else {
                    return toast(self, tr("No Matrix room ID was provided in the link.").into());
                };
                let Some(room) = decode_uri_component(room) else {
                    return toast(self, tr("The link contains an invalid Matrix room ID.").into());
                };
                if self.store.read().resolve_chat_idx(&room).is_some() {
                    self.select_chat(&room);
                    return Task::none();
                }
                let join = beeper_core::matrix::join(self.store.clone(), room);
                return Task::perform(join, UiMessage::RoomJoined);
            }
            "otp" => {
                let code = part(1).unwrap_or_default().to_string();
                return if self.login_window.is_some() { self.update_login(LoginMsg::Code(code)) } else { Task::none() };
            }
            "focus" => {
                let id = query.get("windowID").map_or("main", String::as_str);
                let thread = id.strip_prefix("thread-");
                let window = match id {
                    "main" => None,
                    "prefs" => self.settings_window,
                    _ if let Some(space) = id.strip_prefix("space-") => self.space_windows.iter().find(|w| w.1 == space).map(|w| w.0),
                    _ => self
                        .chat_windows
                        .iter()
                        .find(|w| w.1.selected_chat_id.as_deref() == thread && !self.is_space_window(w.0))
                        .map(|w| w.0),
                };
                return window.map_or_else(Task::none, |w| self.raise(w));
            }
            "connect" => return self.open_settings(SettingsTab::Integrations),
            // Desktop's `subscriptionStore.handleDeepLink`, which opens the subscription info. Desktop then reads the
            // action as a platform, and finds none.
            "subscriptions" if part(1) == Some("openSubscriptionInfo") => {
                let open = self.open_settings(SettingsTab::BeeperPlus);
                return Task::batch([open, toast(self, "Invalid deep link".into())]);
            }
            "tips" if part(1) == Some("add-account") && parts.len() == 2 => {
                return self.update(UiMessage::Networks(crate::networks::Msg::Open));
            }
            _ => {}
        }
        let name = part(1).map(|m| m.strip_prefix("bridge-").unwrap_or(m));
        let Some(network) = name.and_then(|m| NETWORK_SPECS.iter().find(|n| platform(n) == m)) else {
            return toast(self, "Invalid deep link".into());
        };
        let st = self.store.read();
        let account = query.get("accountID").filter(|a| st.accounts.iter().any(|x| x.account_id == **a)).cloned();
        let accounts: Vec<String> = match account {
            Some(a) => vec![a],
            None => {
                let on = |a: &String| find_network(a).is_some_and(|n| n.bridge_id == network.bridge_id);
                st.accounts.iter().map(|a| a.account_id.clone()).filter(on).collect()
            }
        };
        if accounts.is_empty() {
            drop(st);
            return toast(self, format!("Could not find any {} accounts", network.display_name));
        }
        let thread = part(2).and_then(|id| st.resolve_chat_idx(id)).filter(|&i| accounts.contains(&st.chats[i].account_id));
        let thread = thread.map(|i| (st.chats[i].id.clone(), st.chats[i].is_request == Some(true)));
        let at_key = |key: &str| {
            let (cid, _) = thread.as_ref()?;
            Some(st.stored("chat_id = ?1 AND sort_key = ?2", [cid.as_str(), key]).pop()?.id)
        };
        let sorted = part(3).and_then(at_key);
        drop(st);
        let missing = format!("Could not find {} thread", network.display_name);
        match part(0).expect("split gives a part") {
            "compose" => {
                let (kind, value) = (part(2).unwrap_or_default(), part(3).unwrap_or_default().to_string());
                let user = match kind {
                    "id" | "username" | "email" => User { id: value, ..User::default() },
                    "phone" => User { id: value.clone(), phone_number: Some(value), ..User::default() },
                    _ => return toast(self, format!("Could not compose {} chat", network.display_name)),
                };
                self.open_dm(accounts[0].clone(), user, None, text)
            }
            "select-thread" => {
                let Some((cid, request)) = thread else { return toast(self, missing) };
                if request {
                    let _ = self.update(UiMessage::ChatList(chat_list::Msg::SelectTab(InboxTab::Requests)));
                }
                self.select_chat(&cid);
                if let Some(t) = text {
                    self.pane.composer = composer_with(&t);
                }
                let jump = sorted.map_or_else(Task::none, |mid| self.update(UiMessage::CommandBar(commands::Msg::JumpTo(cid, mid))));
                let files = attachment.map(|p| self.update(UiMessage::Composer(composer::Msg::FilesPicked(Ok(vec![p.into()])))));
                Task::batch([jump].into_iter().chain(files))
            }
            "highlight-message" => {
                let Some((cid, _)) = thread else { return toast(self, missing) };
                let mid = part(3).unwrap_or_default().to_string();
                self.update(UiMessage::CommandBar(commands::Msg::JumpTo(cid, mid)))
            }
            _ => toast(self, format!("Unrecognized deep link: {link}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_parse_like_desktop() {
        let (parts, query) = parse("beeper://select-thread/whatsapp/%21a%3Ab.c/k?text=hi+there&accountID=x").unwrap();
        assert_eq!(parts, ["select-thread", "whatsapp", "!a:b.c", "k"]);
        assert_eq!((query["text"].as_str(), query["accountID"].as_str()), ("hi there", "x"));
        assert_eq!(parse("beeper://join-matrix-room/%23a%3Ab.c").unwrap().0[1], "%23a%3Ab.c");
        assert!(parse("beeper://select-thread/%zz").is_none());
        let n = |id| NETWORK_SPECS.iter().find(|n| n.bridge_id == id).unwrap();
        assert_eq!([platform(n("whatsapp")), platform(n("facebook")), platform(n("hungryserv"))], ["whatsapp", "facebookgo", "hungryserv"]);
        assert_eq!(chat_link("acc_instagram", "!r:beeper.com"), "beeper://select-thread/instagramgo/!r%3Abeeper.com");
        #[cfg(target_os = "linux")]
        assert!(desktop_entry("/a b/beeper-rs", "/i.png").contains("\nExec=\"/a b/beeper-rs\" %u\n"));
        #[cfg(target_os = "linux")]
        assert!(
            desktop_entry("/opt/beeper-rs/beeper-rs", "/i.png").contains("\nExec=/opt/beeper-rs/beeper-rs %u\n"),
            "xdg-open's generic mode runs the first word of Exec, so a plain path has no quotes, like Desktop's"
        );
    }

    /// beeper-rs installs as Beeper-RS next to Desktop, and takes back the entry that older builds wrote under
    /// Desktop's name, but not Desktop's own entry.
    #[cfg(target_os = "linux")]
    #[test]
    fn installs_as_beeper_rs_next_to_desktop() {
        let entry = desktop_entry("/b/beeper-rs", "/d/icons/beeper-rs.png");
        assert!(entry.contains("\nName=Beeper-RS\n") && entry.contains("\nStartupWMClass=beeper-rs\n"), "{entry}");
        let apps = beeper_core::testing::temp_dir("old-entry");
        let old = apps.join("com.automattic.beeper.desktop.desktop");
        let icon = std::path::Path::new("/d/icons/beeper-rs.png");
        std::fs::write(&old, "[Desktop Entry]\nName=Beeper\nIcon=beeper-desktop\n").unwrap();
        drop_old_entry(&apps, icon).unwrap();
        assert!(old.exists(), "Desktop's own entry stays");
        std::fs::write(&old, entry.replace("Beeper-RS", "Beeper")).unwrap();
        drop_old_entry(&apps, icon).unwrap();
        assert!(!old.exists(), "the entry of an older build goes");
        drop_old_entry(&apps, icon).unwrap();
    }

    #[test]
    fn links_open_like_desktop() {
        let mut app = crate::fuzz::demo_app("deeplink");
        let (cid, account, mid) = {
            let st = app.store.read();
            let c = st.chats.iter().find(|c| c.account_id == "acc_whatsapp").expect("a WhatsApp chat");
            let first = st.timeline_indices(&c.id).first().expect("the chat has messages");
            (c.id.clone(), c.account_id.clone(), st.messages[*first].id.clone())
        };
        let open = |app: &mut BeeperGuiApp, link: &str| {
            app.status_banner.clear();
            let _ = app.open_link(link);
            app.status_banner.clone()
        };
        let _ = app.update(UiMessage::RunHotkey("Minimize To Tray"));
        assert_eq!(open(&mut app, &format!("{}?text=hi", chat_link(&account, &cid))), "");
        assert!(!app.hidden, "a link shows the main window");
        assert_eq!(app.pane.selected_chat_id.as_deref(), Some(cid.as_str()), "the copied link opens its chat");
        assert_eq!(app.pane.composer.text(), "hi");
        let _ = app.update(UiMessage::SelectChat(crate::NOTE_CHAT_ID.into()));
        assert_eq!(open(&mut app, &format!("beeper://highlight-message/bridge-whatsapp/{cid}/{mid}")), "");
        assert_eq!(app.flash.as_ref().map(|f| f.0.as_str()), Some(mid.as_str()));
        let cases = [
            ("beeper://select-thread/%zz", "Invalid deep link"),
            ("beeper://select-thread/nonet/x", "Invalid deep link"),
            ("beeper://select-thread/whatsapp/nope", "Could not find WhatsApp thread"),
            ("beeper://select-thread/linkedin/x", "Could not find any LinkedIn accounts"),
            ("beeper://subscriptions/whatsapp", "Unrecognized deep link: beeper://subscriptions/whatsapp"),
            ("beeper://join-matrix-room", "No Matrix room ID was provided in the link."),
            ("beeper://join-matrix-room/%zz", "The link contains an invalid Matrix room ID."),
            ("https://example.com", ""),
        ];
        for (link, toast) in cases {
            assert_eq!(open(&mut app, link), toast, "{link}");
        }
        assert_eq!(open(&mut app, "beeper://connect"), "");
        assert_eq!(app.active_settings_tab, SettingsTab::Integrations);
        assert_eq!(open(&mut app, "beeper://subscriptions/openSubscriptionInfo"), "Invalid deep link", "as Desktop goes on");
        assert_eq!(app.active_settings_tab, SettingsTab::BeeperPlus, "the subscription info");
        let _ = open(&mut app, "beeper://tips/add-account");
        assert!(app.children.find(|w| matches!(w, Win::Networks)).is_some(), "the link opens the Add an Account window");
    }
}
