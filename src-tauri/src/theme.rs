//! Theme model (spec v2): the official WebUI is the single source of
//! truth for light/dark. The shell only *observes* it and mirrors to
//! native surfaces. OS theme matters solely because the WebUI itself
//! may be in "follow system" mode.
//!
//! This module keeps only the concrete theme type and OS detection,
//! used at startup to pick an initial tray icon before any WebUI
//! observation has arrived.

use serde::{Deserialize, Serialize};

/// Resolved concrete theme.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Effective {
    Light,
    Dark,
}

impl Effective {
    pub fn as_str(self) -> &'static str {
        match self {
            Effective::Light => "light",
            Effective::Dark => "dark",
        }
    }
}

/// Read a `HKCU\...\Themes\Personalize` DWORD and interpret it as a
/// light/dark flag. `1` => light, `0`/`missing` => dark.
#[cfg(windows)]
fn personalize_flag(name: &str) -> Effective {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let output = Command::new("reg")
        .args([
            "query",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
            "/v",
            name,
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output();

    if let Ok(out) = output {
        let s = String::from_utf8_lossy(&out.stdout);
        for line in s.lines() {
            let line = line.trim();
            if line.starts_with(name) {
                if let Some(token) = line.split_whitespace().last() {
                    return if token.eq_ignore_ascii_case("0x0") {
                        Effective::Dark
                    } else {
                        Effective::Light
                    };
                }
            }
        }
    }
    Effective::Light
}

/// Detect OS *app-mode* theme.
///
/// Windows: `HKCU\...\Themes\Personalize\AppsUseLightTheme` (1=light, 0=dark).
/// This is the value browsers resolve `prefers-color-scheme` from, i.e. the
/// theme the *WebUI* "follow system" mode observes. Falls back to Light.
/// Non-Windows defaults to Light.
#[allow(dead_code)]
pub fn system_theme() -> Effective {
    #[cfg(windows)]
    {
        personalize_flag("AppsUseLightTheme")
    }
    #[cfg(not(windows))]
    {
        Effective::Light
    }
}

/// Detect OS *system/chrome-mode* theme.
///
/// Windows keeps two independent personalization switches:
///   * `AppsUseLightTheme`  -> app content (and `prefers-color-scheme`)
///   * `SystemUsesLightTheme` -> the taskbar / system tray / title-bar chrome
///
/// The **taskbar and tray backgrounds** are driven by `SystemUsesLightTheme`,
/// so this is the value we must consult when choosing a tray/taskbar icon
/// that stays visible against that background. They can diverge (e.g. light
/// Windows mode + dark app mode), which is exactly the case the old code got
/// wrong by reading `AppsUseLightTheme` for the native chrome.
///
/// Falls back to Light, non-Windows defaults to Light.
pub fn system_chrome_theme() -> Effective {
    #[cfg(windows)]
    {
        personalize_flag("SystemUsesLightTheme")
    }
    #[cfg(not(windows))]
    {
        Effective::Light
    }
}
