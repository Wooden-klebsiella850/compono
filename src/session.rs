//! Machine à états du geste de placement.
//!
//! Workflow :
//! 1. Idle : en attente.
//! 2. DraggingWindow : l'utilisateur déplace une fenêtre.
//! 3. DraggingInBand : la fenêtre est maintenue tout au bord de l'écran pendant plus de 0.5 seconde
//!    -> affichage de la grille avec halo.
//! 4. AwaitingSelection : l'utilisateur lâche l'application sur la grille. La fenêtre est temporairement
//!    masquée/réduite pour dégager la vue, et la grille attend que l'utilisateur clique et glisse
//!    pour tracer son rectangle de destination.
//! 5. DrawingSelection : l'utilisateur trace le rectangle de sélection (du clic d'ancrage jusqu'au curseur).
//! 6. Relâchement (MouseUp) : la fenêtre cible est automatiquement restaurée, redimensionnée et placée
//!    sur le rectangle tracé, puis la grille se masque et l'état revient à Idle.
//! 7. Annulation (clic droit ou en dehors) : la fenêtre cible est restaurée et la grille se masque.

use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::UI::WindowsAndMessaging::{GetCursorPos, GetForegroundWindow, WindowFromPoint};

use crate::grid::{Cell, Grid, GridOptions, Rect};
use crate::hooks::InputEvent;
use crate::monitors::{self, MonitorInfo, ScreenEdge};
use crate::placement::{get_top_level_window_for_hwnd, is_placeable_window};

/// Largeur de la bande de déclenchement tout au bord d'un écran, en pixels.
const EDGE_BAND_PX: u32 = 35;
/// Distance minimale de déplacement pour détecter un drag de fenêtre.
const DRAG_THRESHOLD_PX: i32 = 8;
/// Durée minimale de maintien au bord de l'écran avant déclenchement (0.5 seconde).
pub const DWELL_DURATION: Duration = Duration::from_millis(250);
/// Épaisseur du halo affiché sur le bord déclencheur.
pub const HALO_THICKNESS: f32 = 140.0;

/// Action à appliquer à l'overlay et aux fenêtres par le code appelant.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SessionAction {
    None,
    /// Affiche la grille avec un halo optionnel sur le bord d'un écran.
    ShowGrid {
        monitor: usize,
        edge: Option<ScreenEdge>,
    },
    /// Masque la grille, sans modifier la fenêtre en cours de déplacement.
    HideGrid,
    /// L'application a été déposée : masquer la fenêtre cible et attendre le tracé du rectangle.
    AwaitingSelection {
        monitor: usize,
        target_window: Option<usize>,
    },
    /// Met à jour la sélection sur un écran.
    UpdateGrid {
        monitor: usize,
        selection: Option<Rect>,
    },
    /// Annule la session, masque la grille et restaure la fenêtre cible.
    Cancel {
        target_window: Option<usize>,
    },
    /// Place la fenêtre sur le rectangle tracé et la réaffiche.
    Place {
        target_window: Option<usize>,
        rect: Rect,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Idle,
    DraggingWindow,
    DraggingInBand,
    AwaitingDragEnd,
    AwaitingSelection,
    DrawingSelection,
}

pub struct Session {
    state: State,
    options: GridOptions,
    target_window: Option<usize>,
    mouse_down: bool,
    press_pos: Option<(i32, i32)>,
    last_pos: (i32, i32),
    band_enter_time: Option<Instant>,
    band_candidate: Option<(usize, ScreenEdge, Grid)>,
    monitor: Option<usize>,
    grid: Option<Grid>,
    edge: Option<ScreenEdge>,
    anchor: Option<Cell>,
    current_rect: Option<Rect>,
}

impl Session {
    pub fn new(options: GridOptions) -> Self {
        Self {
            state: State::Idle,
            options,
            target_window: None,
            mouse_down: false,
            press_pos: None,
            last_pos: (0, 0),
            band_enter_time: None,
            band_candidate: None,
            monitor: None,
            grid: None,
            edge: None,
            anchor: None,
            current_rect: None,
        }
    }

    #[allow(dead_code)]
    pub fn state(&self) -> State {
        self.state
    }

    /// Ouvre la grille manuellement (via raccourci Win+Alt+G ou menu tray).
    pub fn open_manual(&mut self, monitor_index: Option<usize>) -> SessionAction {
        let (index, monitor) = match monitor_index {
            Some(i) => (i, monitors::current().get(i).copied()),
            None => {
                let (x, y) = self.last_pos;
                match monitor_at(x, y) {
                    Some((i, m)) => (i, Some(m)),
                    None => (0, monitors::current().first().copied()),
                }
            }
        };

        if let Some(mon) = monitor {
            let work = monitors::effective_work(mon.work);
            if let Some(grid) = Grid::new(work, self.options) {
                let fg = unsafe { GetForegroundWindow() };
                let root = if is_placeable_window(fg) {
                    let r = get_top_level_window_for_hwnd(fg);
                    if !r.0.is_null() {
                        Some(r.0 as usize)
                    } else {
                        Some(fg.0 as usize)
                    }
                } else {
                    None
                };
                self.target_window = root;
                self.monitor = Some(index);
                self.grid = Some(grid);
                self.edge = None;
                self.anchor = None;
                self.current_rect = None;
                self.state = State::AwaitingSelection;
                return SessionAction::AwaitingSelection {
                    monitor: index,
                    target_window: root,
                };
            }
        }
        SessionAction::None
    }

    /// Appelé périodiquement par la boucle de messages pour vérifier l'expiration
    /// du délai de 0.5 seconde au bord même lorsque la souris reste immobile.
    pub fn tick(&mut self) -> SessionAction {
        // Certains hôtes XAML, dont Windows Terminal, envoient les WinEvents de
        // déplacement sans transmettre tous les MouseMove au hook bas niveau.
        // Lire le curseur directement garde le déclenchement indépendant de ce flux.
        let should_poll_cursor = ((self.state == State::DraggingWindow
            && self.band_candidate.is_none())
            || self.state == State::DraggingInBand)
            || self.state == State::DrawingSelection;
        if should_poll_cursor {
            let mut cursor = POINT::default();
            if unsafe { GetCursorPos(&mut cursor) }.is_ok()
                && self.last_pos != (cursor.x, cursor.y)
            {
                self.last_pos = (cursor.x, cursor.y);
                return self.on_move(cursor.x, cursor.y);
            }
        }
        if self.state == State::DraggingWindow {
            if let (Some(start), Some((monitor, edge, grid))) =
                (self.band_enter_time, self.band_candidate.take())
            {
                if start.elapsed() >= DWELL_DURATION {
                    self.state = State::DraggingInBand;
                    self.monitor = Some(monitor);
                    self.grid = Some(grid);
                    self.edge = Some(edge);
                    self.band_enter_time = None;
                    return SessionAction::ShowGrid {
                        monitor,
                        edge: Some(edge),
                    };
                } else {
                    self.band_candidate = Some((monitor, edge, grid));
                }
            }
        }
        SessionAction::None
    }

    pub fn handle(&mut self, event: InputEvent) -> SessionAction {
        match event {
            InputEvent::MouseMove { x, y } => {
                self.last_pos = (x, y);
                self.on_move(x, y)
            }
            InputEvent::MouseDown { x, y } => {
                self.last_pos = (x, y);
                self.mouse_down = true;
                self.on_mouse_down(x, y)
            }
            InputEvent::MouseUp { x, y } => {
                self.last_pos = (x, y);
                self.mouse_down = false;
                self.press_pos = None;
                self.on_mouse_up(x, y)
            }
            InputEvent::RightDown => {
                if matches!(self.state, State::AwaitingSelection | State::DrawingSelection) {
                    let target = self.target_window;
                    self.reset();
                    SessionAction::Cancel {
                        target_window: target,
                    }
                } else {
                    SessionAction::None
                }
            }
            InputEvent::DragStart { hwnd } => {
                // Le hook souris a déjà capturé la fenêtre sous le pointeur au
                // MouseDown. Les applications XAML (dont Windows Terminal)
                // peuvent publier ici une surface interne : ne l'écrasons pas.
                if !self.mouse_down || self.target_window.is_none() {
                    self.target_window = hwnd.and_then(|raw| {
                        let h = HWND(raw as *mut core::ffi::c_void);
                        let root = get_top_level_window_for_hwnd(h);
                        if !root.0.is_null() {
                            Some(root.0 as usize)
                        } else {
                            Some(raw)
                        }
                    });
                }
                self.state = State::DraggingWindow;
                SessionAction::None
            }
            InputEvent::DragEnd => self.on_drag_end(),
            _ => SessionAction::None,
        }
    }

    fn on_move(&mut self, x: i32, y: i32) -> SessionAction {
        match self.state {
            State::Idle => {
                if self.mouse_down {
                    if let Some((px, py)) = self.press_pos {
                        if (x - px).abs() > DRAG_THRESHOLD_PX
                            || (y - py).abs() > DRAG_THRESHOLD_PX
                        {
                            self.state = State::DraggingWindow;
                            self.target_window = get_current_drag_window(x, y, px, py);
                        }
                    }
                }
                SessionAction::None
            }
            State::DraggingWindow => {
                // S'assurer d'avoir capturé la fenêtre active si elle ne l'était pas encore
                if self.target_window.is_none() {
                    let (px, py) = self.press_pos.unwrap_or((x, y));
                    self.target_window = get_current_drag_window(x, y, px, py);
                }

                match self.enter_band(x, y) {
                    Some((monitor, edge, grid)) => {
                        let is_same = self
                            .band_candidate
                            .as_ref()
                            .map(|(m, e, _)| *m == monitor && *e == edge)
                            .unwrap_or(false);

                        if !is_same || self.band_enter_time.is_none() {
                            self.band_enter_time = Some(Instant::now());
                            self.band_candidate = Some((monitor, edge, grid));
                        } else if let Some(start) = self.band_enter_time {
                            if start.elapsed() >= DWELL_DURATION {
                                self.state = State::DraggingInBand;
                                self.monitor = Some(monitor);
                                self.grid = Some(grid);
                                self.edge = Some(edge);
                                self.band_enter_time = None;
                                self.band_candidate = None;
                                return SessionAction::ShowGrid {
                                    monitor,
                                    edge: Some(edge),
                                };
                            }
                        }
                    }
                    None => {
                        self.band_enter_time = None;
                        self.band_candidate = None;
                    }
                }
                SessionAction::None
            }
            State::DraggingInBand => {
                if let Some((monitor, edge, grid)) = self.enter_band(x, y) {
                    if self.monitor != Some(monitor) || self.edge != Some(edge) {
                        self.monitor = Some(monitor);
                        self.grid = Some(grid);
                        self.edge = Some(edge);
                        SessionAction::ShowGrid {
                            monitor,
                            edge: Some(edge),
                        }
                    } else {
                        SessionAction::None
                    }
                } else {
                    self.state = State::DraggingWindow;
                    self.monitor = None;
                    self.grid = None;
                    self.edge = None;
                    SessionAction::HideGrid
                }
            }
            State::AwaitingDragEnd | State::AwaitingSelection => SessionAction::None,
            State::DrawingSelection => {
                if let (Some(grid), Some(anchor), Some(monitor)) =
                    (&self.grid, &self.anchor, self.monitor)
                {
                    if let Some(current) = grid.hit_test(x, y) {
                        let range = grid.selection(*anchor, current);
                        let rect = grid.cell_range_rect(range);
                        self.current_rect = Some(rect);
                        SessionAction::UpdateGrid {
                            monitor,
                            selection: Some(rect),
                        }
                    } else {
                        SessionAction::None
                    }
                } else {
                    SessionAction::None
                }
            }
        }
    }

    fn on_mouse_down(&mut self, x: i32, y: i32) -> SessionAction {
        match self.state {
            State::Idle => {
                self.press_pos = Some((x, y));
                self.target_window = get_current_drag_window(x, y, x, y);
                SessionAction::None
            }
            State::AwaitingSelection => {
                if let (Some(grid), Some(monitor)) = (&self.grid, self.monitor) {
                    if let Some(anchor) = grid.hit_test(x, y) {
                        // Clic sur la case tout en haut Ã  droite : fermeture/annulation de la grille
                        if anchor.col == grid.cols().saturating_sub(1) && anchor.row == 0 {
                            let target = self.target_window;
                            self.reset();
                            return SessionAction::Cancel {
                                target_window: target,
                            };
                        }

                        self.anchor = Some(anchor);
                        self.state = State::DrawingSelection;
                        let rect = grid.cell_rect(anchor);
                        self.current_rect = Some(rect);
                        SessionAction::UpdateGrid {
                            monitor,
                            selection: Some(rect),
                        }
                    } else {
                        let target = self.target_window;
                        self.reset();
                        SessionAction::Cancel {
                            target_window: target,
                        }
                    }
                } else {
                    let target = self.target_window;
                    self.reset();
                    SessionAction::Cancel {
                        target_window: target,
                    }
                }
            }
            State::DraggingWindow
            | State::DraggingInBand
            | State::AwaitingDragEnd
            | State::DrawingSelection => {
                SessionAction::None
            }
        }
    }

    fn on_mouse_up(&mut self, x: i32, y: i32) -> SessionAction {
        match self.state {
            State::DrawingSelection => {
                if let (Some(grid), Some(anchor)) = (&self.grid, &self.anchor) {
                    if anchor.col == grid.cols().saturating_sub(1) && anchor.row == 0 {
                        let current = grid.hit_test(x, y);
                        if current == Some(*anchor) || current.is_none() {
                            let target = self.target_window;
                            self.reset();
                            return SessionAction::Cancel {
                                target_window: target,
                            };
                        }
                    }
                }

                let rect = self.current_rect.or_else(|| {
                    if let (Some(grid), Some(anchor)) = (&self.grid, &self.anchor) {
                        let current = grid.hit_test(x, y).unwrap_or(*anchor);
                        Some(grid.cell_range_rect(grid.selection(*anchor, current)))
                    } else {
                        None
                    }
                });

                let target = self.target_window;
                self.reset();
                if let Some(rect) = rect {
                    SessionAction::Place {
                        target_window: target,
                        rect,
                    }
                } else {
                    SessionAction::Cancel {
                        target_window: target,
                    }
                }
            }
            State::DraggingInBand => {
                // Le système termine encore la boucle de déplacement ici.
                // Attendre DragEnd évite qu'il réaffiche la fenêtre après SW_HIDE.
                self.state = State::AwaitingDragEnd;
                self.edge = None;
                SessionAction::None
            }
            State::DraggingWindow => {
                self.reset();
                SessionAction::None
            }
            State::Idle | State::AwaitingDragEnd | State::AwaitingSelection => SessionAction::None,
        }
    }

    fn on_drag_end(&mut self) -> SessionAction {
        let (x, y) = self.last_pos;
        match self.state {
            State::DraggingInBand | State::AwaitingDragEnd => {
                self.state = State::AwaitingSelection;
                self.edge = None;
                if self.target_window.is_none() {
                    self.target_window = get_current_drag_window(x, y, x, y);
                }
                let target = self.target_window;
                if let Some(monitor) = self.monitor {
                    SessionAction::AwaitingSelection {
                        monitor,
                        target_window: target,
                    }
                } else {
                    SessionAction::None
                }
            }
            State::DraggingWindow => {
                self.reset();
                SessionAction::None
            }
            _ => SessionAction::None,
        }
    }

    fn enter_band(&self, x: i32, y: i32) -> Option<(usize, ScreenEdge, Grid)> {
        let (index, monitor) = monitor_at(x, y)?;
        let work = monitors::effective_work(monitor.work);
        let edge = edge_in_band(work, x, y, EDGE_BAND_PX)?;
        let grid = Grid::new(work, self.options)?;
        Some((index, edge, grid))
    }

    /// Annule la session en cours et retourne l'action d'annulation.
    pub fn cancel(&mut self) -> SessionAction {
        let target = self.target_window;
        self.reset();
        SessionAction::Cancel {
            target_window: target,
        }
    }

    /// Reset complet de la session.
    fn reset(&mut self) {
        self.state = State::Idle;
        self.target_window = None;
        self.press_pos = None;
        self.band_enter_time = None;
        self.band_candidate = None;
        self.monitor = None;
        self.grid = None;
        self.edge = None;
        self.anchor = None;
        self.current_rect = None;
    }
}

/// Index et infos du moniteur contenant le point.
fn monitor_at(x: i32, y: i32) -> Option<(usize, MonitorInfo)> {
    monitors::current()
        .iter()
        .enumerate()
        .find(|(_, m)| m.bounds.contains(x, y))
        .map(|(i, m)| (i, *m))
}

/// Bord de la zone utile dans la bande, sinon None.
fn edge_in_band(work: Rect, x: i32, y: i32, band: u32) -> Option<ScreenEdge> {
    let band = band as i32;
    if x <= work.x + band {
        Some(ScreenEdge::Left)
    } else if x >= work.right() - band {
        Some(ScreenEdge::Right)
    } else if y <= work.y + band {
        Some(ScreenEdge::Top)
    } else if y >= work.bottom() - band {
        Some(ScreenEdge::Bottom)
    } else {
        None
    }
}

/// Détermine la fenêtre en cours de déplacement (foreground ou sous le curseur),
/// résolue jusqu'à la véritable fenêtre de premier niveau (gère Windows Terminal, XAML, etc.).
fn get_current_drag_window(x: i32, y: i32, px: i32, py: i32) -> Option<usize> {
    if let Some(hwnd) = top_level_window_at(px, py).or_else(|| top_level_window_at(x, y)) {
        return Some(hwnd);
    }
    let fg = unsafe { GetForegroundWindow() };
    if is_placeable_window(fg) {
        let root = get_top_level_window_for_hwnd(fg);
        if !root.0.is_null() && is_placeable_window(root) {
            return Some(root.0 as usize);
        }
    }
    None
}

/// Remonte toujours à la fenêtre racine propriétaire (évite de cibler un contrôle enfant).
fn top_level_window_at(x: i32, y: i32) -> Option<usize> {
    unsafe {
        let hwnd = WindowFromPoint(POINT { x, y });
        if hwnd.0.is_null() {
            return None;
        }
        let root = get_top_level_window_for_hwnd(hwnd);
        let final_hwnd = if !root.0.is_null() { root } else { hwnd };
        if is_placeable_window(final_hwnd) {
            Some(final_hwnd.0 as usize)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::Rect;

    #[test]
    fn bande_sur_le_bord_gauche() {
        let work = Rect::new(0, 0, 1920, 1080);
        assert_eq!(edge_in_band(work, 10, 500, 35), Some(ScreenEdge::Left));
        assert_eq!(edge_in_band(work, 50, 500, 35), None);
    }

    #[test]
    fn bande_sur_le_bord_bas() {
        let work = Rect::new(0, 0, 1920, 1080);
        assert_eq!(edge_in_band(work, 1000, 1065, 35), Some(ScreenEdge::Bottom));
    }

    #[test]
    fn bande_sur_ecran_a_gauche_du_principal() {
        let work = Rect::new(-1920, 0, 1920, 1080);
        assert_eq!(
            edge_in_band(work, -1910, 500, 35),
            Some(ScreenEdge::Left)
        );
    }

    #[test]
    fn quitter_le_bord_masque_la_grille_et_rearme_le_drag() {
        let work = Rect::new(0, 0, 1920, 1080);
        let mut session = Session::new(GridOptions::default());
        session.state = State::DraggingInBand;
        session.monitor = Some(0);
        session.grid = Grid::new(work, GridOptions::default());
        session.edge = Some(ScreenEdge::Left);

        let action = session.on_move(500, 500);
        assert_eq!(action, SessionAction::HideGrid);
        assert_eq!(session.state(), State::DraggingWindow);
    }

    /// Geste complet avec délai de 0.5 seconde au bord :
    /// 1. Drag vers le bord gauche -> en attente du délai.
    /// 2. Attente de 0.5s (tick) -> grille affichée avec halo.
    /// 3. Lâcher de la fenêtre -> AwaitingSelection avec target_window masquée.
    /// 4. Clic & drag du rectangle sur la grille -> tracé de sélection.
    /// 5. Relâchement -> placement de la fenêtre cible !
    #[test]
    fn geste_complet_delai_puis_lacher_puis_tracer_rectangle() {
        let monitors = crate::monitors::current();
        let primary = monitors
            .iter()
            .find(|m| m.is_primary)
            .expect("écran principal");
        let work = crate::monitors::effective_work(primary.work);
        let mut session = Session::new(GridOptions::default());

        // 1. Début du drag de l'application (HWND = 42)
        let action = session.handle(InputEvent::DragStart { hwnd: Some(42) });
        assert_eq!(action, SessionAction::None);

        // Déplacement vers le bord gauche
        let action = session.handle(InputEvent::MouseMove {
            x: work.x + 10,
            y: work.y + 200,
        });
        // Immédiatement : pas encore d'affichage car délai < 0.5s
        assert_eq!(action, SessionAction::None);

        // Simuler l'écoulement de 0.5 seconde au bord
        session.band_enter_time = Some(Instant::now() - Duration::from_millis(300));
        let action = session.tick();
        match action {
            SessionAction::ShowGrid { edge, .. } => assert_eq!(edge, Some(ScreenEdge::Left)),
            other => panic!("attendu ShowGrid avec bord après 0.5s, obtenu {other:?}"),
        }

        // 2. Lâcher de l'application sur la grille : attendre que Windows quitte
        // sa boucle de déplacement avant de masquer la fenêtre.
        let action = session.handle(InputEvent::MouseUp {
            x: work.x + 10,
            y: work.y + 200,
        });
        assert_eq!(action, SessionAction::None);
        assert_eq!(session.state(), State::AwaitingDragEnd);

        let action = session.handle(InputEvent::DragEnd);
        assert_eq!(
            action,
            SessionAction::AwaitingSelection {
                monitor: 0,
                target_window: Some(42),
            }
        );
        assert_eq!(session.state(), State::AwaitingSelection);

        // 3. Clic pour démarrer la sélection du rectangle
        let action = session.handle(InputEvent::MouseDown {
            x: work.x + 50,
            y: work.y + 50,
        });
        assert!(matches!(
            action,
            SessionAction::UpdateGrid {
                selection: Some(_),
                ..
            }
        ));
        assert_eq!(session.state(), State::DrawingSelection);

        // Déplacement du curseur pour agrandir le rectangle
        let action = session.handle(InputEvent::MouseMove {
            x: work.x + 400,
            y: work.y + 300,
        });
        assert!(matches!(
            action,
            SessionAction::UpdateGrid {
                selection: Some(_),
                ..
            }
        ));

        // 4. Relâchement du clic -> placement automatique sur le rectangle tracé
        let action = session.handle(InputEvent::MouseUp {
            x: work.x + 400,
            y: work.y + 300,
        });
        match action {
            SessionAction::Place { target_window, rect } => {
                assert_eq!(target_window, Some(42));
                assert!(rect.width > 0 && rect.height > 0);
            }
            other => panic!("attendu Place, obtenu {other:?}"),
        }
        assert_eq!(session.state(), State::Idle);
    }
    #[test]
    fn clic_case_haut_droite_ferme_la_grille() {
        let work = Rect::new(0, 0, 1920, 1080);
        let mut session = Session::new(GridOptions::default());
        let grid = Grid::new(work, GridOptions::default()).unwrap();
        session.state = State::AwaitingSelection;
        session.monitor = Some(0);
        session.grid = Some(grid);

        let top_right_cell = Cell { col: 19, row: 0 };
        let cell_rect = grid.cell_rect(top_right_cell);
        let action = session.handle(InputEvent::MouseDown {
            x: cell_rect.x + (cell_rect.width as i32 / 2),
            y: cell_rect.y + (cell_rect.height as i32 / 2),
        });

        assert_eq!(action, SessionAction::Cancel { target_window: None });
        assert_eq!(session.state(), State::Idle);
    }
}