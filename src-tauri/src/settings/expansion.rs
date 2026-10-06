use serde::{Deserialize, Serialize};

use super::config::AppSettings;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutputAction {
    #[default]
    Paste,
    PasteEnter,
    Copy,
    Hud,
    AppendFile {
        path: String,
    },
    RunCommand {
        template: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Hotkeys {
    pub dictation: String,
    pub command: Option<String>,
    pub assistant: Option<String>,
    pub note: Option<String>,
    pub cancel: Option<String>,
}

impl Default for Hotkeys {
    fn default() -> Self {
        Self {
            dictation: crate::platform::default_hotkey().into(),
            command: Some(
                if cfg!(windows) {
                    "Ctrl+Win+Alt"
                } else {
                    "Ctrl+Alt+Space"
                }
                .into(),
            ),
            assistant: None,
            note: None,
            cancel: Some("Escape".into()),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ModeTriggers {
    pub apps: Vec<String>,
    pub spoken: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ModeContext {
    pub selected_text: bool,
    pub clipboard: bool,
    pub window_title: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Mode {
    pub id: String,
    pub name: String,
    pub icon: Option<String>,
    pub hotkey: Option<String>,
    pub triggers: ModeTriggers,
    pub language: Option<String>,
    pub dictation_mode: String,
    pub cleanup_level: String,
    pub intelligence_tier: String,
    pub style: String,
    pub custom_instructions: String,
    pub context: ModeContext,
    pub output: OutputAction,
    pub send_key: String,
    pub translate_to: Option<String>,
    pub enabled: bool,
}

impl Default for Mode {
    fn default() -> Self {
        Self {
            id: "dictation".into(),
            name: "Dictation".into(),
            icon: None,
            hotkey: None,
            triggers: ModeTriggers::default(),
            language: None,
            dictation_mode: "normal".into(),
            cleanup_level: "light".into(),
            intelligence_tier: "smart_flow".into(),
            style: "neutral".into(),
            custom_instructions: String::new(),
            context: ModeContext::default(),
            output: OutputAction::Paste,
            send_key: default_send_key(),
            translate_to: None,
            enabled: true,
        }
    }
}

pub fn default_send_key() -> String {
    "enter".into()
}
pub fn default_mode_id() -> String {
    "dictation".into()
}
pub fn default_modes() -> Vec<Mode> {
    vec![
        Mode::default(),
        Mode {
            id: "message".into(),
            name: "Message".into(),
            style: "chat".into(),
            ..Mode::default()
        },
        Mode {
            id: "email".into(),
            name: "Email".into(),
            style: "email".into(),
            ..Mode::default()
        },
        Mode {
            id: "coding".into(),
            name: "Coding".into(),
            dictation_mode: "coding".into(),
            intelligence_tier: "raw_verbatim".into(),
            cleanup_level: "raw".into(),
            ..Mode::default()
        },
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snippet {
    pub id: String,
    pub trigger: String,
    pub expansion: String,
    #[serde(default = "super::config::default_true")]
    pub enabled: bool,
}

pub fn process_basename(process: &str) -> String {
    process
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(process)
        .trim()
        .to_ascii_lowercase()
        .trim_end_matches(".exe")
        .to_owned()
}

impl AppSettings {
    pub fn validate_expansion(&self) -> Result<(), String> {
        if self.paste_delay_ms > 2000
            || self.min_dictation_ms > 10_000
            || !self.sounds_volume.is_finite()
            || !(0.0..=1.0).contains(&self.sounds_volume)
        {
            return Err("Invalid duration or sound volume".into());
        }
        if !matches!(self.inject_method.as_str(), "paste" | "type")
            || !matches!(self.hud_contrast.as_str(), "standard" | "high")
        {
            return Err("Invalid injection method or HUD contrast".into());
        }
        let custom = self.accent_custom.strip_prefix('#').unwrap_or("");
        if custom.len() != 6
            || !custom.chars().all(|c| c.is_ascii_hexdigit())
            || !self.hud_opacity.is_finite()
            || !(0.6..=1.0).contains(&self.hud_opacity)
        {
            return Err("Invalid custom accent colour or HUD opacity".into());
        }
        if self
            .power_policy
            .battery_preset
            .as_deref()
            .is_some_and(|p| !matches!(p, "auto" | "fast" | "balanced" | "accurate"))
        {
            return Err("Invalid battery preset".into());
        }
        let mut ids = std::collections::HashSet::new();
        for mode in &self.modes {
            if mode.id.trim().is_empty() || mode.name.trim().is_empty() || !ids.insert(&mode.id) {
                return Err("Modes need unique IDs and nonempty names".into());
            }
            if mode.custom_instructions.chars().count() > 2000 {
                return Err("Mode instructions are limited to 2000 characters".into());
            }
            if !matches!(
                mode.send_key.as_str(),
                "enter" | "shift_enter" | "ctrl_enter"
            ) {
                return Err("Invalid mode send key".into());
            }
            if let Some(language) = &mode.translate_to {
                crate::asr::languages::resolve_language_name(language)?;
            }
            if let OutputAction::RunCommand { template } = &mode.output {
                if template.trim().is_empty() {
                    return Err("Command template must not be empty".into());
                }
            }
        }
        ids.clear();
        for snippet in &self.snippets {
            if snippet.id.trim().is_empty()
                || !ids.insert(&snippet.id)
                || snippet.trigger.trim().is_empty()
                || snippet.trigger.chars().count() > 60
                || snippet.expansion.chars().count() > 4000
            {
                return Err("Snippets need unique IDs, a trigger of 1–60 characters and an expansion up to 4000 characters".into());
            }
        }
        if self
            .hotkeys
            .cancel
            .as_deref()
            .is_some_and(|s| s != "Escape")
        {
            return Err("The cancel hotkey must be Escape".into());
        }
        if !matches!(
            self.send_key.as_str(),
            "enter" | "shift_enter" | "ctrl_enter"
        ) {
            return Err("Invalid send key".into());
        }
        Ok(())
    }
    pub fn mode_for_process(&self, process: &str) -> &Mode {
        let triggered = self
            .mode_triggers_enabled
            .then(|| {
                self.modes.iter().find(|m| {
                    m.enabled
                        && m.triggers
                            .apps
                            .iter()
                            .any(|p| process_basename(p) == process_basename(process))
                })
            })
            .flatten();
        triggered
            .or_else(|| {
                self.modes
                    .iter()
                    .find(|m| m.enabled && m.id == self.default_mode_id)
            })
            .unwrap_or_else(|| {
                static FALLBACK: std::sync::OnceLock<Mode> = std::sync::OnceLock::new();
                FALLBACK.get_or_init(Mode::default)
            })
    }

    pub fn resolve_mode(&self, process: &str) -> Mode {
        self.mode_for_process(process).clone()
    }

    pub fn settings_for_mode(&self, process: &str, mode: &Mode) -> Self {
        let mut effective = self.for_application(process);
        // The default Dictation mode follows the existing General/Cleanup controls.
        if mode.id != "dictation" {
            effective.style = mode.style.clone();
            effective.cleanup_level = mode.cleanup_level.clone();
            effective.intelligence_tier = mode.intelligence_tier.clone();
            effective.dictation_mode = mode.dictation_mode.clone();
            effective.auto_style_from_app = false;
        }
        if let Some(language) = &mode.language {
            effective.language = language.clone();
            effective.auto_detect_language = language == "auto";
        }
        effective
    }

    pub fn spoken_mode<'a>(&'a self, text: &'a str) -> Option<(&'a Mode, &'a str)> {
        if !self.mode_triggers_enabled {
            return None;
        }
        let lower = text.to_lowercase();
        self.modes.iter().filter(|m| m.enabled).find_map(|m| {
            m.triggers.spoken.iter().find_map(|trigger| {
                let trigger = trigger.trim();
                if trigger.is_empty() || trigger.split_whitespace().count() > 5 {
                    return None;
                }
                let length = trigger.len();
                if lower.starts_with(&trigger.to_lowercase())
                    && text.is_char_boundary(length)
                    && text[length..]
                        .chars()
                        .next()
                        .is_none_or(|c| !c.is_alphanumeric())
                {
                    Some((
                        m,
                        text[length..].trim_start_matches(|c: char| {
                            c.is_whitespace() || matches!(c, ',' | ':' | '.' | '!')
                        }),
                    ))
                } else {
                    None
                }
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn app_trigger_beats_default_and_disabled_modes_are_skipped() {
        let mut s = AppSettings::default();
        s.modes[1].triggers.apps.push("C:\\Apps\\Chat.EXE".into());
        assert_eq!(s.resolve_mode("chat.exe").id, "message");
        s.modes[1].enabled = false;
        assert_eq!(s.resolve_mode("chat.exe").id, "dictation");
    }
    #[test]
    fn spoken_trigger_requires_a_boundary() {
        let mut s = AppSettings::default();
        s.modes[2].triggers.spoken.push("email mode".into());
        assert_eq!(s.spoken_mode("Email mode, hello").unwrap().1, "hello");
        assert!(s.spoken_mode("email model hello").is_none());
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PowerPolicy {
    pub unload_on_battery: bool,
    pub battery_preset: Option<String>,
}

impl AppSettings {
    pub fn process_is_excluded(process: &str, exclusions: &[String]) -> bool {
        let basename = |p: &str| p.rsplit(['/', '\\']).next().unwrap_or(p).to_lowercase();
        let process = basename(process);
        !process.is_empty() && exclusions.iter().any(|p| basename(p) == process)
    }
}
