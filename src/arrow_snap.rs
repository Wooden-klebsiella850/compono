//! Placement au clavier (Win+Flèche), équivalent de l'ancrage natif pendant qu'il
//! est désactivé. Une flèche place la fenêtre active sur une moitié d'écran ;
//! rappuyer sur la même flèche réduit cette moitié en quart (et inversement).
//! Combiner un axe horizontal et un axe vertical (ex : Gauche puis Haut) donne
//! un coin. Chaque fenêtre garde son propre état, tant qu'elle n'est pas oubliée.

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
}

#[derive(Debug, Clone, Copy, Default)]
struct WindowZones {
    horizontal: Option<(HDir, Level)>,
    vertical: Option<(VDir, Level)>,
}

/// Mémorise, par fenêtre (HWND en usize), la moitié/quart courant sur chaque axe.
#[derive(Default)]
pub struct ArrowSnap {
    zones: HashMap<usize, WindowZones>,
}

impl ArrowSnap {
    pub fn new() -> Self {
        Self::default()
    }

    /// Applique une flèche pour `window` et retourne le rectangle cible dans `area`.
    pub fn press(&mut self, window: usize, key: ArrowKey, area: Rect) -> Rect {
        let state = self.zones.entry(window).or_default();
        match key {
            ArrowKey::Left | ArrowKey::Right => {
                let dir = if key == ArrowKey::Left {
                    HDir::Left
                } else {
                    HDir::Right
                };
                state.horizontal = Some(match state.horizontal {
                    Some((d, Level::Half)) if d == dir => (dir, Level::Quarter),
                    Some((d, Level::Quarter)) if d == dir => (dir, Level::Half),
                    _ => (dir, Level::Half),
                });
            }
            ArrowKey::Up | ArrowKey::Down => {
                let dir = if key == ArrowKey::Up {
                    VDir::Top
                } else {
                    VDir::Bottom
                };
                state.vertical = Some(match state.vertical {
                    Some((d, Level::Half)) if d == dir => (dir, Level::Quarter),
                    Some((d, Level::Quarter)) if d == dir => (dir, Level::Half),
                    _ => (dir, Level::Half),
                });
            }
        }
        rect_for(*state, area)
    }

    /// Oublie l'état d'une fenêtre (fermeture, replacement manuel...).
    #[allow(dead_code)]
    pub fn forget(&mut self, window: usize) {
        self.zones.remove(&window);
    }
}

fn rect_for(state: WindowZones, area: Rect) -> Rect {
    let width = match state.horizontal {
        Some((_, Level::Half)) => area.width / 2,
        Some((_, Level::Quarter)) => area.width / 4,
        None => area.width,
    };
    let height = match state.vertical {
        Some((_, Level::Half)) => area.height / 2,
        Some((_, Level::Quarter)) => area.height / 4,
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
    fn gauche_place_la_moitie_gauche() {
        let mut snap = ArrowSnap::new();
        let rect = snap.press(1, ArrowKey::Left, AREA);
        assert_eq!(rect, Rect::new(0, 0, 960, 1080));
    }

    #[test]
    fn rappuyer_gauche_reduit_en_quart_puis_revient_a_la_moitie() {
        let mut snap = ArrowSnap::new();
        snap.press(1, ArrowKey::Left, AREA);
        let quarter = snap.press(1, ArrowKey::Left, AREA);
        assert_eq!(quarter, Rect::new(0, 0, 480, 1080));
        let half_again = snap.press(1, ArrowKey::Left, AREA);
        assert_eq!(half_again, Rect::new(0, 0, 960, 1080));
    }

    #[test]
    fn droite_place_la_moitie_droite() {
        let mut snap = ArrowSnap::new();
        let rect = snap.press(1, ArrowKey::Right, AREA);
        assert_eq!(rect, Rect::new(960, 0, 960, 1080));
    }

    #[test]
    fn changer_de_cote_repart_de_la_moitie() {
        let mut snap = ArrowSnap::new();
        snap.press(1, ArrowKey::Left, AREA);
        snap.press(1, ArrowKey::Left, AREA); // quart gauche
        let rect = snap.press(1, ArrowKey::Right, AREA);
        assert_eq!(rect, Rect::new(960, 0, 960, 1080));
    }

    #[test]
    fn combiner_gauche_et_haut_donne_un_coin() {
        let mut snap = ArrowSnap::new();
        snap.press(1, ArrowKey::Left, AREA);
        let corner = snap.press(1, ArrowKey::Up, AREA);
        assert_eq!(corner, Rect::new(0, 0, 960, 540));
    }

    #[test]
    fn rappuyer_haut_dans_un_coin_ne_touche_que_la_hauteur() {
        let mut snap = ArrowSnap::new();
        snap.press(1, ArrowKey::Left, AREA);
        snap.press(1, ArrowKey::Up, AREA);
        let rect = snap.press(1, ArrowKey::Up, AREA);
        assert_eq!(rect, Rect::new(0, 0, 960, 270));
    }

    #[test]
    fn bas_droite_ancre_sur_le_coin_oppose() {
        let mut snap = ArrowSnap::new();
        snap.press(1, ArrowKey::Right, AREA);
        let corner = snap.press(1, ArrowKey::Down, AREA);
        assert_eq!(corner, Rect::new(960, 540, 960, 540));
    }

    #[test]
    fn chaque_fenetre_garde_son_propre_etat() {
        let mut snap = ArrowSnap::new();
        snap.press(1, ArrowKey::Left, AREA);
        let rect_2 = snap.press(2, ArrowKey::Right, AREA);
        assert_eq!(rect_2, Rect::new(960, 0, 960, 1080));
        let rect_1_again = snap.press(1, ArrowKey::Left, AREA);
        assert_eq!(rect_1_again, Rect::new(0, 0, 480, 1080));
    }
}
