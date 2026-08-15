//! Icône de zone de notification (tray) : ajout, retrait, menu contextuel.

use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_SETVERSION, NIN_SELECT,
    NOTIFYICONDATAW, NOTIFYICON_VERSION, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, GetCursorPos, LoadIconW, MF_CHECKED, MF_SEPARATOR,
    MF_STRING, MF_UNCHECKED, PostMessageW, RegisterWindowMessageW, SetForegroundWindow,
    TrackPopupMenu, TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_APP, WM_CONTEXTMENU, WM_LBUTTONUP, WM_NULL,
    WM_RBUTTONDBLCLK, WM_RBUTTONUP,
};

use crate::i18n::I18n;

pub const WM_TRAY: u32 = WM_APP + 1;
pub const TRAY_ID: u32 = 1;

const IDM_SHOW_GRID: u16 = 1;
const IDM_TOGGLE_STARTUP: u16 = 2;
const IDM_TOGGLE_SNAP: u16 = 3;
const IDM_QUIT: u16 = 4;

const ICON_RESOURCE_ID: usize = 101;

/// Action demandée par l'utilisateur depuis le tray.
pub enum TrayAction {
    None,
    ShowGrid,
    ToggleStartup,
    ToggleSnap,
    Quit,
}

/// Ajoute l'icône dans la zone de notification.
pub fn add(hwnd: HWND, tr: &I18n) -> windows::core::Result<()> {
    unsafe {
        let hinstance = GetModuleHandleW(None)?;
        let icon = LoadIconW(Some(hinstance.into()), PCWSTR(ICON_RESOURCE_ID as _))?;
        let mut nid = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: TRAY_ID,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
            uCallbackMessage: WM_TRAY,
            hIcon: icon,
            ..Default::default()
        };
        copy_to_wide(tr.t("app.tray_tip"), &mut nid.szTip);
        if Shell_NotifyIconW(NIM_ADD, &nid).0 == 0 {
            return Err(windows::core::Error::from_win32());
        }
        // La version classique fournit directement WM_RBUTTONUP dans lParam.
        // Elle reste la plus fiable pour les menus contextuels sur les shells Windows.
        nid.Anonymous.uVersion = NOTIFYICON_VERSION;
        let _ = Shell_NotifyIconW(NIM_SETVERSION, &nid);
        Ok(())
    }
}

/// Retire l'icône (arrêt de l'app ou fin de session).
pub fn remove(hwnd: HWND) {
    unsafe {
        let nid = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: TRAY_ID,
            ..Default::default()
        };
        let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
    }
}

/// Message système à écouter pour recréer l'icône après un redémarrage d'explorer.
pub fn taskbar_created_message() -> u32 {
    unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) }
}

/// Traite la notification de la zone de notification (message WM_TRAY).
pub fn on_callback(hwnd: HWND, lparam: LPARAM, tr: &I18n) -> TrayAction {
    let raw = lparam.0 as u32;
    let low_word = raw & 0xFFFF;
    // Accepte aussi les messages envoyés au format Version 4 si Explorer a
    // conservé la version précédente de l'icône pendant son redémarrage.
    let msg = match low_word {
        WM_LBUTTONUP | WM_RBUTTONUP | WM_RBUTTONDBLCLK | WM_CONTEXTMENU | NIN_SELECT => low_word,
        _ => raw,
    };
    match msg {
        // Un simple clic gauche ouvre aussi le menu contextuel, comme le clic droit.
        WM_LBUTTONUP | NIN_SELECT | WM_RBUTTONUP | WM_CONTEXTMENU | WM_RBUTTONDBLCLK => {
            show_menu(hwnd, tr)
        }
        _ => TrayAction::None,
    }
}

/// Traite un WM_COMMAND issu du menu contextuel.
pub fn on_command(wparam: WPARAM) -> TrayAction {
    match wparam.0 as u16 {
        IDM_SHOW_GRID => TrayAction::ShowGrid,
        IDM_TOGGLE_STARTUP => TrayAction::ToggleStartup,
        IDM_TOGGLE_SNAP => TrayAction::ToggleSnap,
        IDM_QUIT => TrayAction::Quit,
        _ => TrayAction::None,
    }
}

fn show_menu(hwnd: HWND, tr: &I18n) -> TrayAction {
    unsafe {
        let menu = match CreatePopupMenu() {
            Ok(menu) => menu,
            Err(_) => return TrayAction::None,
        };
        let show_grid = HSTRING::from(tr.t("app.show_grid"));
        let _ = AppendMenuW(menu, MF_STRING, IDM_SHOW_GRID as usize, &show_grid);
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, w!(""));

        let startup_enabled = crate::startup::is_enabled();
        let toggle_startup_text = if startup_enabled {
            format!("{} [ON]", tr.t("startup.action"))
        } else {
            format!("{} [OFF]", tr.t("startup.action"))
        };
        let toggle_startup = HSTRING::from(toggle_startup_text);
        let startup_flags = if startup_enabled {
            MF_STRING | MF_CHECKED
        } else {
            MF_STRING | MF_UNCHECKED
        };
        let _ = AppendMenuW(menu, startup_flags, IDM_TOGGLE_STARTUP as usize, &toggle_startup);

        let snap_enabled = crate::snap::is_snap_enabled();
        let toggle_snap_text = if snap_enabled {
            format!("{} [ON]", tr.t("snap.action"))
        } else {
            format!("{} [OFF]", tr.t("snap.action"))
        };
        let toggle_snap = HSTRING::from(toggle_snap_text);
        let quit = HSTRING::from(tr.t("app.quit"));

        let snap_flags = if snap_enabled {
            MF_STRING | MF_CHECKED
        } else {
            MF_STRING | MF_UNCHECKED
        };
        let _ = AppendMenuW(menu, snap_flags, IDM_TOGGLE_SNAP as usize, &toggle_snap);
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, w!(""));
        let _ = AppendMenuW(menu, MF_STRING, IDM_QUIT as usize, &quit);

        let _ = SetForegroundWindow(hwnd);
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let command = TrackPopupMenu(
            menu,
            TPM_RIGHTBUTTON | TPM_RETURNCMD,
            pt.x,
            pt.y,
            Some(0),
            hwnd,
            None,
        );
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        if command.0 == 0 {
            TrayAction::None
        } else {
            on_command(WPARAM(command.0 as usize))
        }
    }
}

fn copy_to_wide(s: &str, buf: &mut [u16]) {
    let mut index = 0;
    for unit in s.encode_utf16() {
        if index >= buf.len() - 1 {
            break;
        }
        buf[index] = unit;
        index += 1;
    }
    buf[index] = 0;
}
