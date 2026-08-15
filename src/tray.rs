//! Icône de zone de notification (tray) : ajout, retrait, menu contextuel.

use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_SETVERSION, NOTIFYICONDATAW,
    NOTIFYICON_VERSION_4, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, GetCursorPos, LoadIconW, MF_SEPARATOR, MF_STRING,
    PostMessageW, RegisterWindowMessageW, SetForegroundWindow, TrackPopupMenu, TPM_RIGHTBUTTON,
    WM_APP, WM_LBUTTONUP, WM_NULL, WM_RBUTTONDBLCLK, WM_RBUTTONUP,
};

use crate::i18n::I18n;

pub const WM_TRAY: u32 = WM_APP + 1;
pub const TRAY_ID: u32 = 1;

const IDM_SHOW_GRID: u16 = 1;
const IDM_CONFIGURE: u16 = 2;
const IDM_QUIT: u16 = 3;

const ICON_RESOURCE_ID: usize = 101;

/// Action demandée par l'utilisateur depuis le tray.
pub enum TrayAction {
    None,
    ShowGrid,
    Configure,
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
        nid.Anonymous.uVersion = NOTIFYICON_VERSION_4;
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
    match lparam.0 as u32 {
        WM_LBUTTONUP => TrayAction::ShowGrid,
        WM_RBUTTONUP | WM_RBUTTONDBLCLK => {
            show_menu(hwnd, tr);
            TrayAction::None
        }
        _ => TrayAction::None,
    }
}

/// Traite un WM_COMMAND issu du menu contextuel.
pub fn on_command(wparam: WPARAM) -> TrayAction {
    match wparam.0 as u16 {
        IDM_SHOW_GRID => TrayAction::ShowGrid,
        IDM_CONFIGURE => TrayAction::Configure,
        IDM_QUIT => TrayAction::Quit,
        _ => TrayAction::None,
    }
}

fn show_menu(hwnd: HWND, tr: &I18n) {
    unsafe {
        let menu = match CreatePopupMenu() {
            Ok(menu) => menu,
            Err(_) => return,
        };
        let show_grid = HSTRING::from(tr.t("app.show_grid"));
        let configure = HSTRING::from(tr.t("app.configure"));
        let quit = HSTRING::from(tr.t("app.quit"));
        let _ = AppendMenuW(menu, MF_STRING, IDM_SHOW_GRID as usize, &show_grid);
        let _ = AppendMenuW(menu, MF_STRING, IDM_CONFIGURE as usize, &configure);
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, w!(""));
        let _ = AppendMenuW(menu, MF_STRING, IDM_QUIT as usize, &quit);

        let _ = SetForegroundWindow(hwnd);
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let _ = TrackPopupMenu(menu, TPM_RIGHTBUTTON, pt.x, pt.y, Some(0), hwnd, None);
        // Message nul posté pour que le menu se referme au premier clic.
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
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
