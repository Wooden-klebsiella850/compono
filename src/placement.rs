//! Placement et redimensionnement d'une fenêtre sur un rectangle de grille.

use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetAncestor, GetClassNameW, GetDesktopWindow, GetShellWindow, GetWindow,
    GetWindowLongPtrW, GetWindowRect, GetWindowThreadProcessId, IsWindowVisible, IsZoomed,
    SetForegroundWindow, SetWindowPos, ShowWindow, GA_PARENT, GA_ROOTOWNER, GWL_STYLE, GW_OWNER,
    SWP_FRAMECHANGED, SWP_NOZORDER, SWP_SHOWWINDOW, SW_RESTORE, WS_CHILD,
};

use crate::grid::Rect;

/// Détermine si une fenêtre est plaçable (exclut le bureau, la barre des tâches et les overlays).
pub fn is_placeable_window(hwnd: HWND) -> bool {
    unsafe {
        if hwnd.0.is_null() {
            return false;
        }
        let desktop = GetDesktopWindow();
        let shell = GetShellWindow();
        if hwnd == desktop || hwnd == shell {
            return false;
        }
        if let Some(name) = window_class_name(hwnd) {
            if name == "Progman"
                || name == "WorkerW"
                || name == "Shell_TrayWnd"
                || name == "Shell_SecondaryTrayWnd"
                || name == "Compono.Overlay" || name == "Compono.Core"
            {
                return false;
            }
        }
        true
    }
}

fn window_class_name(hwnd: HWND) -> Option<String> {
    unsafe {
        let mut class_name = [0u16; 256];
        let len = GetClassNameW(hwnd, &mut class_name);
        (len > 0).then(|| String::from_utf16_lossy(&class_name[..len as usize]))
    }
}

/// Résout la fenêtre racine de plus haut niveau en remontant les chaînes de parents (GetParent)
/// et de propriétaires (GW_OWNER), indispensable pour les applications UWP / WinUI / XAML Islands / Windows Terminal
/// qui utilisent des surfaces enfants ou popups hôtes (ex: Intermediate D3D Window).
pub fn resolve_top_level_window(mut hwnd: HWND) -> HWND {
    unsafe {
        if hwnd.0.is_null() {
            return hwnd;
        }
        let root_owner = GetAncestor(hwnd, GA_ROOTOWNER);
        if !root_owner.0.is_null() {
            hwnd = root_owner;
        }
        let desktop = GetDesktopWindow();
        loop {
            if let Ok(owner) = GetWindow(hwnd, GW_OWNER) {
                if !owner.0.is_null() && owner != desktop {
                    hwnd = owner;
                    continue;
                }
            }
            let parent = GetAncestor(hwnd, GA_PARENT);
            if !parent.0.is_null() && parent != desktop {
                hwnd = parent;
                continue;
            }
            break;
        }
        hwnd
    }
}

/// Trouve la fenêtre principale de premier niveau associée à un HWND ou à son processus.
/// Pour Windows Terminal (`WindowsTerminal.exe` / `CASCADIA_HOSTING_WINDOW_CLASS`), cette fonction
/// garantit de trouver la véritable fenêtre parente même si le clic touche un contrôle XAML interne.
pub fn get_top_level_window_for_hwnd(hwnd: HWND) -> HWND {
    unsafe {
        if hwnd.0.is_null() {
            return hwnd;
        }
        let candidate = resolve_top_level_window(hwnd);

        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 || pid == std::process::id() {
            return candidate;
        }

        struct FindContext {
            pid: u32,
            found: HWND,
            terminal_host: HWND,
        }
        unsafe extern "system" fn enum_proc(wnd: HWND, lparam: LPARAM) -> BOOL {
            let ctx = &mut *(lparam.0 as *mut FindContext);
            let mut wnd_pid = 0u32;
            GetWindowThreadProcessId(wnd, Some(&mut wnd_pid));
            if wnd_pid == ctx.pid && IsWindowVisible(wnd).as_bool() {
                let style = GetWindowLongPtrW(wnd, GWL_STYLE) as u32;
                if (style & WS_CHILD.0) == 0 && is_placeable_window(wnd) {
                    if window_class_name(wnd).as_deref() == Some("CASCADIA_HOSTING_WINDOW_CLASS") {
                        ctx.terminal_host = wnd;
                        return BOOL(0);
                    }
                    if ctx.found.0.is_null() {
                        ctx.found = wnd;
                    }
                }
            }
            BOOL(1)
        }

        let mut ctx = FindContext {
            pid,
            found: HWND(std::ptr::null_mut()),
            terminal_host: HWND(std::ptr::null_mut()),
        };
        let _ = EnumWindows(Some(enum_proc), LPARAM(&mut ctx as *mut _ as isize));
        if !ctx.terminal_host.0.is_null() {
            return ctx.terminal_host;
        }

        if is_placeable_window(candidate) {
            let style = GetWindowLongPtrW(candidate, GWL_STYLE) as u32;
            if (style & WS_CHILD.0) == 0 {
                return candidate;
            }
        }
        if !ctx.found.0.is_null() {
            return ctx.found;
        }

        candidate
    }
}

/// Place et redimensionne une fenêtre sur le rectangle cible en tenant compte
/// des marges invisibles DWM (ombres et bordures de redimensionnement de Windows 10/11).
pub fn place_window(hwnd: HWND, target: Rect) -> bool {
    unsafe {
        if hwnd.0.is_null() {
            return false;
        }

        // Remonter à la véritable fenêtre racine (essentiel pour Windows Terminal, Electron, Chrome, etc.)
        let hwnd = get_top_level_window_for_hwnd(hwnd);

        // Si la fenêtre est maximisée, la restaurer pour permettre son redimensionnement
        if IsZoomed(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }

        // Récupérer le rectangle brut de la fenêtre et son cadre visible réel (DWM)
        let mut window_rect = RECT::default();
        let mut frame_rect = RECT::default();
        let _ = GetWindowRect(hwnd, &mut window_rect);
        let hr = DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut frame_rect as *mut _ as *mut _,
            std::mem::size_of::<RECT>() as u32,
        );

        // Décalage entre le rectangle logique et le rendu visuel
        let (offset_left, offset_top, offset_right, offset_bottom) = if hr.is_ok() {
            (
                (frame_rect.left - window_rect.left).max(0),
                (frame_rect.top - window_rect.top).max(0),
                (window_rect.right - frame_rect.right).max(0),
                (window_rect.bottom - frame_rect.bottom).max(0),
            )
        } else {
            (0, 0, 0, 0)
        };

        let x = target.x - offset_left;
        let y = target.y - offset_top;
        let width = target.width as i32 + offset_left + offset_right;
        let height = target.height as i32 + offset_top + offset_bottom;

        let result = SetWindowPos(
            hwnd,
            None,
            x,
            y,
            width,
            height,
            SWP_NOZORDER | SWP_FRAMECHANGED | SWP_SHOWWINDOW,
        );
        if let Err(err) = &result {
            // Accès refusé ici signale presque toujours une fenêtre élevée
            // (UIPI) : le manifeste doit demander requireAdministrator.
            log::error!("SetWindowPos a échoué sur {hwnd:?} : {err}");
        }

        let _ = SetForegroundWindow(hwnd);

        result.is_ok()
    }
}
