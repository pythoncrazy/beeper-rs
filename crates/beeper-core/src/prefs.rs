#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    Toggle(bool),
    Select(&'static [&'static str], u8),
    Slider(u8),
    Archive(&'static Pref, &'static Pref),
    /// Desktop's `AUDIO_INPUT_DEVICE_ID`, a device name in [`crate::models::BeeperSettings::microphone`].
    Microphone,
    /// Desktop's multiselect of a Plus feature, which shows the labels of its default values.
    Multi(&'static [&'static str]),
    /// Desktop's "Share Analytics", the `analyticsDisabled` field of the Beeper account.
    Analytics,
    /// Desktop's "Downloads location", [`crate::models::BeeperSettings::downloads_dir`].
    Downloads,
    /// Desktop's `BlockedUsersPref`: the number of blocked people, which opens the Blocked users window.
    BlockedUsers,
    /// Desktop's `QuickReactionEmojiPreference`: the emoji button of [`crate::models::BeeperSettings::quick_reaction`].
    /// The General row shows only when a double click on a message reacts.
    ReactionEmoji {
        general: bool,
    },
    /// Desktop's `QuickReactionsSettings` under "Show quick reactions": the frequent emoji, or the
    /// [`crate::models::BeeperSettings::quick_reactions`] that the toggle picks.
    QuickReactions(&'static Pref),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Badge {
    Labs,
    Plus,
    /// A Plus feature with Desktop's "Labs" tag.
    PlusLabs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pref {
    pub key: &'static str,
    pub label: &'static str,
    pub desc: Option<&'static str>,
    /// The values of the `%s` placeholders in the label or the description, where the name of a hotkey stands for
    /// its keys.
    pub args: &'static [&'static str],
    pub badge: Option<Badge>,
    /// The key of Desktop's `featureFlag`, which shows, locks or hides the row.
    pub feature: Option<&'static str>,
    /// The toggle that must be on for this row to be enabled.
    pub requires: Option<&'static str>,
    /// Desktop's `displayName` in search when it is not the label, like "Notification Sound" for "Sound".
    pub name: Option<&'static str>,
    /// Desktop's `description` of a row without a `footer`, which only the settings search reads.
    pub about: Option<&'static str>,
    /// Desktop's `searchKeywords`.
    pub keywords: &'static [&'static str],
    pub control: Control,
}

#[derive(Debug, Clone, Copy)]
pub enum Item {
    /// A text with the values of its `%s` placeholders.
    Subtitle(&'static str, &'static [&'static str]),
    Heading(&'static str),
    Group(&'static [Pref]),
    Theme,
    /// Desktop's `ChatAppearancePicker` of the default chat theme.
    ChatTheme,
    /// Desktop's zoom slider, with a text below it.
    Zoom(&'static str, &'static [&'static str]),
    /// Desktop's `TimeSuggestionsPreference`: the editor of [`crate::models::BeeperSettings::time_suggestions`].
    TimeSuggestions,
}

impl Pref {
    pub fn default_value(&self) -> u8 {
        match self.control {
            Control::Toggle(on) => on as u8,
            Control::Select(_, i) | Control::Slider(i) => i,
            _ => unreachable!("the row {} has no value", self.key),
        }
    }

    pub fn max_value(&self) -> u8 {
        match self.control {
            Control::Toggle(_) => 1,
            Control::Select(options, _) => options.len() as u8 - 1,
            Control::Slider(_) => 100,
            _ => unreachable!("the row {} has no value", self.key),
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

    /// A row that shows only when the account shows the Desktop feature.
    const fn gated(self, feature: &'static str) -> Self {
        Pref { feature: Some(feature), ..self }
    }

    /// A row of a Desktop feature with a Plus pill.
    const fn plus(self, feature: &'static str) -> Self {
        let badge = if matches!(self.badge, Some(Badge::Labs)) { Badge::PlusLabs } else { Badge::Plus };
        Pref { badge: Some(badge), feature: Some(feature), ..self }
    }

    /// The `args` of the label, then those of the description.
    pub fn split_args(&self) -> (&'static [&'static str], &'static [&'static str]) {
        self.args.split_at(self.label.matches('%').count())
    }

    const fn requires(self, key: &'static str) -> Self {
        Pref { requires: Some(key), ..self }
    }

    const fn named(self, name: &'static str) -> Self {
        Pref { name: Some(name), ..self }
    }

    /// A description that the settings search finds, but that the row does not show.
    const fn about(self, about: &'static str) -> Self {
        Pref { about: Some(about), ..self }
    }

    const fn keywords(self, keywords: &'static [&'static str]) -> Self {
        Pref { keywords, ..self }
    }
}

const fn t(key: &'static str, label: &'static str, on: bool) -> Pref {
    custom(key, label, Control::Toggle(on))
}

/// A row with its own control and no value.
const fn custom(key: &'static str, label: &'static str, control: Control) -> Pref {
    Pref {
        key,
        label,
        desc: None,
        args: &[],
        badge: None,
        feature: None,
        requires: None,
        name: None,
        about: None,
        keywords: &[],
        control,
    }
}

const fn s(key: &'static str, label: &'static str, options: &'static [&'static str], default: u8) -> Pref {
    assert!((default as usize) < options.len());
    custom(key, label, Control::Select(options, default))
}

macro_rules! cmd {
    ($first:literal $(, $rest:literal)*) => {
        if cfg!(target_os = "macos") { concat!($first $(, "⌘", $rest)*) } else { concat!($first $(, "Ctrl", $rest)*) }
    };
}

const AFTER: &[&str] = &["Select next chat", "Select previous chat", "Close chat", "Do nothing"];
/// The values of the composer buttons. Sticker Picker came later, so it follows None to keep the stored values.
pub const COMPOSER_ACTIONS: &[&str] = &[
    "Emoji Picker",
    "Talk to Type",
    "Record Audio",
    "GIF Picker",
    "Add Attachment",
    "Send Later",
    "None",
    "Sticker Picker",
];
/// The index of None in [`COMPOSER_ACTIONS`].
pub const COMPOSER_NONE: u8 = 6;
/// The index of Sticker Picker in [`COMPOSER_ACTIONS`].
pub const COMPOSER_STICKERS: u8 = 7;
/// Desktop's order of the options (`sFt`): Sticker Picker comes before None.
pub const COMPOSER_ORDER: [u8; 8] = [0, 1, 2, 3, 4, 5, COMPOSER_STICKERS, COMPOSER_NONE];
/// The values of "Allow undo send", in milliseconds.
pub const UNDO_SEND_MS: &[i64] = &[0, 750, 1000, 2000, 3000, 5000, 10000];
const UNDO_SEND_DELAYS: &[&str] =
    &["0 ms (disabled)", "750 ms", "1 second", "2 seconds", "3 seconds", "5 seconds", "10 seconds"];
const SOUND: &[&str] = &["Default", "None"];

/// Desktop's `font_size` options. The root font size is the value plus 3 px, so the default 13 is 16 px.
pub const FONT_SIZES: &[&str] = &[
    "9", "10", "11", "12", "13", "14", "15", "16", "17", "18", "19", "20", "21", "22", "23", "24", "25", "26", "27",
    "28", "29", "30", "31", "32", "33", "34", "35", "36",
];
pub const LANGUAGES: &[&str] =
    &["System", "English", "Deutsch", "Español", "Français", "日本語", "Português do Brasil"];
pub const ARCHIVE_AGE_DAYS: [i64; 6] = [3, 7, 14, 28, 90, 365];
/// Desktop's `quick_reactions_customized`: the quick reactions are the custom emoji.
pub const QUICK_REACTIONS_CUSTOMIZED: Pref = t("quickReactionsCustomized", "", false);
/// Desktop's `message_double_click_reaction`, any emoji, which [`crate::models::BeeperSettings`] keeps as text.
pub const QUICK_REACTION: &str = "message_double_click_reaction";
/// Desktop's `include_matrix_account_search`, the internal-only "Include Matrix results in Start New Chat": the
/// user search of the Matrix account, off unless Desktop turned it on.
pub const INCLUDE_MATRIX_ACCOUNT_SEARCH: &str = "include_matrix_account_search";
pub const ARCHIVE_SCOPE: Pref = s("archiveScope", "", &["all", "read"], 0);
pub const ARCHIVE_AGE: Pref = s("archiveAge", "", &["3 days", "7 days", "14 days", "28 days", "3 months", "1 year"], 0);

pub const GENERAL: &[Item] = &[
    Item::Subtitle("These settings are for %s and aren't synced to the messaging platforms.", &["Beeper"]),
    Item::Group(&[
        t("incognito", "Incognito mode", false)
            .plus("stealth-mode")
            .desc("Incognito mode keeps chats unread even when you click on them and doesn't notify recipients you've read them. Respond or hit %s to mark a chat as read.")
            .args(&["Chat: Toggle Mark As Read"]),
        t("typingIndicators", "Show recipients I'm typing", true).plus("stealth-mode"),
        custom("blockedUsers", "Blocked users", Control::BlockedUsers).gated("block-users"),
    ]),
    Item::Group(&[s("appLanguage", "App language", LANGUAGES, 0).keywords(&["locale", "localization", "translation"])]),
    Item::Group(&[
        s("remindLaterDefault", "Default for remind later", &["If no reply", "Always"], 0).plus("remind-later"),
        t("archiveOnReminder", "Archive after setting reminder on chat", false).plus("remind-later"),
    ]),
    Item::Group(&[
        t("markReadOnArchive", "Mark unread chats as read on archive", false).gated("archive-as-unread"),
        s("afterArchive", "After archive", AFTER, 0),
        custom("archiveOlder", "", Control::Archive(&ARCHIVE_SCOPE, &ARCHIVE_AGE)),
    ]),
    Item::Group(&[
        s("afterMarkRead", "After mark as read/unread", AFTER, 0),
        s(
            "messageDoubleClick",
            "On double-clicking a message",
            &["Do nothing", "Open the reaction picker", "Reply", "Forward", "Show menu", "React with..."],
            2,
        ),
        custom("doubleClickEmoji", "Quick reaction emoji", Control::ReactionEmoji { general: true }),
    ]),
    Item::Group(&[
        s("undoSendDelay", "Allow undo send (%s) for", UNDO_SEND_DELAYS, 0)
            .plus("undo-send")
            .desc("Hit %s to send instantly.")
            .args(const { &[cmd!("", " Z"), cmd!("", " Shift Z")] }),
        t("undoOnPendingClick", "Click pending messages to undo send", false).plus("undo-send"),
        t("copyTimestamps", "Include timestamps when copying messages with %s", false).args(const { &[cmd!("", " C")] }),
        t("emojiAutocomplete", "Enable emoji autocomplete", true),
        t("autoSendGifs", "Automatically send GIFs after selection", false),
        t("autoPlayVoiceNotes", "Automatically play the next voice note", true),
        custom("microphone", "Microphone", Control::Microphone),
        t("keepEmoticons", "Disable automatic replacement of emoticons with emojis", false),
        t("autoCopyOtp", "Auto copy OTP/2FA codes", true).desc("If the chat is muted, you'll still be notified."),
        s("upArrow", "Press ↑ in the composer to:", &["Move focus to the message list", "Edit your last message"], 0),
    ]),
    Item::Heading("Sticker packs"),
    Item::Group(&[t("stickerPacks", "Use sticker packs", false)
        .badge(Badge::Labs)
        .desc("Send stickers from your Telegram, Signal and WhatsApp packs, and from Matrix sticker packs.")]),
    Item::Heading("Time Suggestions"),
    Item::TimeSuggestions,
    Item::Group(&[
        custom("downloadsDir", "Downloads location", Control::Downloads).named("Downloads Preferences"),
        t("alwaysAskDownload", "Always ask where to save files before downloading", false),
    ]),
    Item::Group(&[
        t("startMinimized", "Keep %s minimized on launch", false).args(&["Beeper"]),
        t("quitOnClose", "Quit %s when closing the main window", false).args(&["Beeper"]),
    ]),
    Item::Group(&[s("sendLaterDefault", "Default for send later", &["If no reply", "Regardless"], 0).plus("scheduled-message")]),
    Item::Group(&[
        t("whatsappCommunities", "Show WhatsApp Communities as separate spaces", false),
        t("telegramTopics", "Show Telegram Topics as separate spaces", false),
    ]),
    Item::Group(&[
        t("hwAcceleration", "Hardware acceleration", true),
        t("hiddenEvents", "Show hidden events in timeline", false),
        s("timeFormat", "Time Format", &["12-hour", "24-hour"], 0),
    ]),
    Item::Group(&[custom("analytics", "Share Analytics", Control::Analytics)
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
        t("hideDeleted", "Hide deleted messages", false)
            .about("Hide deleted messages from conversations instead of showing a placeholder"),
        t("enableNudges", "Enable nudges", true)
            .about("Shake the conversation when someone sends you a nudge")
            .gated("nudge"),
        t("messageAnimations", "Enable message animations", true),
        t("autoplayGifs", "Autoplay GIFs", true),
        t("animateAvatars", "Animate avatars and emoji", true),
        t("balanceText", "Balance message text", false).about("Adjust line breaks to create more even message bubbles"),
        t("loadEmbeds", "Load external embeds automatically", true),
        t("incognitoIndicator", "Show incognito mode indicator", true).plus("stealth-mode").requires("incognito"),
        t("quickReactions", "Show quick reactions", true),
        custom("quickReactionsFooter", "", Control::QuickReactions(&QUICK_REACTIONS_CUSTOMIZED)),
        custom("quickReactionEmoji", "Quick reaction emoji", Control::ReactionEmoji { general: false })
            .desc("Used for quick react keyboard shortcut (%s)")
            .args(&["Message: Quick React"]),
        t("reactionsOnSelect", "Show reactions when a message is selected", false)
            .about("Display who reacted to a message when you select it"),
        t("feedbackWidget", "Show feedback widget", true),
        t("membershipEvents", "Show membership events", true).about("Show when people join or leave a conversation"),
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
    Item::Heading("Chat Appearance"),
    Item::ChatTheme,
    Item::Heading("Zoom"),
    Item::Zoom("You can use %1$s to zoom in and %2$s to zoom out.", &["Zoom In", "Zoom Out"]),
    Item::Heading("Font Size"),
    Item::Group(&[s("fontSize", "Font size", FONT_SIZES, 4)]),
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
        custom("soundVolume", "Sound effect volume", Control::Slider(50)).named("Sound Effect Volume"),
        t("messageNotifications", "Message notifications", true),
        s("messageSound", "Sound", SOUND, 0).named("Notification Sound"),
        t("groupNotifications", "Group message notifications", true),
        t("requestNotifications", "Message request notifications", true),
        s("groupSound", "Sound", SOUND, 0).named("Group Notification Sound"),
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

const TRANSLATION_LANGUAGES: &[&str] =
    &["Arabic", "Chinese", "English", "French", "German", "Italian", "Japanese", "Portuguese", "Russian", "Spanish"];

pub const AI: &[Item] = &[
    Item::Heading("General"),
    Item::Group(&[custom("translationLanguages", "Translation Languages", Control::Multi(TRANSLATION_LANGUAGES))
        .desc("AI response quality may vary for less commonly spoken languages.")
        .badge(Badge::PlusLabs)
        .gated("ai-in-beeper-chats")]),
    Item::Heading("Voice Transcription"),
    Item::Group(&[
        t("transcribeWithOpenai", "Use OpenAI's Whisper model for Talk to Type and transcribing voice notes", false)
            .plus("talk-to-type")
            .desc("Privacy: this will send audio clips to OpenAI via our servers.")
            .keywords(&["speech to text", "voice to text"]),
        // Desktop shows these two rows only when `transcribeWithOpenai` is on.
        t("autoTranscribe", "Automatically transcribe voice notes when they are received", false).plus("talk-to-type"),
        s("transcriptionLanguage", "Default language", &TRANSCRIPTION_NAMES, 0)
            .plus("voice-transcription")
            .desc("Improves accuracy.")
            .keywords(&["spoken language", "voice note language", "transcription language"]),
    ]),
    Item::Heading("AI Mentions"),
    Item::Group(&[t("mentionLlms", "Mention an AI to get answers and help in chat", false)
        .plus("ai-in-beeper-chats")
        .desc("Will send the %d most recent messages, including sender names and any @mentioned users.")
        .args(&["20"])]),
];

/// Desktop's "Auto-detect" (`Gd`) and its Whisper languages (`yx`), by code.
pub const TRANSCRIPTION_LANGUAGES: [(&str, &str); 58] = [
    ("auto", "Auto-detect"),
    ("af", "Afrikaans"),
    ("ar", "Arabic"),
    ("hy", "Armenian"),
    ("az", "Azerbaijani"),
    ("be", "Belarusian"),
    ("bs", "Bosnian"),
    ("bg", "Bulgarian"),
    ("ca", "Catalan"),
    ("zh", "Chinese"),
    ("hr", "Croatian"),
    ("cs", "Czech"),
    ("da", "Danish"),
    ("nl", "Dutch"),
    ("en", "English"),
    ("et", "Estonian"),
    ("fi", "Finnish"),
    ("fr", "French"),
    ("gl", "Galician"),
    ("de", "German"),
    ("el", "Greek"),
    ("he", "Hebrew"),
    ("hi", "Hindi"),
    ("hu", "Hungarian"),
    ("is", "Icelandic"),
    ("id", "Indonesian"),
    ("it", "Italian"),
    ("ja", "Japanese"),
    ("kn", "Kannada"),
    ("kk", "Kazakh"),
    ("ko", "Korean"),
    ("lv", "Latvian"),
    ("lt", "Lithuanian"),
    ("mk", "Macedonian"),
    ("ms", "Malay"),
    ("mr", "Marathi"),
    ("mi", "Maori"),
    ("ne", "Nepali"),
    ("no", "Norwegian"),
    ("fa", "Persian"),
    ("pl", "Polish"),
    ("pt", "Portuguese"),
    ("ro", "Romanian"),
    ("ru", "Russian"),
    ("sr", "Serbian"),
    ("sk", "Slovak"),
    ("sl", "Slovenian"),
    ("es", "Spanish"),
    ("sw", "Swahili"),
    ("sv", "Swedish"),
    ("tl", "Tagalog"),
    ("ta", "Tamil"),
    ("th", "Thai"),
    ("tr", "Turkish"),
    ("uk", "Ukrainian"),
    ("ur", "Urdu"),
    ("vi", "Vietnamese"),
    ("cy", "Welsh"),
];

/// The name of the Whisper language `code`, or None for "auto" and unknown codes (Desktop's `isTranscriptionLanguage`).
pub fn transcription_language(code: &str) -> Option<&'static str> {
    TRANSCRIPTION_LANGUAGES[1..].iter().find(|(c, _)| *c == code).map(|(_, name)| *name)
}

/// The codes (0) or the names (1) of [`TRANSCRIPTION_LANGUAGES`].
const fn transcription_column(i: usize) -> [&'static str; 58] {
    let (mut out, mut k) = ([""; 58], 0);
    while k < out.len() {
        out[k] = if i == 0 { TRANSCRIPTION_LANGUAGES[k].0 } else { TRANSCRIPTION_LANGUAGES[k].1 };
        k += 1;
    }
    out
}
/// The values of Desktop's `transcription_language`.
pub const TRANSCRIPTION_CODES: [&str; 58] = transcription_column(0);
const TRANSCRIPTION_NAMES: [&str; 58] = transcription_column(1);

pub const PAGES: [(&str, &[Item]); 4] =
    [("General", GENERAL), ("Appearance", APPEARANCE), ("Notifications", NOTIFICATIONS), ("AI", AI)];

/// The rows of the settings pages, with the page of each.
impl Item {
    /// Desktop's `displayName` and `searchKeywords` of a custom item.
    pub fn custom(&self) -> Option<(&'static str, &'static [&'static str])> {
        match self {
            Item::Theme => Some(("Theme Selector", &["light mode", "dark mode", "system mode"])),
            Item::ChatTheme => Some(("Default chat theme", &["wallpaper", "background", "bubble color", "chat theme"])),
            Item::Zoom(..) => Some(("Zoom Controls", &[])),
            Item::TimeSuggestions => Some(("Time suggestions", &[])),
            _ => None,
        }
    }
}

/// The page, the name and the search keywords of each custom item.
pub fn customs() -> impl Iterator<Item = (&'static str, &'static str, &'static [&'static str])> {
    PAGES.iter().flat_map(|(page, items)| items.iter().filter_map(move |i| i.custom().map(|(n, k)| (*page, n, k))))
}

pub fn rows() -> impl Iterator<Item = (&'static str, &'static Pref)> {
    PAGES.iter().flat_map(|(page, items)| {
        items.iter().flat_map(|i| if let Item::Group(prefs) = i { *prefs } else { &[] }).map(move |p| (*page, p))
    })
}

pub fn all() -> impl Iterator<Item = (&'static str, &'static Pref)> {
    rows().flat_map(|(page, p)| match p.control {
        Control::Archive(scope, age) => vec![(page, scope), (page, age)],
        Control::QuickReactions(customized) => vec![(page, customized)],
        _ => vec![(page, p)],
    })
}

/// The key of Desktop's `feature` of a hotkey, which hides the hotkey when the account does not show the feature.
pub fn hotkey_feature(name: &str) -> Option<&'static str> {
    match name {
        "Toggle Incognito Mode" => Some("stealth-mode"),
        "Remind Later" => Some("remind-later"),
        "Message: Send Later" => Some("scheduled-message"),
        "Undo Pending Send" => Some("undo-send"),
        "Message: Talk to Type" => Some("voice-transcription"),
        _ => None,
    }
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
    ("Toggle Incognito Mode", "App", "disabled"),
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
    ("Message: Talk to Type", "Messages", "ctrl+shift+t"),
    ("Message: Send GIF", "Messages", "ctrl+shift+g"),
    ("Message: Send Later", "Messages", "ctrl+shift+l"),
    ("Message: Send Message and Archive", "Messages", "ctrl+enter"),
    ("Message: Send Sticker", "Messages", "ctrl+alt+shift+s"),
    ("Undo Pending Send", "Composer", "disabled"),
    ("Show Replies", "Messages", "ctrl+down"),
    ("Insert New Line", "Composer", "alt+enter, ctrl+enter, shift+enter"),
    ("Send Message", "Composer", "enter"),
];

/// The default keys of a hotkey of either list. A fixed hotkey cannot be reassigned, but Desktop's Texts.app
/// keybindings turn Italicize Text Selection off.
pub fn hotkey_default(name: &str) -> &'static str {
    HOTKEYS.iter().chain(FIXED_HOTKEYS).find(|h| h.0 == name).unwrap_or_else(|| panic!("unknown hotkey {name}")).2
}

/// Desktop's "Switch to Texts.app keybindings" (`o` of `HotkeysPrefsSubView`): the keys that it gives each hotkey.
pub const TEXTS_KEYBINDINGS: &[(&str, &str)] = &[
    ("Message: Edit", "ctrl+i"),
    ("Italicize Text Selection", "disabled"),
    ("Select Previous Chat", "ctrl+[, ctrl+shift+tab"),
    ("Select Next Chat", "ctrl+], ctrl+tab"),
    ("Chat: Toggle Mark As Read", "ctrl+u"),
    ("Chat: Toggle Archive", "ctrl+e, ctrl+shift+e"),
    ("Switch First 9 Chats Modifier", "ctrl+alt"),
    ("Switch First 9 Spaces Modifier", "ctrl"),
    ("History: Previous Chat", "ctrl+alt+left"),
    ("History: Next Chat", "ctrl+alt+right"),
    ("Chat: Archive All Read", "disabled"),
];

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

/// Desktop's hotkeys that neither list shows, but that a recorded hotkey cannot take. Copy has no name.
pub const HIDDEN_HOTKEYS: &[(&str, &str, &str)] = &[
    ("Chat: Create New Group", "Chats", "ctrl+shift+n"),
    ("Select First 9 Commands Modifier", "App", "ctrl+alt"),
    ("", "Messages", "ctrl+c"),
];

/// Desktop's `modifierOnly` hotkeys, which record only modifiers.
pub const MODIFIER_ONLY: &[&str] = &["Switch First 9 Chats Modifier", "Switch First 9 Spaces Modifier"];

/// Desktop's `allowSingleKey` hotkeys, which can be one navigation or function key.
pub const SINGLE_KEY: &[&str] = &["Undo Pending Send"];

/// Desktop's `canShare` of a hotkey: the keys that it can share with a hotkey of another group.
pub fn hotkey_shares(name: &str) -> &'static [&'static str] {
    match name {
        "Message: Quote or Edit Selected Message" | "Send Message" | "Insert New Line" => &["enter"],
        "Select Previous Message" | "Undo Pending Send" => &["up"],
        "Switch First 9 Chats Modifier" | "Select First 9 Commands Modifier" => &["ctrl+alt"],
        _ => &[],
    }
}

/// Desktop's `gRt`: the hotkey other than `name` whose keys have the combination `combo`, unless the two hotkeys
/// are in different groups and both can share it. `keys` gives the keys of a reassignable hotkey.
pub fn hotkey_conflict<'a>(name: &str, combo: &str, keys: impl Fn(&'static str) -> &'a str) -> Option<&'static str> {
    let parts: Vec<&str> = combo.split('+').collect();
    let mods = ["ctrl", "shift", "alt"].map(|m| parts.contains(&m));
    let key = parts.last().copied().filter(|p| !["ctrl", "shift", "alt"].contains(p)).unwrap_or("");
    let group = |n: &str| HOTKEYS.iter().find(|h| h.0 == n).expect("a reassignable hotkey").1;
    let shared = |n: &str| hotkey_shares(n).contains(&combo) && hotkey_shares(name).contains(&combo);
    let reassignable = HOTKEYS.iter().map(|h| (h.0, h.1, keys(h.0)));
    let fixed = FIXED_HOTKEYS.iter().chain(HIDDEN_HOTKEYS).map(|h| -> (&str, &str, &'a str) { (h.0, h.1, h.2) });
    reassignable
        .chain(fixed)
        .filter(|h| h.0 != name && hotkey_has(h.2, mods, key))
        .find(|h| !(h.1 != group(name) && shared(h.0)))
        .map(|h| h.0)
}

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

/// The preference `key`. The views look up their preferences each time they draw, so the first lookup indexes them.
pub fn find(key: &str) -> &'static Pref {
    static BY_KEY: std::sync::OnceLock<std::collections::HashMap<&'static str, &'static Pref>> =
        std::sync::OnceLock::new();
    let by_key = BY_KEY.get_or_init(|| {
        let mut by_key = std::collections::HashMap::new();
        for (_, p) in all() {
            by_key.entry(p.key).or_insert(p);
        }
        by_key
    });
    by_key.get(key).copied().unwrap_or_else(|| panic!("unknown pref {key}"))
}

/// Desktop's global account data with the preferences that sync between devices.
pub const DESKTOP_PREFS: &str = "com.beeper.desktop.prefs";
/// Desktop's `time_suggestion_options` of [`DESKTOP_PREFS`]: the time phrases of the reminder menus.
pub const TIME_SUGGESTION_OPTIONS: &str = "time_suggestion_options";
/// Desktop's synced `quick_replies`.
pub const QUICK_REPLIES: &str = "quick_replies";
/// Desktop's `y0`: the most time suggestions.
pub const TIME_SUGGESTIONS_MAX: usize = 8;
/// Desktop's `Odt`: the time suggestions when there are none.
pub const TIME_SUGGESTIONS: [&str; 6] =
    ["In an Hour", "In Two Hours", "This Evening", "Tomorrow", "This Weekend", "Next Week"];

/// Desktop's `xpe`: the strings of a list, trimmed, without empty or repeated ones, and at most
/// [`TIME_SUGGESTIONS_MAX`].
pub fn time_suggestion_options(v: &serde_json::Value) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for s in v.as_array().into_iter().flatten().filter_map(serde_json::Value::as_str).map(str::trim) {
        if !s.is_empty() && !out.iter().any(|o| o == s) {
            out.push(s.to_string());
        }
        if out.len() == TIME_SUGGESTIONS_MAX {
            break;
        }
    }
    out
}

/// The JSON values that Desktop stores for the options of a preference.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Values {
    Bool,
    Str(&'static [&'static str]),
    /// Seconds, which Desktop writes as strings from its select.
    Secs(&'static [i64]),
    /// A fraction from 0 to 1, for a slider from 0 to 100.
    Volume,
    /// The numbers of the option labels.
    Num(&'static [&'static str]),
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
            Values::Num(labels) => {
                let n = v.as_i64().or_else(|| v.as_str()?.parse().ok());
                at(labels.iter().position(|x| x.parse().ok() == n))
            }
        }
    }

    pub fn encode(self, x: u8) -> serde_json::Value {
        match self {
            Values::Bool => (x == 1).into(),
            Values::Str(values) => values[usize::from(x)].into(),
            Values::Secs(values) => values[usize::from(x)].to_string().into(),
            Values::Volume => (f64::from(x) / 100.0).into(),
            Values::Num(labels) => labels[usize::from(x)].parse::<i64>().expect("a number option").into(),
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
    &["emoji", "talk_to_type", "record_audio", "gif_picker", "add_attachment", "send_later", "none", "sticker_picker"];

/// The preferences that Desktop syncs through account data. The other preferences stay on this device, as in Desktop.
pub const SYNCED: &[(&str, Ad)] = &[
    ("incognito", b("dont_mark_as_read_until_responded")),
    ("typingIndicators", b("show_typing_indicator")),
    ("incognitoIndicator", b("show_incognito_mode_indicator")),
    ("remindLaterDefault", v("dismiss_reminder_on_incoming_default", &["no_reply", "always"])),
    ("sendLaterDefault", v("schedule_submenu_default", &["no_reply", "always"])),
    ("archiveOnReminder", Ad::Event("com.beeper.reminder_settings", "archive_on_reminder_set")),
    ("markReadOnArchive", Ad::Event("com.beeper.mark_read_when_archive", "enabled")),
    ("afterArchive", v("after_archive", AFTER_VALUES)),
    ("afterMarkRead", v("after_toggle_read", AFTER_VALUES)),
    (
        "messageDoubleClick",
        v("message_double_click_action", &["do_nothing", "react", "reply", "forward", "show_menu", "react_with"]),
    ),
    ("undoSendDelay", Ad::Desktop("undo_send_delay_ms", Values::Secs(UNDO_SEND_MS))),
    ("undoOnPendingClick", b("undo_send_on_pending_click")),
    ("copyTimestamps", b("copy_message_timestamps")),
    ("emojiAutocomplete", b("enable_emoji_autocomplete")),
    ("autoSendGifs", b("auto_send_gifs")),
    ("autoPlayVoiceNotes", b("auto_play_next_voice_note")),
    ("keepEmoticons", b("disable_emoticon_replacement")),
    ("autoCopyOtp", b("auto_copy_otp")),
    ("upArrow", v("compose_up_arrow_action", &["focus_message_list", "edit_last_message"])),
    ("stickerPacks", b("sticker_packs")),
    ("whatsappCommunities", Ad::Event("com.beeper.labs.wa_communities_as_spaces", "wa_communities_as_spaces")),
    ("telegramTopics", Ad::Event("com.beeper.labs.tg_topics_as_spaces", "tg_topics_as_spaces")),
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
    ("enableNudges", b("enable_nudges")),
    ("messageAnimations", b("enable_message_animations")),
    ("autoplayGifs", b("autoplay_gifs")),
    ("animateAvatars", b("animate_avatars_and_emoji")),
    ("balanceText", b("balance_message_text")),
    ("loadEmbeds", b("load_iframes_automatically")),
    ("quickReactions", b("quick_reactions_enabled")),
    ("quickReactionsCustomized", b("quick_reactions_customized")),
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
    ("fontSize", Ad::Desktop("font_size", Values::Num(FONT_SIZES))),
    ("alwaysAskDownload", b("always_ask_download_location")),
    ("transcribeWithOpenai", b("transcribe_with_openai")),
    ("autoTranscribe", b("auto_transcribe")),
    ("transcriptionLanguage", v("transcription_language", &TRANSCRIPTION_CODES)),
    ("mentionLlms", b("mention_llms")),
];

/// The keys of the preferences that earlier versions stored. The settings drop them on load.
pub const RETIRED: &[&str] = &["shareAnalytics", "quickReactionEmoji"];

pub fn synced(key: &str) -> Option<Ad> {
    SYNCED.iter().find(|(k, _)| *k == key).map(|(_, a)| *a)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The index of `find` gives each key the first row of the key, as a walk over the rows does.
    #[test]
    fn find_takes_the_first_row_of_a_key() {
        for (_, p) in all() {
            let first = all().map(|(_, q)| q).find(|q| q.key == p.key).expect("a row");
            assert!(std::ptr::eq(find(p.key), first), "{}", p.key);
        }
    }

    #[test]
    fn synced_prefs_cover_every_option() {
        for (key, ad) in SYNCED {
            let p = find(key);
            let n = match ad {
                Ad::Desktop(_, Values::Str(v) | Values::Num(v)) => v.len(),
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
    fn time_suggestion_options_like_desktop() {
        use serde_json::json;
        let v = json!([" Tomorrow ", "", "  ", 5, null, "Tomorrow", "In 3w", "a", "b", "c", "d", "e", "f", "g"]);
        assert_eq!(time_suggestion_options(&v), ["Tomorrow", "In 3w", "a", "b", "c", "d", "e", "f"]);
        assert!(time_suggestion_options(&json!("Tomorrow")).is_empty(), "only a list");
        assert!(time_suggestion_options(&serde_json::Value::Null).is_empty());
        let mut s = crate::models::BeeperSettings::default();
        assert_eq!(s.time_suggestions(), TIME_SUGGESTIONS, "an empty list is Desktop's `Odt`");
        s.time_suggestions = vec!["In 3w".into()];
        assert_eq!(s.time_suggestions(), ["In 3w"]);
    }

    #[test]
    fn font_size_reads_desktop_numbers() {
        let Some(Ad::Desktop("font_size", values)) = synced("fontSize") else { panic!("font size is synced") };
        assert_eq!(find("fontSize").default_value(), values.decode(&serde_json::json!(13)).unwrap());
        assert_eq!(values.decode(&serde_json::json!("36")), Some(27), "the combobox writes the option key");
        assert_eq!(values.decode(&serde_json::json!(40)), None);
        assert_eq!(values.encode(0), serde_json::json!(9));
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
            let shared = |o: &str| (*name, o) == ("Show Replies", "Scroll to the bottom of a chat");
            let peers = HOTKEYS[..i].iter().filter(|h| (h.1 == "Composer") == (*group == "Composer") && !shared(h.0));
            for (other, g, k) in peers {
                assert!(combos().all(|c| !k.split(", ").any(|x| x == c)), "{name} and {other} ({g}) share {keys}");
            }
        }
    }

    #[test]
    fn hotkey_conflicts_like_desktop() {
        let conflict = |name, combo| hotkey_conflict(name, combo, hotkey_default);
        assert_eq!(conflict("Search", "ctrl+j"), Some("Command Bar"));
        assert_eq!(conflict("Search", "ctrl+k"), None, "a hotkey does not conflict with itself");
        assert_eq!(conflict("Search", "ctrl+shift+n"), Some("Chat: Create New Group"), "hidden hotkeys count");
        assert_eq!(conflict("Search", "ctrl+c"), Some(""));
        assert_eq!(conflict("Send Message", "enter"), None, "both can share Enter across groups");
        assert_eq!(conflict("Search", "enter"), Some("Message: Quote or Edit Selected Message"));
        assert_eq!(conflict("Switch First 9 Chats Modifier", "ctrl+alt"), Some("Switch First 9 Spaces Modifier"));
        assert_eq!(hotkey_conflict("Search", "ctrl+j", |n| if n == "Command Bar" { "ctrl+m" } else { "" }), None);
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
