#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    Toggle(bool),
    Select(&'static [&'static str], u8),
    Slider(u8),
    Archive(&'static Pref, &'static Pref),
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
            Control::Archive(..) => unreachable!("the archive row {} has no value", self.key),
        }
    }

    pub fn max_value(&self) -> u8 {
        match self.control {
            Control::Toggle(_) => 1,
            Control::Select(options, _) => options.len() as u8 - 1,
            Control::Slider(_) => 100,
            Control::Archive(..) => unreachable!("the archive row {} has no value", self.key),
        }
    }

    const fn desc(self, desc: &'static str) -> Self {
        Pref { desc: Some(desc), ..self }
    }

    const fn badge(self, badge: Badge) -> Self {
        Pref { badge: Some(badge), ..self }
    }
}

const fn t(key: &'static str, label: &'static str, on: bool) -> Pref {
    Pref { key, label, desc: None, badge: None, control: Control::Toggle(on) }
}

const fn s(key: &'static str, label: &'static str, options: &'static [&'static str], default: u8) -> Pref {
    assert!((default as usize) < options.len());
    Pref { key, label, desc: None, badge: None, control: Control::Select(options, default) }
}

macro_rules! cmd {
    ($first:literal $(, $rest:literal)*) => {
        if cfg!(target_os = "macos") { concat!($first $(, "⌘", $rest)*) } else { concat!($first $(, "Ctrl", $rest)*) }
    };
}

const AFTER: &[&str] = &["Select next chat", "Select previous chat", "Close chat", "Do nothing"];
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
        t("typingIndicators", "Show recipients I'm typing", false).badge(Badge::Plus),
    ]),
    Item::Group(&[s("appLanguage", "App language", LANGUAGES, 0)]),
    Item::Group(&[
        s("remindLaterDefault", "Default for remind later", &["If no reply", "Always"], 0).badge(Badge::Plus),
        t("archiveOnReminder", "Archive after setting reminder on chat", false).badge(Badge::Plus),
    ]),
    Item::Group(&[
        t("markReadOnArchive", "Mark unread chats as read on archive", false),
        s("afterArchive", "After archive", AFTER, 0),
        Pref { key: "archiveOlder", label: "", desc: None, badge: None, control: Control::Archive(&ARCHIVE_SCOPE, &ARCHIVE_AGE) },
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
        s("microphone", "Microphone", &["Select a microphone", "Same as system"], 1),
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
    Item::Group(&[t("shareAnalytics", "Share Analytics", false)
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
        t("colorizeNetworkIcons", "Colorize network icons", true),
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
        s(
            "primaryComposerButton",
            "Primary composer button",
            &["Emoji Picker", "Talk to Type", "GIF Picker", "Add Attachment", "Send Later", "None"],
            0,
        ),
        s(
            "secondaryComposerButton",
            "Secondary composer button",
            &["Talk to Type", "Record Audio", "GIF Picker", "Add Attachment", "Send Later", "None"],
            1,
        ),
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
        Pref { key: "soundVolume", label: "Sound effect volume", desc: None, badge: None, control: Control::Slider(50) },
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

pub fn all() -> impl Iterator<Item = (&'static str, &'static Pref)> {
    PAGES.iter().flat_map(|(page, items)| {
        items.iter().flat_map(move |i| match i {
            Item::Group(prefs) => prefs
                .iter()
                .flat_map(|p| match p.control {
                    Control::Archive(scope, age) => vec![scope, age],
                    _ => vec![p],
                })
                .map(move |p| (*page, p))
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        })
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
