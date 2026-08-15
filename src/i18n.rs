//! Traductions chargées depuis le dossier locales, avec repli sur le français embarqué.

use std::collections::HashMap;
use std::path::Path;

/// Base embarquée dans le binaire, garantit un fonctionnement sans dossier locales.
const FALLBACK: &str = include_str!("../locales/fr.json");

/// Traductions chargées. Les clés inconnues sont retournées telles quelles.
pub struct I18n {
    strings: HashMap<String, String>,
}

impl I18n {
    pub fn load(lang: &str, locales_dir: &Path) -> Self {
        let mut strings: HashMap<String, String> =
            serde_json::from_str(FALLBACK).expect("fr.json embarqué invalide");

        let file = locales_dir.join(format!("{lang}.json"));
        if lang != "fr" {
            if let Ok(raw) = std::fs::read_to_string(&file) {
                if let Ok(parsed) = serde_json::from_str::<HashMap<String, String>>(&raw) {
                    strings = parsed;
                }
            }
        }
        Self { strings }
    }

    /// Retourne la chaîne pour la clé, ou la clé elle-même si inconnue.
    pub fn t<'a>(&'a self, key: &'a str) -> &'a str {
        self.strings.get(key).map(String::as_str).unwrap_or(key)
    }
}
