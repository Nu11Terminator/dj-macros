use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Config {
    /// Length of the volume fade-out, in seconds.
    pub fade_seconds: f32,
}

impl Default for Config {
    fn default() -> Self {
        Self { fade_seconds: 2.5 }
    }
}

fn config_path() -> Option<PathBuf> {
    let mut dir = dirs::config_dir()?;
    dir.push("fade-and-skip");
    std::fs::create_dir_all(&dir).ok()?;
    dir.push("config.json");
    Some(dir)
}

impl Config {
    pub fn load() -> Self {
        config_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        if let Some(path) = config_path() {
            if let Ok(s) = serde_json::to_string_pretty(self) {
                let _ = std::fs::write(path, s);
            }
        }
    }
}
