use crate::config::{resolve_data_dir_for_os, resolve_original_data_dir_for_os, APP_BUNDLE_ID, TRAY_ICON_SIZE};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OsKind {
    MacOs,
    Windows,
    Linux,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayServerKind {
    Wayland,
    X11,
    Quartz,
    Win32,
    Headless,
}

impl OsKind {
    pub fn current() -> Self {
        match std::env::consts::OS {
            "macos" | "ios" => Self::MacOs,
            "windows" => Self::Windows,
            "linux" | "freebsd" | "openbsd" | "netbsd" => Self::Linux,
            other => panic!("unsupported OS: {other}"),
        }
    }

    pub fn node_platform_str(&self) -> &'static str {
        match self {
            Self::MacOs => "darwin",
            Self::Windows => "win32",
            Self::Linux => "linux",
        }
    }

    pub fn primary_mod_label(&self) -> &'static str {
        match self {
            Self::MacOs => "⌘",
            Self::Windows | Self::Linux => "Ctrl",
        }
    }

    pub fn alt_mod_label(&self) -> &'static str {
        match self {
            Self::MacOs => "⌥",
            Self::Windows | Self::Linux => "Alt",
        }
    }

    pub fn detect_display_server(&self, env_vars: &HashMap<String, String>) -> DisplayServerKind {
        match self {
            Self::MacOs => DisplayServerKind::Quartz,
            Self::Windows => DisplayServerKind::Win32,
            Self::Linux => {
                if env_vars.get("WAYLAND_DISPLAY").is_some_and(|v| !v.trim().is_empty()) {
                    DisplayServerKind::Wayland
                } else if env_vars.get("DISPLAY").is_some_and(|v| !v.trim().is_empty()) {
                    DisplayServerKind::X11
                } else {
                    DisplayServerKind::Headless
                }
            }
        }
    }
}

pub fn resolve_home_dir(env_vars: &HashMap<String, String>) -> PathBuf {
    if let Some(h) = env_vars.get("HOME").or_else(|| env_vars.get("USERPROFILE")).filter(|v| !v.trim().is_empty()) {
        return PathBuf::from(h);
    }
    if let (Some(drive), Some(path)) = (env_vars.get("HOMEDRIVE"), env_vars.get("HOMEPATH")) {
        let combined = format!("{drive}{path}");
        if !combined.trim().is_empty() {
            return PathBuf::from(combined);
        }
    }
    std::env::temp_dir()
}

pub fn path_to_file_url(path: &Path) -> String {
    let raw = path.to_string_lossy();
    if raw.starts_with('/') {
        format!("file://{raw}")
    } else {
        format!("file:///{}", raw.replace('\\', "/"))
    }
}

pub fn file_url_to_path(url: &str) -> Option<PathBuf> {
    let stripped = url.strip_prefix("file://")?;
    let bytes = stripped.as_bytes();
    if bytes.len() >= 3 && bytes[0] == b'/' && bytes[1].is_ascii_alphabetic() && bytes[2] == b':' {
        return Some(PathBuf::from(&stripped[1..]));
    }
    Some(PathBuf::from(stripped))
}

pub fn open_url_or_path_command(os: OsKind, target: &str) -> (String, Vec<String>) {
    assert!(!target.trim().is_empty(), "target must not be empty");
    match os {
        OsKind::MacOs => ("open".to_string(), vec![target.to_string()]),
        OsKind::Linux => ("xdg-open".to_string(), vec![target.to_string()]),
        OsKind::Windows => {
            ("cmd".to_string(), vec!["/C".to_string(), "start".to_string(), String::new(), target.to_string()])
        }
    }
}

pub fn resolve_original_beeper_data_dir(
    os: OsKind,
    profile: Option<&str>,
    env_vars: &HashMap<String, String>,
    home_dir: &Path,
) -> PathBuf {
    resolve_original_data_dir_for_os(os.node_platform_str(), profile, env_vars, home_dir)
}

pub fn current_original_beeper_data_dir(profile: Option<&str>) -> PathBuf {
    let env_map: HashMap<String, String> = std::env::vars().collect();
    let home = resolve_home_dir(&env_map);
    resolve_original_beeper_data_dir(OsKind::current(), profile, &env_map, &home)
}

pub fn resolve_desktop_data_dir(
    os: OsKind,
    profile: Option<&str>,
    env_vars: &HashMap<String, String>,
    home_dir: &Path,
) -> PathBuf {
    resolve_data_dir_for_os(os.node_platform_str(), profile, env_vars, home_dir)
}

pub fn current_desktop_data_dir(profile: Option<&str>) -> PathBuf {
    let env_map: HashMap<String, String> = std::env::vars().collect();
    let home = resolve_home_dir(&env_map);
    resolve_desktop_data_dir(OsKind::current(), profile, &env_map, &home)
}

#[derive(Debug, Clone)]
pub struct OsAutostartManifest {
    pub os: OsKind,
    pub config_path: PathBuf,
    pub content: String,
    pub desktop_entry_path: Option<PathBuf>,
    pub desktop_entry_content: Option<String>,
}

pub fn generate_os_autostart_manifest(os: OsKind, home_dir: &Path, bin_path: &Path) -> OsAutostartManifest {
    let bin_str = bin_path.to_str().expect("valid UTF-8 binary path");
    assert!(!bin_str.is_empty(), "binary path must not be empty");
    match os {
        OsKind::MacOs => OsAutostartManifest {
            os,
            config_path: home_dir.join(format!("Library/LaunchAgents/{APP_BUNDLE_ID}.plist")),
            content: format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\"><dict><key>Label</key><string>{APP_BUNDLE_ID}</string><key>ProgramArguments</key><array><string>{bin_str}</string></array><key>RunAtLoad</key><true/></dict></plist>\n"
            ),
            desktop_entry_path: None,
            desktop_entry_content: None,
        },
        OsKind::Linux => OsAutostartManifest {
            os,
            config_path: home_dir.join(".config/systemd/user/beeper-desktop.service"),
            content: format!(
                "[Unit]\nDescription=Beeper Desktop\nAfter=graphical-session.target\n\n[Service]\nExecStart={bin_str}\nRestart=on-failure\n\n[Install]\nWantedBy=default.target\n"
            ),
            desktop_entry_path: Some(home_dir.join(".local/share/applications/beeper-desktop.desktop")),
            desktop_entry_content: Some(format!(
                "[Desktop Entry]\nType=Application\nName=Beeper\nExec={bin_str} %u\nTerminal=false\nCategories=Network;InstantMessaging;Chat;\nMimeType=x-scheme-handler/beeper;\nStartupWMClass={APP_BUNDLE_ID}\n"
            )),
        },
        OsKind::Windows => OsAutostartManifest {
            os,
            config_path: home_dir.join("AppData/Roaming/Microsoft/Windows/Start Menu/Programs/Startup/Beeper.cmd"),
            content: format!("@echo off\r\nstart \"\" \"{bin_str}\"\r\n"),
            desktop_entry_path: None,
            desktop_entry_content: None,
        },
    }
}

pub fn build_tray_icon_rgba(unread_count: u32, muted: bool) -> (Vec<u8>, u32, u32) {
    let width = TRAY_ICON_SIZE;
    let height = TRAY_ICON_SIZE;
    let mut rgba = vec![0u8; (width * height * 4) as usize];
    let base_color: [u8; 4] = if muted { [108, 117, 125, 255] } else { [107, 83, 255, 255] };

    for y in 0..height {
        for x in 0..width {
            let dx = x as i32 - 15;
            let dy = y as i32 - 15;
            let idx = ((y * width + x) * 4) as usize;
            if dx * dx + dy * dy <= 169 {
                let in_glyph = ((11..=20).contains(&x) && (y == 11 || y == 15 || y == 19))
                    || ((x == 11 || x == 20) && (11..=19).contains(&y));
                let color = if in_glyph { [255, 255, 255, 255] } else { base_color };
                rgba[idx..idx + 4].copy_from_slice(&color);
            }
            if unread_count > 0 {
                let bdx = x as i32 - 25;
                let bdy = y as i32 - 7;
                if bdx * bdx + bdy * bdy <= 25 {
                    rgba[idx..idx + 4].copy_from_slice(&[255, 59, 48, 255]);
                }
            }
        }
    }
    assert_eq!(rgba.len(), (width * height * 4) as usize);
    (rgba, width, height)
}

pub struct SystemTrayState {
    pub unread_count: u32,
    pub muted: bool,
    pub window_visible: bool,
    pub tooltip: String,
    pub icon_rgba: Vec<u8>,
    pub icon_width: u32,
    pub icon_height: u32,
    pub menu_items: Vec<String>,
    pub native_enabled: bool,
}

impl SystemTrayState {
    pub fn new(unread_count: u32, muted: bool, try_native: bool) -> Self {
        let (icon_rgba, icon_width, icon_height) = build_tray_icon_rgba(unread_count, muted);
        let tooltip = Self::format_tooltip(unread_count, muted);
        let menu_items = vec![
            "Open Beeper".to_string(),
            format!("Unread Chats ({unread_count})"),
            if muted { "Unmute Notifications".to_string() } else { "Mute Notifications (DND)".to_string() },
            "Quit Beeper".to_string(),
        ];
        Self {
            unread_count,
            muted,
            window_visible: true,
            tooltip,
            icon_rgba,
            icon_width,
            icon_height,
            menu_items,
            native_enabled: try_native,
        }
    }

    fn format_tooltip(unread_count: u32, muted: bool) -> String {
        match (unread_count, muted) {
            (0, false) => "Beeper — All caught up".to_string(),
            (0, true) => "Beeper — Notifications muted (DND)".to_string(),
            (n, false) => format!("Beeper — {n} unread conversation(s)"),
            (n, true) => format!("Beeper — {n} unread (DND active)"),
        }
    }

    pub fn update(&mut self, unread_count: u32, muted: bool) {
        self.unread_count = unread_count;
        self.muted = muted;
        let (rgba, w, h) = build_tray_icon_rgba(unread_count, muted);
        self.icon_rgba = rgba;
        self.icon_width = w;
        self.icon_height = h;
        self.tooltip = Self::format_tooltip(unread_count, muted);
        self.menu_items[1] = format!("Unread Chats ({unread_count})");
        self.menu_items[2] =
            if muted { "Unmute Notifications".to_string() } else { "Mute Notifications (DND)".to_string() };
    }
}
