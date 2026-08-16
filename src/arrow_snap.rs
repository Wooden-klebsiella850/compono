//! Placement au clavier (Ctrl+Alt+FlÃ¨che).
//! - FlÃ¨ches Gauche/Droite : MoitiÃ© (50% L, 100% H) -> Quart (25% L, 100% H) -> Deux Tiers (66% L, 100% H).
//!   (RÃ©initialise toujours la hauteur Ã  100% si on venait d'un coin ou d'une moitiÃ© haute/basse).
//! - FlÃ¨che Haut depuis un cÃ´tÃ© : Coin (50% L, 50% H) -> Coin fin (50% L, 25% H) -> MoitiÃ© haute (100% L, 50% H) -> Plein Ã©cran (100% L, 100% H).
//! - FlÃ¨che Bas depuis un cÃ´tÃ© : Coin (50% L, 50% H) -> Coin fin (50% L, 25% H) -> MoitiÃ© basse (100% L, 50% H).

use std::collections::HashMap;

use crate::grid::Rect;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArrowKey {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HDir {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VDir {
    Top,
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Level {
    Half,
    Quarter,
    TwoThirds,
    Full,
}

#[derive(Debug, Clone, Copy, Default)]
struct WindowZones {
    horizontal: Option<(HDir, Level)>,
    vertical: Option<(VDir, Level)>,
}

/// MÃ©morise, par fenÃªtre (HWND en usize), la zone courante sur chaque axe.
#[derive(Default)]
pub struct ArrowSnap {
    zones: HashMap<usize, WindowZones>,
}

impl ArrowSnap {
    pub fn new() -> Self {
        Self::default()
    }

    /// Applique une flÃ¨che pour `window` et retourne le rectangle cible dans `area`.
    pub fn press(&mut self, window: usize, key: ArrowKey, area: Rect) -> Rect {
        let state = self.zones.entry(window).or_default();
        match key {
            ArrowKey::Left | ArrowKey::Right => {
                let dir = if key == ArrowKey::Left {
                    HDir::Left
                } else {
                    HDir::Right
                };

                // Si on Ã©tait en coin, moitiÃ© haute/basse ou plein Ã©cran, appuyer sur Gauche/Droite
                // rÃ©initialise l'axe vertical pour garantir 100% de hauteur !
                let reset_vertical = state.vertical.is_some();
                state.vertical = None;

                if reset_vertical {
                    // Passage direct en MoitiÃ© (50% largeur, 100% hauteur)
                    state.horizontal = Some((dir, Level::Half));
                } else {
                    // Cycle horizontal : MoitiÃ© -> Quart -> Deux Tiers -> MoitiÃ©...
                    state.horizontal = Some(match state.horizontal {
                        Some((d, Level::Half)) if d == dir => (dir, Level::Quarter),
                        Some((d, Level::Quarter)) if d == dir => (dir, Level::TwoThirds),
                        Some((d, Level::TwoThirds)) if d == dir => (dir, Level::Half),
                        _ => (dir, Level::Half),
                    });
                }
            }
            ArrowKey::Up => {
                if let Some((_hdir, _hlvl)) = state.horizontal {
                    match state.vertical {
                        None => {
                            // 1Ã¨re fois : Coin supÃ©rieur (50% L, 50% H = 1/4 Ã©cran)
                            state.vertical = Some((VDir::Top, Level::Half));
                        }
                        Some((VDir::Top, Level::Half)) => {
                            // 2Ã¨me fois : Coin supÃ©rieur plus fin (50% L, 25% H = 1/8 Ã©cran)
                            state.vertical = Some((VDir::Top, Level::Quarter));
                        }
                        Some((VDir::Top, Level::Quarter)) => {
                            // 3Ã¨me fois : MoitiÃ© supÃ©rieure complÃ¨te (100% L, 50% H = 1/2 Ã©cran)
                            state.horizontal = None;
                            state.vertical = Some((VDir::Top, Level::Half));
                        }
                        _ => {
                            state.vertical = Some((VDir::Top, Level::Half));
                        }
                    }
                } else {
                    // Pas d'ancrage horizontal (mode vertical pur) :
                    state.vertical = match state.vertical {
                        None => Some((VDir::Top, Level::Half)),
                        Some((VDir::Top, Level::Half)) => Some((VDir::Top, Level::Quarter)),
                        Some((VDir::Top, Level::Quarter)) => Some((VDir::Top, Level::Full)),
                        Some((VDir::Top, Level::Full)) => Some((VDir::Top, Level::Half)),
                        _ => Some((VDir::Top, Level::Half)),
                    };
                }
            }
            ArrowKey::Down => {
                if let Some((_hdir, _hlvl)) = state.horizontal {
                    match state.vertical {
                        None => {
                            // 1Ã¨re fois : Coin infÃ©rieur (50% L, 50% H = 1/4 Ã©cran)
                            state.vertical = Some((VDir::Bottom, Level::Half));
                        }
                        Some((VDir::Bottom, Level::Half)) => {
                            // 2Ã¨me fois : Coin infÃ©rieur plus fin (50% L, 25% H = 1/8 Ã©cran)
                            state.vertical = Some((VDir::Bottom, Level::Quarter));
                        }
                        Some((VDir::Bottom, Level::Quarter)) => {
                            // 3Ã¨me fois : MoitiÃ© infÃ©rieure complÃ¨te (100% L, 50% H = 1/2 Ã©cran)
                            state.horizontal = None;
                            state.vertical = Some((VDir::Bottom, Level::Half));
                        }
                        _ => {
                            state.vertical = Some((VDir::Bottom, Level::Half));
                        }
                    }
                } else {
                    // Pas d'ancrage horizontal (mode vertical pur) :
                    state.vertical = match state.vertical {
                        None => Some((VDir::Bottom, Level::Half)),
                        Some((VDir::Bottom, Level::Half)) => Some((VDir::Bottom, Level::Quarter)),
                        Some((VDir::Bottom, Level::Quarter)) => Some((VDir::Bottom, Level::Half)),
                        _ => Some((VDir::Bottom, Level::Half)),
                    };
                }
            }
        }
        rect_for(*state, area)
    }

    /// Oublie l'Ã©tat d'une fenÃªtre.
    #[allow(dead_code)]
    pub fn forget(&mut self, window: usize) {
        self.zones.remove(&window);
    }
}

fn rect_for(state: WindowZones, area: Rect) -> Rect {
    let width = match state.horizontal {
        Some((_, Level::Half)) => area.width / 2,
        Some((_, Level::Quarter)) => area.width / 4,
        Some((_, Level::TwoThirds)) => (area.width * 2) / 3,
        Some((_, Level::Full)) => area.width,
        None => area.width,
    };
    let height = match state.vertical {
        Some((_, Level::Half)) => area.height / 2,
        Some((_, Level::Quarter)) => area.height / 4,
        Some((_, Level::TwoThirds)) => (area.height * 2) / 3,
        Some((_, Level::Full)) => area.height,
        None => area.height,
    };
    let x = match state.horizontal {
        Some((HDir::Right, _)) => area.right() - width as i32,
        _ => area.x,
    };
    let y = match state.vertical {
        Some((VDir::Bottom, _)) => area.bottom() - height as i32,
        _ => area.y,
    };
    Rect {
        x,
        y,
        width,
        height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rect = Rect {
        x: 0,
        y: 0,
        width: 1920,
        height: 1080,
    };

    #[test]
    fn cycle_horizontal_moitie_quart_deux_tiers() {
        let mut snap = ArrowSnap::new();
        // 1er appui : MoitiÃ© gauche (50% L, 100% H)
        let rect1 = snap.press(1, ArrowKey::Left, AREA);
        assert_eq!(rect1, Rect::new(0, 0, 960, 1080));

        // 2e appui : Quart gauche (25% L, 100% H)
        let rect2 = snap.press(1, ArrowKey::Left, AREA);
        assert_eq!(rect2, Rect::new(0, 0, 480, 1080));

        // 3e appui : Deux tiers gauche (66.6% L, 100% H)
        let rect3 = snap.press(1, ArrowKey::Left, AREA);
        assert_eq!(rect3, Rect::new(0, 0, 1280, 1080));

        // 4e appui : Retour MoitiÃ© gauche (50% L, 100% H)
        let rect4 = snap.press(1, ArrowKey::Left, AREA);
        assert_eq!(rect4, Rect::new(0, 0, 960, 1080));
    }

    #[test]
    fn retour_a_100_pourcent_hauteur_depuis_un_coin() {
        let mut snap = ArrowSnap::new();
        // Gauche -> 50% L, 100% H
        snap.press(1, ArrowKey::Left, AREA);
        // Haut -> Coin 50% L, 50% H
        let corner = snap.press(1, ArrowKey::Up, AREA);
        assert_eq!(corner, Rect::new(0, 0, 960, 540));

        // RÃ©appui sur Gauche -> Doit rÃ©initialiser la hauteur Ã  100% (50% L, 100% H) !
        let full_height = snap.press(1, ArrowKey::Left, AREA);
        assert_eq!(full_height, Rect::new(0, 0, 960, 1080));
    }

    #[test]
    fn cycle_vertical_depuis_coin() {
        let mut snap = ArrowSnap::new();
        snap.press(1, ArrowKey::Left, AREA);
        // 1er Haut : Coin (960x540)
        let corner = snap.press(1, ArrowKey::Up, AREA);
        assert_eq!(corner, Rect::new(0, 0, 960, 540));

        // 2e Haut : Coin fin (960x270)
        let fine = snap.press(1, ArrowKey::Up, AREA);
        assert_eq!(fine, Rect::new(0, 0, 960, 270));

        // 3e Haut : MoitiÃ© supÃ©rieure complÃ¨te (1920x540) !
        let half_top = snap.press(1, ArrowKey::Up, AREA);
        assert_eq!(half_top, Rect::new(0, 0, 1920, 540));

        // 4e Haut : Plein Ã©cran (1920x1080) !
        let full = snap.press(1, ArrowKey::Up, AREA);
        assert_eq!(full, Rect::new(0, 0, 1920, 1080));
    }
}