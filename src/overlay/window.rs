//! Fenêtre overlay : une fenêtre layered click-through par écran.

use windows::core::{Result, w};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::grid::{Grid, GridOptions, Rect};
use crate::monitors::MonitorInfo;

use super::render::{Gfx, OverlayState, Renderer};

/// Une fenêtre overlay couvrant un écran complet, avec sa grille et son rendu.
pub struct OverlayWindow {
    pub hwnd: HWND,
    /// Périmètre complet de l'écran, coordonnées absolues.
    pub bounds: Rect,
    grid: Option<Grid>,
    renderer: Renderer,
}

impl OverlayWindow {
    pub fn new(gfx: &Gfx, monitor: &MonitorInfo, options: GridOptions) -> Result<OverlayWindow> {
        let bounds = monitor.bounds;
        let hwnd = create_overlay_window(bounds)?;
        let renderer = Renderer::new(gfx, hwnd, bounds.width, bounds.height)?;
        let grid = Grid::new(monitor.work, options);
        Ok(OverlayWindow {
            hwnd,
            bounds,
            grid,
            renderer,
        })
    }

    /// Affiche la fenêtre sans l'activer.
    pub fn show(&self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
        }
    }

    pub fn hide(&self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
    }

    pub fn render(&self, state: &OverlayState) -> Result<()> {
        if let Some(grid) = &self.grid {
            self.renderer
                .render(grid, (self.bounds.x, self.bounds.y), state)?;
        }
        Ok(())
    }

    /// Détruit la fenêtre et libère le rendu, dans le bon ordre.
    pub fn destroy(self) {
        drop(self.renderer);
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

fn create_overlay_window(bounds: Rect) -> Result<HWND> {
    unsafe {
        let hinstance = GetModuleHandleW(None)?;
        let wc = WNDCLASSW {
            lpfnWndProc: Some(overlay_wnd_proc),
            hInstance: HINSTANCE(hinstance.0),
            lpszClassName: w!("Compono.Overlay"),
            ..Default::default()
        };
        // La classe peut déjà exister, l'erreur est alors sans conséquence.
        RegisterClassW(&wc);
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED
                | WS_EX_TRANSPARENT
                | WS_EX_NOACTIVATE
                | WS_EX_TOOLWINDOW
                | WS_EX_TOPMOST,
            w!("Compono.Overlay"),
            w!("Compono.Overlay"),
            WS_POPUP,
            bounds.x,
            bounds.y,
            bounds.width as i32,
            bounds.height as i32,
            None,
            None,
            Some(HINSTANCE(hinstance.0)),
            None,
        )?;
        Ok(hwnd)
    }
}

unsafe extern "system" fn overlay_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    DefWindowProcW(hwnd, msg, wparam, lparam)
}
