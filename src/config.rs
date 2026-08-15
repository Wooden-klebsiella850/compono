//! Configuration TOML chargée depuis %APPDATA%\Compono\config.toml.

use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Deserialize, Clone)]
#[serde(default)]
pub struct Config {
    /// Langue de l'interface. None = détection système.
    pub lang: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self { lang: None }
    }
}

/// Chemin du fichier de configuration.
pub fn config_path(appdata: &Path) -> PathBuf {
    appdata.join("config.toml")
}

/// Charge la configuration, repli sur les valeurs par défaut.
pub fn load(path: &Path) -> Config {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| toml::from_str(&raw).ok())
        .unwrap_or_default()
}
