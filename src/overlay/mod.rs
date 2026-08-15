//! Overlays : une fenêtre layered par écran, dessinée en D2D via DComp.

pub mod render;
mod window;

use windows::core::Result;

use crate::grid::GridOptions;
use crate::monitors;

use render::{Gfx, OverlayState};
use window::OverlayWindow;

/// Gère les overlays : une fenêtre par écran, visibles ou non.
pub struct OverlayManager {
    gfx: Option<Gfx>,
    windows: Vec<OverlayWindow>,
    options: GridOptions,
    visible: bool,
}

impl OverlayManager {
    pub fn new(options: GridOptions) -> Self {
        Self {
            gfx: None,
            windows: Vec::new(),
            options,
            visible: false,
        }
    }

    #[allow(dead_code)]
    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Affiche les overlays, en créant les fenêtres à la première apparition.
    #[allow(dead_code)]
    pub fn show(&mut self) -> Result<()> {
        if self.windows.is_empty() {
            self.build_windows()?;
        }
        for window in &self.windows {
            window.show();
            window.render(&OverlayState::default())?;
        }
        self.visible = true;
        Ok(())
    }

    /// Affiche la grille, avec halo et sélection sur l'écran actif (drag).
    pub fn show_with_state(
        &mut self,
        active: usize,
        halo: Option<render::HaloState>,
    ) -> Result<()> {
        if self.windows.is_empty() {
            self.build_windows()?;
        }
        for (index, window) in self.windows.iter().enumerate() {
            window.show();
            let state = if index == active {
                render::OverlayState {
                    halo,
                    selection: None,
                }
            } else {
                render::OverlayState::default()
            };
            window.render(&state)?;
        }
        self.visible = true;
        Ok(())
    }

    /// Met à jour l'état dessiné sur un écran.
    pub fn update(&self, monitor: usize, state: &render::OverlayState) -> Result<()> {
        if let Some(window) = self.windows.get(monitor) {
            window.render(state)?;
        }
        Ok(())
    }

    pub fn hide(&mut self) {
        for window in &self.windows {
            window.hide();
        }
        self.visible = false;
    }

    #[allow(dead_code)]
    pub fn toggle(&mut self) -> Result<()> {
        if self.visible {
            self.hide();
            Ok(())
        } else {
            self.show()
        }
    }

    /// Reconstruit les fenêtres après un changement d'écrans.
    pub fn on_display_change(&mut self) -> Result<()> {
        self.destroy_windows();
        if self.visible {
            self.build_windows()?;
            for window in &self.windows {
                window.show();
                window.render(&OverlayState::default())?;
            }
        }
        Ok(())
    }

    fn build_windows(&mut self) -> Result<()> {
        let options = self.options;
        if self.gfx.is_none() {
            self.gfx = Some(Gfx::new()?);
        }
        let monitors = monitors::current();
        let mut windows = Vec::new();
        {
            let gfx = self.gfx.as_ref().expect("gfx initialisé");
            for monitor in &monitors {
                match OverlayWindow::new(gfx, monitor, options) {
                    Ok(window) => windows.push(window),
                    // DIAG : à retirer.
                    Err(err) => log::error!("création overlay échouée : {err}"),
                }
            }
        }
        // DIAG : à retirer.
        log::info!("overlays construits : {}/{}", windows.len(), monitors.len());
        self.windows = windows;
        Ok(())
    }

    fn destroy_windows(&mut self) {
        for window in self.windows.drain(..) {
            window.destroy();
        }
    }
}

impl Drop for OverlayManager {
    fn drop(&mut self) {
        self.destroy_windows();
    }
}
