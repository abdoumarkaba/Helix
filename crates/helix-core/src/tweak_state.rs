use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TweakState {
    #[serde(default)]
    pub applied: BTreeMap<String, String>,
}

impl TweakState {
    pub fn load() -> Self {
        let path = Self::path();
        if let Ok(contents) = fs::read_to_string(&path) {
            serde_json::from_str(&contents).unwrap_or_default()
        } else {
            Self::default()
        }
    }

    pub fn path() -> PathBuf {
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("~/.local/share"))
            .join("helix")
            .join("tweak_state.json")
    }

    pub fn save(&self) {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = fs::write(path, json);
        }
    }

    pub fn record_applied(&mut self, tweak_id: &str, value: &str) {
        self.applied.insert(tweak_id.to_string(), value.to_string());
    }

    pub fn get(&self, tweak_id: &str) -> Option<&String> {
        self.applied.get(tweak_id)
    }
}
