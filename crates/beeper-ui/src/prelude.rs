//! The names that most modules of the UI use, so that each module imports only what is its own.

pub(crate) use crate::blocking::blocked_chats;
pub(crate) use crate::chat_list::{message_tag, preview_body};
pub(crate) use crate::emoji::EMOJI_FONT;
pub(crate) use crate::focus::{Keys, Look};
pub(crate) use crate::login::{actions, heading, Login, LoginMsg};
pub(crate) use crate::style::{
    bar, bold, bordered_box, bordered_btn, brand_svg, btn_style, clamp, clamped, done_btn, editor_style, ell, fill_box, fw, glow_box, hex,
    hidden_slider, ic, icon, input_style, lh, med, mix, pad, pick, rem, scroll_style, section_btn, semi, shadow_box, spaced, stable_bar, t,
    tabular, text_width, txt, Bare, Palette, CLEAR, DELETE_RED, ERROR, GUTTER, ON_ERROR,
};
pub(crate) use crate::time::{day_num, format_short_ts, iso_millis, list_ts, today};
pub(crate) use crate::units::human_size;
pub(crate) use crate::verify::{brand_btn_maybe, Brand};
pub(crate) use crate::{
    accounts, badge, blocking, brand_key, checkbox, commands, composer_with, deletable, delete_dialog, distance, downloads_dir, ease_out,
    feedback_btn, fmt_phone, forward, has_flag, hotkey_caps, hotkey_text, icon_btn, inbox_label, integrations, loads, locate, member_name,
    members, membership, menu_item, menu_pitch, menu_rule, merged, network_name, new_chat, outline_input, pending, pill_btn, pill_style,
    plan_pill, rail, reconnect, record, reminders, round_avatar, scheduled, sound, spell, stored, threads, tinted, toggle, video,
    view_once, vlist, voice, zoom, BeeperGuiApp, Confirm, El, Frames, InboxTab, Pane, PendingMedia, PillLook, SettingsTab, Toast,
    UiMessage, ACCOUNT_SUB_W, AVATAR_SLOTS, BUBBLE_PAD_X, CHEVRON_DOWN, COMPOSER_ICONS, ERROR_CIRCLE, INBOX_OPTIONS, ISSUES_URL, MENU_RULE,
    MESSAGE_MENU_W, ORANGE,
};
pub(crate) use beeper_core::cloud::{Cloud, CloudError, CodeResult};
pub(crate) use beeper_core::codec::Emoticon;
pub(crate) use beeper_core::config::{
    account_network, civil_from_days, cloud_bridge, find_network, millis_from_rfc3339, now_unix_millis, now_unix_secs, NETWORK_SPECS,
};
pub(crate) use beeper_core::matrix::{DeviceInfo, E2eeSetup, Mention, ROOM_MENTION};
pub(crate) use beeper_core::models::{Attachment, Chat, ChatFlag, Message, NetworkKind, NetworkSpec, User};
pub(crate) use beeper_core::store::{BeeperStore, Draft, OutFile, Query, SharedStore, NOTE_CHAT_ID};
pub(crate) use iced::advanced::layout::Layout;
pub(crate) use iced::advanced::renderer::Quad;
pub(crate) use iced::advanced::widget::{tree, Operation, Tree, Widget};
pub(crate) use iced::advanced::{Clipboard, Shell};
pub(crate) use iced::font::Weight;
pub(crate) use iced::gradient::Linear;
pub(crate) use iced::keyboard::key::Named;
pub(crate) use iced::keyboard::{Key, Modifiers};
pub(crate) use iced::widget::scrollable::{Direction, Scrollbar};
pub(crate) use iced::widget::text::Wrapping;
pub(crate) use iced::widget::{
    button, container, hover, image, mouse_area, pick_list, pin, rich_text, row, scrollable, slider, span, stack, svg, text_editor,
    text_input, tooltip, Column, Row, Space, Stack,
};
pub(crate) use iced::{
    Background, Border, Bottom, Center, Color, ContentFit, Degrees, Element, Event, Fill, Gradient, Length, Padding, Pixels, Point,
    Radians, Rectangle, Renderer, Right, Shadow, Shrink, Size, Subscription, Task, Theme, Transformation, Vector,
};
pub(crate) use std::cell::{Cell, RefCell};
pub(crate) use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
pub(crate) use std::num::NonZero;
pub(crate) use std::ops::Range;
pub(crate) use std::path::{Path, PathBuf};
pub(crate) use std::rc::Rc;
pub(crate) use std::sync::atomic::{AtomicBool, Ordering};
pub(crate) use std::sync::{mpsc, Arc, LazyLock, Mutex, OnceLock};
pub(crate) use std::time::{Duration, Instant};
