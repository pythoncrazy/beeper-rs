use crate::config::{resolve_data_dir_for_os, resolve_original_data_dir_for_os, APP_BUNDLE_ID};
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

    pub fn detect_display_server(&self, env_vars: &HashMap<String, String>) -> DisplayServerKind {
        match self {
            Self::MacOs => DisplayServerKind::Quartz,
            Self::Windows => DisplayServerKind::Win32,
            Self::Linux if env_vars.contains_key("WAYLAND_DISPLAY") => DisplayServerKind::Wayland,
            Self::Linux if env_vars.contains_key("DISPLAY") => DisplayServerKind::X11,
            Self::Linux => DisplayServerKind::Headless,
        }
    }
}

pub fn resolve_home_dir(env_vars: &HashMap<String, String>) -> PathBuf {
    PathBuf::from(env_vars.get("HOME").or_else(|| env_vars.get("USERPROFILE")).expect("HOME or USERPROFILE is set"))
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
    resolve_original_beeper_data_dir(OsKind::current(), profile, &env_map, &resolve_home_dir(&env_map))
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
    resolve_desktop_data_dir(OsKind::current(), profile, &env_map, &resolve_home_dir(&env_map))
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
