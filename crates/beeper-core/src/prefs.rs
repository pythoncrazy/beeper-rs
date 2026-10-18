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
    pub badge: Option<Badge>,
    /// The toggle that must be on for this row to be enabled.
    pub requires: Option<&'static str>,
    pub control: Control,
}

#[derive(Debug, Clone, Copy)]
pub enum Item {
    Subtitle(&'static str),
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

    const fn badge(self, badge: Badge) -> Self {
        Pref { badge: Some(badge), ..self }
    }

    const fn requires(self, key: &'static str) -> Self {
        Pref { requires: Some(key), ..self }
    }
}

const fn t(key: &'static str, label: &'static str, on: bool) -> Pref {
    Pref { key, label, desc: None, badge: None, requires: None, control: Control::Toggle(on) }
}

const fn s(key: &'static str, label: &'static str, options: &'static [&'static str], default: u8) -> Pref {
    assert!((default as usize) < options.len());
    Pref { key, label, desc: None, badge: None, requires: None, control: Control::Select(options, default) }
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
pub const ZOOM_HINT: &str = cmd!("You can use ", " + to zoom in and ", " - to zoom out.");
pub const ARCHIVE_AGE_DAYS: [i64; 6] = [1, 3, 7, 14, 30, 90];
pub const ARCHIVE_SCOPE: Pref = s("archiveScope", "Archive", &["all", "read"], 0);
pub const ARCHIVE_AGE: Pref =
    s("archiveAge", "chats older than", &["1 day", "3 days", "1 week", "2 weeks", "1 month", "3 months"], 1);

pub const GENERAL: &[Item] = &[
    Item::Subtitle("These settings are for Beeper and aren't synced to the messaging platforms."),
    Item::Group(&[
        t("incognito", "Incognito mode", false).badge(Badge::Plus).desc(cmd!(
            "Incognito mode keeps chats unread even when you click on them and doesn't notify recipients you've read them. Respond or hit ",
            " Shift U to mark a chat as read.\n\nRequires a paid subscription."
        )),
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
        Pref { key: "archiveOlder", label: "", desc: None, badge: None, requires: None, control: Control::Archive(&ARCHIVE_SCOPE, &ARCHIVE_AGE) },
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
        t("copyTimestamps", cmd!("Include timestamps when copying messages with ", " C"), false),
        t("emojiAutocomplete", "Enable emoji autocomplete", true),
        t("autoSendGifs", "Automatically send GIFs after selection", false),
        t("autoPlayVoiceNotes", "Automatically play the next voice note", true),
        Pref { key: "microphone", label: "Microphone", desc: None, badge: None, requires: None, control: Control::Microphone },
        t("keepEmoticons", "Disable automatic replacement of emoticons with emojis", false),
        t("autoCopyOtp", "Auto copy OTP/2FA codes", true).desc("If the chat is muted, you'll still be notified."),
        s("upArrow", "Press ↑ in the composer to:", &["Move focus to the message list", "Edit your last message"], 0),
    ]),
    Item::Group(&[
        t("startMinimized", "Keep Beeper minimized on launch", false),
        t("quitOnClose", "Quit Beeper when closing the main window", false),
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
            .desc(cmd!("Used for quick react keyboard shortcut (", " Shift R)")),
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
    Item::Subtitle(ZOOM_HINT),
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
        Pref { key: "soundVolume", label: "Sound effect volume", desc: None, badge: None, requires: None, control: Control::Slider(50) },
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

pub const HOTKEYS: &[(&str, &[(&str, &str)])] = &[
    (
        "Chats",
        &[
            ("Chat: Toggle Archive", "Ctrl E"),
            ("Chat: Toggle Mark As Read", "Ctrl Shift U"),
            ("Chat: Toggle Mute", "Ctrl Shift M"),
            ("Chat: Toggle Pin", "Ctrl P"),
            ("Select Next Chat", "Ctrl ], Ctrl Alt ↓"),
            ("Select Previous Chat", "Ctrl [, Ctrl Alt ↑"),
            ("Toggle Chat Info", "Ctrl Shift I"),
        ],
    ),
    (
        "App",
        &[
            ("Filter: Unread", "Ctrl Shift Y"),
            ("Quit App", "Ctrl Q"),
            ("Report a Problem", "Ctrl Shift P"),
            ("Reset Zoom", "Ctrl 0"),
            ("Search", "Ctrl K"),
            ("Search: Room", "Ctrl F"),
            ("Settings", "Ctrl ,"),
            ("Zoom In", "Ctrl ="),
            ("Zoom Out", "Ctrl -"),
        ],
    ),
];

pub fn assert_hotkey(name: &str) {
    assert!(HOTKEYS.iter().any(|(_, keys)| keys.iter().any(|(n, _)| *n == name)), "unknown hotkey {name}");
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
}
