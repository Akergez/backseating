//! Small, application-independent persistence for the project picker.
//! The file is deliberately plain JSON so Aiddiction and other hosts can
//! share it without depending on a UI implementation.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Project {
    pub path: PathBuf,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectStore {
    pub projects: Vec<Project>,
    #[serde(default)]
    pub last_project: Option<PathBuf>,
    #[serde(default)]
    pub code_font: Option<String>,
}

impl ProjectStore {
    pub fn config_path() -> Option<PathBuf> {
        dirs::config_dir().map(|dir| dir.join("backseating").join("projects.json"))
    }

    pub fn load() -> Result<Self, String> {
        let Some(path) = Self::config_path() else {
            return Ok(Self::default());
        };
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).map_err(|e| format!("reading projects: {e}")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("reading projects: {e}")),
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let Some(path) = Self::config_path() else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("creating project config: {e}"))?;
        }
        let text = serde_json::to_vec_pretty(self).map_err(|e| format!("writing projects: {e}"))?;
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, text).map_err(|e| format!("writing projects: {e}"))?;
        std::fs::rename(temporary, path).map_err(|e| format!("saving projects: {e}"))
    }

    pub fn add(&mut self, path: impl Into<PathBuf>) {
        let path = path.into();
        if !self.projects.iter().any(|p| p.path == path) {
            self.projects.push(Project {
                path: path.clone(),
                name: None,
            });
        }
        self.last_project = Some(path);
    }

    pub fn remove(&mut self, path: &Path) {
        self.projects.retain(|p| p.path != path);
        if self.last_project.as_deref() == Some(path) {
            self.last_project = None;
        }
    }
}

/// Font choice persisted next to projects; hosts can expose this in Settings.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UiSettings {
    #[serde(default)]
    pub code_font: Option<String>,
}

impl Default for UiSettings {
    fn default() -> Self {
        Self { code_font: None }
    }
}
