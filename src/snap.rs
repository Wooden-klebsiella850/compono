//! Ancrage des fenêtres (Aero Snap) : lecture et bascule du réglage Windows.
//! Équivalent natif de Activer_Desactiver_Ancrage_Fenêtres.bat.

use windows::core::HSTRING;
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_DWORD, REG_SZ, RRF_RT_REG_SZ, RegCloseKey,
    RegGetValueW, RegOpenKeyExW, RegSetValueExW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    SPIF_SENDCHANGE, SPIF_UPDATEINIFILE, SPI_SETDRAGFROMMAXIMIZE, SystemParametersInfoW,
};

const DESKTOP_KEY: &str = "Control Panel\\Desktop";
const WINDOW_ARRANGEMENT_VALUE: &str = "WindowArrangementActive";
const DOCK_MOVING_VALUE: &str = "DockMoving";
const EXPLORER_ADVANCED_KEY: &str =
    "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Advanced";
const SNAP_ASSIST_VALUE: &str = "SnapAssist";

/// L'ancrage Windows est-il actif ?
pub fn is_snap_enabled() -> bool {
    read_sz(HKEY_CURRENT_USER, DESKTOP_KEY, WINDOW_ARRANGEMENT_VALUE)
        .as_deref()
        == Some("1")
}

/// Active ou désactive l'ancrage. Retourne false en cas d'échec d'écriture.
pub fn set_snap(enabled: bool) -> bool {
    let value = if enabled { "1" } else { "0" };
    let arrangement_ok = write_sz(HKEY_CURRENT_USER, DESKTOP_KEY, WINDOW_ARRANGEMENT_VALUE, value);
    let dock_moving_ok = write_sz(HKEY_CURRENT_USER, DESKTOP_KEY, DOCK_MOVING_VALUE, value);
    let snap_assist_ok = if enabled {
        write_dword(HKEY_CURRENT_USER, EXPLORER_ADVANCED_KEY, SNAP_ASSIST_VALUE, 1)
    } else {
        true
    };

    unsafe {
        // SPI_SETDRAGFROMMAXIMIZE : permet ou interdit le drag depuis une fenêtre maximisée.
        let _ = SystemParametersInfoW(
            SPI_SETDRAGFROMMAXIMIZE,
            u32::from(enabled),
            None,
            SPIF_UPDATEINIFILE | SPIF_SENDCHANGE,
        );
    }

    arrangement_ok && dock_moving_ok && snap_assist_ok
}

/// Redémarre l'explorateur, nécessaire pour appliquer le réglage.
pub fn restart_explorer() {
    let _ = std::process::Command::new("taskkill")
        .args(["/f", "/im", "explorer.exe"])
        .status();
    let _ = std::process::Command::new("explorer.exe").spawn();
}

fn read_sz(root: HKEY, subkey: &str, value: &str) -> Option<String> {
    let mut buffer = [0u16; 64];
    let mut size = (buffer.len() * 2) as u32;
    let subkey = HSTRING::from(subkey);
    let value = HSTRING::from(value);
    let status = unsafe {
        RegGetValueW(
            root,
            &subkey,
            &value,
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr() as *mut core::ffi::c_void),
            Some(&mut size),
        )
    };
    if status.0 != 0 {
        return None;
    }
    let len = (size / 2) as usize;
    let len = buffer[..len.min(buffer.len())]
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(len.min(buffer.len()));
    Some(String::from_utf16_lossy(&buffer[..len]))
}

fn write_sz(root: HKEY, subkey: &str, value: &str, data: &str) -> bool {
    let mut key = HKEY(std::ptr::null_mut());
    let subkey = HSTRING::from(subkey);
    let status = unsafe { RegOpenKeyExW(root, &subkey, None, KEY_SET_VALUE, &mut key) };
    if status.0 != 0 {
        return false;
    }
    let wide: Vec<u16> = data.encode_utf16().chain(std::iter::once(0)).collect();
    let bytes = unsafe {
        std::slice::from_raw_parts(wide.as_ptr() as *const u8, wide.len() * 2)
    };
    let value = HSTRING::from(value);
    let status = unsafe { RegSetValueExW(key, &value, None, REG_SZ, Some(bytes)) };
    unsafe {
        let _ = RegCloseKey(key);
    }
    status.0 == 0
}

fn write_dword(root: HKEY, subkey: &str, value: &str, data: u32) -> bool {
    let mut key = HKEY(std::ptr::null_mut());
    let subkey = HSTRING::from(subkey);
    let status = unsafe { RegOpenKeyExW(root, &subkey, None, KEY_SET_VALUE, &mut key) };
    if status.0 != 0 {
        return false;
    }
    let value = HSTRING::from(value);
    let bytes = data.to_le_bytes();
    let status = unsafe { RegSetValueExW(key, &value, None, REG_DWORD, Some(&bytes)) };
    unsafe {
        let _ = RegCloseKey(key);
    }
    status.0 == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lecture seule du registre réel, sans effet de bord.
    #[test]
    fn lit_letat_de_lancrage() {
        let _enabled = is_snap_enabled();
    }
}
