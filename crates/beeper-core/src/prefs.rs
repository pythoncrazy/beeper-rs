#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    Toggle(bool),
    Select(&'static [&'static str], u8),
    Slider(u8),
    Archive(&'static Pref, &'static Pref),
    /// Desktop's `AUDIO_INPUT_DEVICE_ID`, a device name in [`crate::models::BeeperSettings::microphone`].
    Microphone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Badge {
    Labs,
    Plus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pref {
    pub key: &'static str,
    pub label: &'static str,
    pub desc: Option<&'static str>,
    /// The values of the `%s` placeholders in the label or the description.
    pub args: &'static [&'static str],
    pub badge: Option<Badge>,
    /// The toggle that must be on for this row to be enabled.
    pub requires: Option<&'static str>,
    pub control: Control,
}

#[derive(Debug, Clone, Copy)]
pub enum Item {
    /// A text with the values of its `%s` placeholders.
    Subtitle(&'static str, &'static [&'static str]),
    Heading(&'static str),
    Group(&'static [Pref]),
    Theme,
}

impl Pref {
    pub fn default_value(&self) -> u8 {
        match self.control {
            Control::Toggle(on) => on as u8,
            Control::Select(_, i) | Control::Slider(i) => i,
            Control::Archive(..) | Control::Microphone => unreachable!("the row {} has no value", self.key),
        }
    }

    pub fn max_value(&self) -> u8 {
        match self.control {
            Control::Toggle(_) => 1,
            Control::Select(options, _) => options.len() as u8 - 1,
            Control::Slider(_) => 100,
            Control::Archive(..) | Control::Microphone => unreachable!("the row {} has no value", self.key),
        }
    }

    const fn desc(self, desc: &'static str) -> Self {
        Pref { desc: Some(desc), ..self }
    }

    const fn args(self, args: &'static [&'static str]) -> Self {
        Pref { args, ..self }
    }

    const fn badge(self, badge: Badge) -> Self {
        Pref { badge: Some(badge), ..self }
    }

    const fn requires(self, key: &'static str) -> Self {
        Pref { requires: Some(key), ..self }
    }
}

const fn t(key: &'static str, label: &'static str, on: bool) -> Pref {
    Pref { key, label, desc: None, args: &[], badge: None, requires: None, control: Control::Toggle(on) }
}

const fn s(key: &'static str, label: &'static str, options: &'static [&'static str], default: u8) -> Pref {
    assert!((default as usize) < options.len());
    Pref { key, label, desc: None, args: &[], badge: None, requires: None, control: Control::Select(options, default) }
}

macro_rules! cmd {
    ($first:literal $(, $rest:literal)*) => {
        if cfg!(target_os = "macos") { concat!($first $(, "⌘", $rest)*) } else { concat!($first $(, "Ctrl", $rest)*) }
    };
}

const AFTER: &[&str] = &["Select next chat", "Select previous chat", "Close chat", "Do nothing"];
pub const COMPOSER_ACTIONS: &[&str] =
    &["Emoji Picker", "Talk to Type", "Record Audio", "GIF Picker", "Add Attachment", "Send Later", "None"];
const SOUND: &[&str] = &["Default", "None"];

pub const LANGUAGES: &[&str] =
    &["System", "English", "Deutsch", "Español", "Français", "日本語", "Português do Brasil"];
pub const ARCHIVE_AGE_DAYS: [i64; 6] = [3, 7, 14, 28, 90, 365];
pub const ARCHIVE_SCOPE: Pref = s("archiveScope", "", &["all", "read"], 0);
pub const ARCHIVE_AGE: Pref = s("archiveAge", "", &["3 days", "7 days", "14 days", "28 days", "3 months", "1 year"], 0);

pub const GENERAL: &[Item] = &[
    Item::Subtitle("These settings are for %s and aren't synced to the messaging platforms.", &["Beeper"]),
    Item::Group(&[
        t("incognito", "Incognito mode", false)
            .badge(Badge::Plus)
            .desc("Incognito mode keeps chats unread even when you click on them and doesn't notify recipients you've read them. Respond or hit %s to mark a chat as read.")
            .args(const { &[cmd!("", " Shift U")] }),
        t("typingIndicators", "Show recipients I'm typing", true).badge(Badge::Plus),
    ]),
    Item::Group(&[s("appLanguage", "App language", LANGUAGES, 0)]),
    Item::Group(&[
        s("remindLaterDefault", "Default for remind later", &["If no reply", "Always"], 0).badge(Badge::Plus),
        t("archiveOnReminder", "Archive after setting reminder on chat", false).badge(Badge::Plus),
    ]),
    Item::Group(&[
        t("markReadOnArchive", "Mark unread chats as read on archive", false),
        s("afterArchive", "After archive", AFTER, 0),
        Pref { key: "archiveOlder", label: "", desc: None, args: &[], badge: None, requires: None, control: Control::Archive(&ARCHIVE_SCOPE, &ARCHIVE_AGE) },
    ]),
    Item::Group(&[
        s("afterMarkRead", "After mark as read/unread", AFTER, 0),
        s(
            "messageDoubleClick",
            "On double-clicking a message",
            &["Do nothing", "Open the reaction picker", "Reply", "Forward", "Show menu", "React with..."],
            2,
        ),
    ]),
    Item::Group(&[
        t("copyTimestamps", "Include timestamps when copying messages with %s", false).args(const { &[cmd!("", " C")] }),
        t("emojiAutocomplete", "Enable emoji autocomplete", true),
        t("autoSendGifs", "Automatically send GIFs after selection", false),
        t("autoPlayVoiceNotes", "Automatically play the next voice note", true),
        Pref { key: "microphone", label: "Microphone", desc: None, args: &[], badge: None, requires: None, control: Control::Microphone },
        t("keepEmoticons", "Disable automatic replacement of emoticons with emojis", false),
        t("autoCopyOtp", "Auto copy OTP/2FA codes", true).desc("If the chat is muted, you'll still be notified."),
        s("upArrow", "Press ↑ in the composer to:", &["Move focus to the message list", "Edit your last message"], 0),
    ]),
    Item::Group(&[
        t("startMinimized", "Keep %s minimized on launch", false).args(&["Beeper"]),
        t("quitOnClose", "Quit %s when closing the main window", false).args(&["Beeper"]),
    ]),
    Item::Group(&[
        t("whatsappCommunities", "Show WhatsApp Communities as separate spaces", false),
        t("telegramTopics", "Show Telegram Topics as separate spaces", false),
    ]),
    Item::Group(&[
        t("hiddenEvents", "Show hidden events in timeline", false),
        s("timeFormat", "Time Format", &["12-hour", "24-hour"], 0),
    ]),
    Item::Group(&[t("shareAnalytics", "Share Analytics", true)
        .desc("Allow us to collect anonymous analytics to help improve the app.")]),
];

pub const APPEARANCE: &[Item] = &[
    Item::Heading("Chat List"),
    Item::Group(&[
        t("showSidebar", "Show sidebar", true),
        t("regularPinned", "Use regular style for pinned chats", false),
        t("showUnreadCount", "Show number of unread messages", true),
        t("showMessagePreviews", "Show message previews", true),
        t("compactChatList", "Compact chat list", false).badge(Badge::Labs),
        t("showNetworkIcons", "Show network icons", true),
        t("colorizeNetworkIcons", "Colorize network icons", true).requires("showNetworkIcons"),
        s(
            "chatActionButton",
            "Chat action button (on hover) should",
            &["Archive", "Mark Read/Unread", "Open Context Menu", "Do Nothing"],
            0,
        ),
        s(
            "chatDoubleClick",
            "On double-clicking a chat",
            &["Open in New Window", "Archive", "Mark Read/Unread", "Mute/Unmute", "Normal/Low Priority", "Do Nothing"],
            0,
        ),
    ]),
    Item::Heading("Conversation View"),
    Item::Group(&[
        t("showTimestamps", "Show message timestamps", true),
        t("hideDeleted", "Hide deleted messages", false),
        t("messageAnimations", "Enable message animations", true),
        t("balanceText", "Balance message text", false),
        t("loadEmbeds", "Load external embeds automatically", true),
        t("quickReactions", "Show quick reactions", true),
        s("quickReactionEmoji", "Quick reaction emoji", &["❤️", "👍", "😂", "😮", "😢", "🔥"], 0)
            .desc("Used for quick react keyboard shortcut (%s)")
            .args(const { &[cmd!("", " Shift R")] }),
        t("reactionsOnSelect", "Show reactions when a message is selected", false),
        t("feedbackWidget", "Show feedback widget", true),
        t("membershipEvents", "Show membership events", true),
    ]),
    Item::Heading("Composer"),
    Item::Group(&[
        t("spellcheck", "Enable spellcheck", true),
        t("formattingMenu", "Show formatting menu", true),
        t("emojiPickerOnHover", "Open emoji picker on hover", true),
        t("noOutgoingLinkPreviews", "Disable link previews in outgoing messages", false),
        s("primaryComposerButton", "Primary composer button", COMPOSER_ACTIONS, 0),
        s("secondaryComposerButton", "Secondary composer button", COMPOSER_ACTIONS, 2),
    ]),
    Item::Group(&[
        t("showTrayIcon", "Show tray icon", true),
        t("pointerCursor", "Use pointer cursor for interactive elements", false),
        s("badgeCount", "Badge count", &["none", "all chats", "unread chats"], 2),
    ]),
    Item::Heading("Theme"),
    Item::Theme,
    Item::Heading("Zoom"),
    Item::Subtitle("You can use %1$s to zoom in and %2$s to zoom out.", const { &[cmd!("", " +"), cmd!("", " -")] }),
];

pub const NOTIFICATIONS: &[Item] = &[
    Item::Heading("General"),
    Item::Group(&[
        s(
            "soundEffects",
            "Sound effects",
            &["Never", "Only for sent messages", "Only for received messages", "Always"],
            0,
        ),
        Pref { key: "soundVolume", label: "Sound effect volume", desc: None, args: &[], badge: None, requires: None, control: Control::Slider(50) },
        t("messageNotifications", "Message notifications", true),
        s("messageSound", "Sound", SOUND, 0),
        t("groupNotifications", "Group message notifications", true),
        t("requestNotifications", "Message request notifications", true),
        s("groupSound", "Sound", SOUND, 0),
        t("notifyInFocus", "Notify when app is in focus", true),
        t("notifyReactions", "Notify when someone reacts to a message", true),
        t("notifyTyping", "Notify when someone starts typing (supported platforms only)", false),
        s(
            "notificationDelay",
            "Delay notifications for",
            &["0 seconds (disabled)", "10 seconds", "30 seconds", "1 minute", "2 minutes", "5 minutes"],
            0,
        )
        .desc("Instead of getting notified for each message, delay and batch notifications for successive texts from someone. OTP/2FA codes are always notified immediately."),
        s("repeatNotification", "Repeat notification if still unread after", &["Never", "2 minutes", "5 minutes", "10 minutes"], 0)
            .desc("If a chat is still unread after this time, its notification is sent one more time."),
    ]),
];

pub const PAGES: [(&str, &[Item]); 3] =
    [("General", GENERAL), ("Appearance", APPEARANCE), ("Notifications", NOTIFICATIONS)];

/// The rows of the settings pages, with the page of each.
pub fn rows() -> impl Iterator<Item = (&'static str, &'static Pref)> {
    PAGES.iter().flat_map(|(page, items)| {
        items.iter().flat_map(|i| if let Item::Group(prefs) = i { *prefs } else { &[] }).map(move |p| (*page, p))
    })
}

pub fn all() -> impl Iterator<Item = (&'static str, &'static Pref)> {
    rows().flat_map(|(page, p)| match p.control {
        Control::Archive(scope, age) => vec![(page, scope), (page, age)],
        _ => vec![(page, p)],
    })
}

/// Desktop's reassignable hotkeys: the name, the group and the default keys. In the keys, ", " separates the
/// combinations, a combination without a key is a modifier for the digits 1 to 9, and "disabled" has no keys.
pub const HOTKEYS: &[(&str, &str, &str)] = &[
    ("Chat: Archive All Read", "Chats", "ctrl+shift+e"),
    ("Chat: Send File", "Chats", "ctrl+o"),
    ("Chat: Start New", "Chats", "ctrl+n"),
    ("Chat: Toggle Archive", "Chats", "ctrl+e, e"),
    ("Chat: Toggle Mark As Read", "Chats", "ctrl+shift+u"),
    ("Chat: Toggle Mute", "Chats", "ctrl+shift+m"),
    ("Chat: Toggle Pin", "Chats", "ctrl+p"),
    ("Cycle Chats (hold shift to reverse)", "Chats", "ctrl+tab"),
    ("History: Next Chat", "Chats", "ctrl+alt+right"),
    ("History: Previous Chat", "Chats", "ctrl+alt+left"),
    ("Open User Profile", "Chats", "ctrl+shift+o"),
    ("Remind Later", "Chats", "ctrl+l"),
    ("Scroll to the bottom of a chat", "Chats", "ctrl+down"),
    ("Select Next Chat", "Chats", "ctrl+], ctrl+alt+down"),
    ("Select Next Unread Chat", "Chats", "ctrl+u"),
    ("Select Previous Chat", "Chats", "ctrl+[, ctrl+alt+up"),
    ("Switch First 9 Chats Modifier", "Chats", "ctrl"),
    ("Toggle Chat Info", "Chats", "ctrl+shift+i"),
    ("Command Bar", "App", "ctrl+j"),
    ("Cycle Filters", "App", "alt+tab"),
    ("Filter: All", "App", "disabled"),
    ("Filter: DMs", "App", "disabled"),
    ("Filter: Drafts", "App", "disabled"),
    ("Filter: Groups", "App", "disabled"),
    ("Filter: Unanswered", "App", "disabled"),
    ("Filter: Unread", "App", "ctrl+shift+y"),
    ("Keyboard Shortcuts", "App", "ctrl+/"),
    ("Minimize To Tray", "App", "ctrl+alt+q"),
    ("Quit App", "App", "ctrl+q"),
    ("Reload App", "App", "ctrl+shift+alt+r"),
    ("Report a Problem", "App", "ctrl+shift+p"),
    ("Reset Zoom", "App", "ctrl+0"),
    ("Search", "App", "ctrl+k"),
    ("Search: Room", "App", "ctrl+f"),
    ("Select Next Account", "App", "ctrl+shift+]"),
    ("Select Previous Account", "App", "ctrl+shift+["),
    ("Send Feedback", "App", "ctrl+alt+/"),
    ("Settings", "App", "ctrl+,"),
    ("Switch First 9 Spaces Modifier", "App", "ctrl+alt"),
    ("Filter By Account", "App", "ctrl+alt+a"),
    ("Toggle Sidebar", "App", "ctrl+alt+s"),
    ("Toggle Space Bar", "App", "ctrl+s"),
    ("Zoom In", "App", "ctrl+="),
    ("Zoom Out", "App", "ctrl+-"),
    ("Download Attachments", "Messages", "ctrl+d"),
    ("Message: Edit", "Messages", "ctrl+t"),
    ("Message: Forward", "Messages", "ctrl+shift+f"),
    ("Message: Open First Link", "Messages", "ctrl+shift+h"),
    ("Message: Quick React", "Messages", "ctrl+shift+r"),
    ("Message: Quote & Reply", "Messages", "ctrl+r"),
    ("Message: Quote or Edit Selected Message", "Messages", "enter"),
    ("Message: Record Audio", "Messages", "ctrl+shift+a"),
    ("Message: Send GIF", "Messages", "ctrl+shift+g"),
    ("Message: Send Message and Archive", "Messages", "ctrl+enter"),
    ("Insert New Line", "Composer", "alt+enter, ctrl+enter, shift+enter"),
    ("Send Message", "Composer", "enter"),
];

pub fn hotkey_default(name: &str) -> &'static str {
    HOTKEYS.iter().find(|h| h.0 == name).unwrap_or_else(|| panic!("unknown hotkey {name}")).2
}

/// The hotkeys that Desktop's Keyboard Shortcuts list shows, but that you cannot reassign.
pub const FIXED_HOTKEYS: &[(&str, &str, &str)] = &[
    ("Message: React", "Messages", "right"),
    ("Multi Select Next Message", "Messages", "shift+down"),
    ("Multi Select Previous Message", "Messages", "shift+up"),
    ("Select Next Message", "Messages", "down"),
    ("Select Previous Message", "Messages", "up"),
    ("Unselect Chat / Select First Chat", "Chats", "escape"),
    ("Bold Text Selection", "Composer", "ctrl+b"),
    ("Format Text Selection as Inline Code", "Composer", "`"),
    ("Italicize Text Selection", "Composer", "ctrl+i"),
    ("Strikethrough Text Selection", "Composer", "ctrl+shift+x"),
];

/// match-sorter's rank of `text` for `query`, which Desktop uses to filter and sort lists: 7 for the same text,
/// down to 1 and a fraction for the letters in order. None does not match.
pub fn match_rank(text: &str, query: &str) -> Option<f32> {
    if query.chars().count() > text.chars().count() {
        return None;
    }
    let (t, q) = (text.to_lowercase(), query.to_lowercase());
    let acronym = || t.split(' ').flat_map(|w| w.split('-')).filter_map(|w| w.chars().next()).collect::<String>();
    Some(match () {
        _ if text == query => 7.0,
        _ if t == q => 6.0,
        _ if t.starts_with(&q) => 5.0,
        _ if t.contains(&format!(" {q}")) => 4.0,
        _ if t.contains(&q) => 3.0,
        _ if q.chars().count() == 1 => return None,
        _ if acronym().contains(&q) => 2.0,
        _ => {
            let t: Vec<char> = t.chars().collect();
            let (mut at, mut first) = (0, None);
            for c in q.chars() {
                let i = at + t[at..].iter().position(|&x| x == c)?;
                first.get_or_insert(i);
                at = i + 1;
            }
            1.0 + 1.0 / (at - 1 - first.expect("a matched letter")) as f32
        }
    })
}

/// The items that match `query`, the best match first and then in alphabetical order, as match-sorter does.
pub fn match_sort<T>(items: impl IntoIterator<Item = T>, query: &str, key: impl Fn(&T) -> String) -> Vec<T> {
    match_sort_keys(items, query, |i| vec![key(i)])
}

/// [`match_sort`] with several keys: the best rank of the keys, then the earliest key, then the matched value.
pub fn match_sort_keys<T>(items: impl IntoIterator<Item = T>, query: &str, keys: impl Fn(&T) -> Vec<String>) -> Vec<T> {
    let best = |i: &T| {
        let ranked = keys(i).into_iter().enumerate().filter_map(|(k, v)| Some((match_rank(&v, query)?, k, v)));
        ranked.max_by(|a, b| a.0.total_cmp(&b.0).then_with(|| b.1.cmp(&a.1)))
    };
    let mut ranked: Vec<_> = items.into_iter().filter_map(|i| Some((best(&i)?, i))).collect();
    ranked.sort_by(|(a, _), (b, _)| {
        b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)).then_with(|| a.2.to_lowercase().cmp(&b.2.to_lowercase()))
    });
    ranked.into_iter().map(|r| r.1).collect()
}

/// Desktop's `di`: the keys as the settings and the menus show them.
pub fn hotkey_label(keys: &str) -> String {
    [
        ("+", " "),
        ("=", "+"),
        ("UP", "↑"),
        ("DOWN", "↓"),
        ("LEFT", "←"),
        ("RIGHT", "→"),
        ("CTRL", "Ctrl"),
        ("SHIFT", "Shift"),
        ("ALT", "Alt"),
        ("ESCAPE", "Escape"),
        ("TAB", "Tab"),
        ("BACKSPACE", "Backspace"),
        ("ENTER", "Enter"),
    ]
    .iter()
    .fold(if keys == "disabled" { String::new() } else { keys.to_uppercase() }, |s, (a, b)| s.replace(a, b))
}

/// Whether `keys` has the combination of the modifiers (Ctrl, Shift and Alt) and the key, which is empty for a
/// modifier-only combination.
pub fn hotkey_has(keys: &str, mods: [bool; 3], key: &str) -> bool {
    keys != "disabled"
        && keys.split(", ").filter(|k| !k.is_empty()).any(|combo| {
            let parts: Vec<&str> = combo.split('+').collect();
            let is_mod = |p: &str| ["ctrl", "shift", "alt"].contains(&p);
            let last = parts.last().copied().filter(|p| !is_mod(p)).unwrap_or("");
            mods == ["ctrl", "shift", "alt"].map(|m| parts.contains(&m)) && last == key
        })
}

pub fn find(key: &str) -> &'static Pref {
    all().map(|(_, p)| p).find(|p| p.key == key).unwrap_or_else(|| panic!("unknown pref {key}"))
}

/// Desktop's global account data with the preferences that sync between devices.
pub const DESKTOP_PREFS: &str = "com.beeper.desktop.prefs";

/// The JSON values that Desktop stores for the options of a preference.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Values {
    Bool,
    Str(&'static [&'static str]),
    /// Seconds, which Desktop writes as strings from its select.
    Secs(&'static [i64]),
    /// A fraction from 0 to 1, for a slider from 0 to 100.
    Volume,
}

/// Where Desktop keeps a preference.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Ad {
    /// A key of [`DESKTOP_PREFS`].
    Desktop(&'static str, Values),
    /// The boolean field of a global account data event.
    Event(&'static str, &'static str),
}

impl Values {
    pub fn decode(self, v: &serde_json::Value) -> Option<u8> {
        let at = |i: Option<usize>| i.map(|i| u8::try_from(i).expect("few options"));
        match self {
            Values::Bool => v.as_bool().map(u8::from),
            Values::Str(values) => at(values.iter().position(|x| v.as_str() == Some(x))),
            Values::Secs(values) => {
                let secs = v.as_i64().or_else(|| v.as_str()?.parse().ok());
                at(values.iter().position(|x| Some(*x) == secs))
            }
            Values::Volume => v.as_f64().filter(|f| (0.0..=1.0).contains(f)).map(|f| (f * 100.0).round() as u8),
        }
    }

    pub fn encode(self, x: u8) -> serde_json::Value {
        match self {
            Values::Bool => (x == 1).into(),
            Values::Str(values) => values[usize::from(x)].into(),
            Values::Secs(values) => values[usize::from(x)].to_string().into(),
            Values::Volume => (f64::from(x) / 100.0).into(),
        }
    }
}

const fn b(key: &'static str) -> Ad {
    Ad::Desktop(key, Values::Bool)
}

const fn v(key: &'static str, values: &'static [&'static str]) -> Ad {
    Ad::Desktop(key, Values::Str(values))
}

const AFTER_VALUES: &[&str] = &["select_next_thread", "select_prev_thread", "close_chat", "none"];
const COMPOSER_VALUES: &[&str] =
    &["emoji", "talk_to_type", "record_audio", "gif_picker", "add_attachment", "send_later", "none"];

/// The preferences that Desktop syncs through account data. The other preferences stay on this device, as in Desktop.
pub const SYNCED: &[(&str, Ad)] = &[
    ("incognito", b("dont_mark_as_read_until_responded")),
    ("typingIndicators", b("show_typing_indicator")),
    ("remindLaterDefault", v("dismiss_reminder_on_incoming_default", &["no_reply", "always"])),
    ("archiveOnReminder", Ad::Event("com.beeper.reminder_settings", "archive_on_reminder_set")),
    ("markReadOnArchive", Ad::Event("com.beeper.mark_read_when_archive", "enabled")),
    ("afterArchive", v("after_archive", AFTER_VALUES)),
    ("afterMarkRead", v("after_toggle_read", AFTER_VALUES)),
    (
        "messageDoubleClick",
        v("message_double_click_action", &["do_nothing", "react", "reply", "forward", "show_menu", "react_with"]),
    ),
    ("copyTimestamps", b("copy_message_timestamps")),
    ("emojiAutocomplete", b("enable_emoji_autocomplete")),
    ("autoSendGifs", b("auto_send_gifs")),
    ("autoPlayVoiceNotes", b("auto_play_next_voice_note")),
    ("keepEmoticons", b("disable_emoticon_replacement")),
    ("autoCopyOtp", b("auto_copy_otp")),
    ("upArrow", v("compose_up_arrow_action", &["focus_message_list", "edit_last_message"])),
    ("whatsappCommunities", Ad::Event("com.beeper.labs.wa_communities_as_spaces", "wa_communities_as_spaces")),
    ("telegramTopics", Ad::Event("com.beeper.labs.tg_topics_as_spaces", "tg_topics_as_spaces")),
    ("shareAnalytics", b("share_analytics")),
    ("showSidebar", b("show_space_switcher")),
    ("regularPinned", b("compact_pinned_threads")),
    ("showUnreadCount", b("show_unreads")),
    ("showMessagePreviews", b("message_previews")),
    ("compactChatList", b("compact_chat_list")),
    ("showNetworkIcons", b("network_icons")),
    ("colorizeNetworkIcons", b("colorize_network_icons_in_chat_list")),
    (
        "chatActionButton",
        v("primary_thread_action", &["archive_unarchive", "mark_read_unread", "open_context_menu", "none"]),
    ),
    (
        "chatDoubleClick",
        v(
            "thread_double_click_action",
            &["open_in_new_window", "archive_unarchive", "mark_read_unread", "mute_unmute", "toggle_priority", "none"],
        ),
    ),
    ("showTimestamps", b("message_timestamps")),
    ("hideDeleted", b("hide_deleted_messages")),
    ("messageAnimations", b("enable_message_animations")),
    ("balanceText", b("balance_message_text")),
    ("loadEmbeds", b("load_iframes_automatically")),
    ("quickReactions", b("quick_reactions_enabled")),
    ("quickReactionEmoji", v("message_double_click_reaction", &["❤️", "👍", "😂", "😮", "😢", "🔥"])),
    ("reactionsOnSelect", b("show_reactions_on_selection")),
    ("feedbackWidget", b("show_send_feedback_button")),
    ("formattingMenu", b("show_formatting_menu")),
    ("emojiPickerOnHover", b("emoji_picker_open_on_hover")),
    ("noOutgoingLinkPreviews", b("disable_link_previews")),
    ("primaryComposerButton", v("primary_composer_action", COMPOSER_VALUES)),
    ("secondaryComposerButton", v("secondary_composer_action", COMPOSER_VALUES)),
    ("pointerCursor", b("interactive_cursor_pointer")),
    ("badgeCount", v("badge_count", &["none", "all", "unread"])),
    ("soundEffects", v("sounds_effects_on_messages", &["never", "on_sent", "on_received", "always"])),
    ("soundVolume", Ad::Desktop("sound_effect_volume", Values::Volume)),
    ("messageNotifications", b("message_notifications")),
    ("messageSound", v("notification_sound_name", &["Default", "None"])),
    ("groupNotifications", b("group_message_notifications")),
    ("requestNotifications", b("message_request_notifications")),
    ("groupSound", v("notification_sound_name_group", &["Default", "None"])),
    ("notifyInFocus", b("notify_in_focus")),
    ("notifyReactions", b("notify_for_reactions")),
    ("notifyTyping", b("show_typing_notifications")),
    ("notificationDelay", Ad::Desktop("debounce_notifications", Values::Secs(&[0, 10, 30, 60, 120, 300]))),
    ("repeatNotification", Ad::Desktop("renotify_unread_delay", Values::Secs(&[0, 120, 300, 600]))),
];

pub fn synced(key: &str) -> Option<Ad> {
    SYNCED.iter().find(|(k, _)| *k == key).map(|(_, a)| *a)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synced_prefs_cover_every_option() {
        for (key, ad) in SYNCED {
            let p = find(key);
            let n = match ad {
                Ad::Desktop(_, Values::Str(v)) => v.len(),
                Ad::Desktop(_, Values::Secs(v)) => v.len(),
                Ad::Desktop(_, Values::Bool) | Ad::Event(..) => 2,
                Ad::Desktop(_, Values::Volume) => 101,
            };
            assert_eq!(n, usize::from(p.max_value()) + 1, "{key}");
            let Ad::Desktop(_, values) = ad else { continue };
            for x in 0..=p.max_value() {
                assert_eq!(values.decode(&values.encode(x)), Some(x), "{key}");
            }
        }
    }

    #[test]
    fn hotkeys_match_desktop_keys() {
        let next = hotkey_default("Select Next Chat");
        assert!(hotkey_has(next, [true, false, false], "]") && hotkey_has(next, [true, false, true], "down"));
        assert!(!hotkey_has(next, [true, true, false], "]"), "Ctrl Shift ] is Select Next Account");
        assert!(hotkey_has(hotkey_default("Reload App"), [true, true, true], "r"));
        assert!(hotkey_has("ctrl+alt", [true, false, true], "") && !hotkey_has("ctrl+alt", [true, false, false], ""));
        assert!(!hotkey_has("disabled", [false; 3], "") && !hotkey_has("", [false; 3], ""));
        assert_eq!(hotkey_label("ctrl+shift+alt+r"), "Ctrl Shift Alt R");
        assert_eq!(hotkey_label("ctrl+="), "Ctrl +");
        assert_eq!(hotkey_label("ctrl+], ctrl+alt+down"), "Ctrl ], Ctrl Alt ↓");
        assert_eq!(hotkey_label("disabled"), "");
        for (i, (name, group, keys)) in HOTKEYS.iter().enumerate() {
            assert!(["Chats", "App", "Messages", "Composer"].contains(group), "{name}");
            assert!(HOTKEYS[..i].iter().all(|h| h.0 != *name), "{name} is unique");
            let combos = || keys.split(", ").filter(|_| *keys != "disabled");
            for (other, g, k) in HOTKEYS[..i].iter().filter(|h| (h.1 == "Composer") == (*group == "Composer")) {
                assert!(combos().all(|c| !k.split(", ").any(|x| x == c)), "{name} and {other} ({g}) share {keys}");
            }
        }
    }

    #[test]
    fn match_sort_ranks_like_match_sorter() {
        let names = ["Zoom In", "Search: Room", "Search", "Reset Zoom", "Send Feedback", "Select Next Chat"];
        assert_eq!(match_sort(names, "zoom", |n| n.to_string()), ["Zoom In", "Reset Zoom"]);
        let sea = ["Search", "Search: Room", "Send Feedback", "Select Next Chat"];
        assert_eq!(match_sort(names, "sea", |n| n.to_string()), sea, "the closer letters first");
        assert_eq!(match_sort(names, "snc", |n| n.to_string()), ["Select Next Chat", "Send Feedback"], "an acronym");
        assert_eq!(match_sort(names, "sfk", |n| n.to_string()), ["Send Feedback"], "letters in order");
        assert_eq!(match_sort(names, "", |n| n.to_string())[..2], ["Reset Zoom", "Search"]);
        assert_eq!(match_rank("Search", "Search"), Some(7.0));
        assert_eq!(match_rank("abcd", "ad"), Some(1.0 + 1.0 / 3.0));
        assert_eq!(match_rank("ab", "x"), None);
    }
}
