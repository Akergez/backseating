//! What is chosen on the settings page, kept in `settings.json` beside the
//! projects. The projects stay in their own file, which other hosts of the
//! review read too; this one is the application's alone.

use std::path::PathBuf;

use gpui_kit::{App, Global};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub appearance: Appearance,
}

/// Every field is optional: nothing chosen is the default, and a file from a
/// later version that has more is still read.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Appearance {
    /// `light`, `dark`, or nothing for the system's.
    #[serde(default)]
    pub mode: Option<String>,
    /// `#rrggbb` the scheme is made from, or nothing for the system's.
    #[serde(default)]
    pub color: Option<String>,
    /// What the system said its colour was, to start in it next time.
    #[serde(default)]
    pub system_color: Option<String>,
    #[serde(default)]
    pub light_theme: Option<String>,
    #[serde(default)]
    pub dark_theme: Option<String>,
    #[serde(default)]
    pub ui_font: Option<String>,
    #[serde(default)]
    pub code_font: Option<String>,
}

impl Global for Settings {}

impl Settings {
    fn path() -> Option<PathBuf> {
        dirs::config_dir().map(|dir| dir.join("backseating").join("settings.json"))
    }

    /// The file, or the defaults when there is none or it cannot be read: a
    /// broken settings file must not keep the window from opening.
    pub fn load() -> Self {
        Self::path()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    fn save(&self) -> Result<(), String> {
        let Some(path) = Self::path() else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("creating settings: {e}"))?;
        }
        let text = serde_json::to_vec_pretty(self).map_err(|e| format!("writing settings: {e}"))?;
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, text).map_err(|e| format!("writing settings: {e}"))?;
        std::fs::rename(temporary, path).map_err(|e| format!("saving settings: {e}"))
    }

    pub fn get(cx: &App) -> &Self {
        cx.global::<Self>()
    }

    /// Changes the settings and writes them out.
    pub fn update(cx: &mut App, change: impl FnOnce(&mut Self)) {
        let settings = cx.global_mut::<Self>();
        change(settings);
        if let Err(error) = settings.save() {
            eprintln!("backseating: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_or_older_file_is_the_defaults() {
        assert_eq!(
            serde_json::from_str::<Settings>("{}").unwrap(),
            Settings::default()
        );
        let settings: Settings =
            serde_json::from_str(r#"{"appearance":{"mode":"dark","future":1}}"#).unwrap();
        assert_eq!(settings.appearance.mode.as_deref(), Some("dark"));
    }
}
