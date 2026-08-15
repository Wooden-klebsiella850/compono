# Compono

Overlay de placement de fenêtres sur grille pour Windows.

## État

Phase 0 : squelette. Instance unique, fenêtre cachée, boucle de messages, log fichier,
configuration TOML, i18n en/fr.

## Structure

- `src/main.rs` : point d'entrée, boucle de messages, fenêtre cachée
- `src/config.rs` : configuration TOML, %APPDATA%\Compono\config.toml
- `src/i18n.rs` : traductions en/fr
- `src/logging.rs` : log fichier
- `src/single_instance.rs` : instance unique et notification inter-instances
- `res/` : manifeste PerMonitorV2, icône
- `locales/` : fichiers de traduction
- `tools/make_ico.py` : régénère l'icône depuis le PNG source

## Construire

```
cargo build
```
