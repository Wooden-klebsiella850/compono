//! Démarrage automatique avec Windows, via HKCU\...\Run (pas besoin d'administrateur).

use windows::core::HSTRING;
use windows::Win32::Foundation::ERROR_FILE_NOT_FOUND;
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_SZ, RRF_RT_REG_SZ, RegCloseKey, RegDeleteValueW,
    RegGetValueW, RegOpenKeyExW, RegSetValueExW,
};

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const VALUE_NAME: &str = "Compono";

/// Le lancement au démarrage de Windows est-il actif ?
pub fn is_enabled() -> bool {
    read_command().is_some()
}

/// Active ou désactive le lancement au démarrage. Retourne false en cas d'échec.
pub fn set_enabled(enabled: bool) -> bool {
    if enabled {
        let Ok(exe) = std::env::current_exe() else {
            return false;
        };
        write_command(&format!("\"{}\"", exe.display()))
    } else {
        delete_command()
    }
}

fn read_command() -> Option<String> {
    let mut buffer = [0u16; 512];
    let mut size = (buffer.len() * 2) as u32;
    let subkey = HSTRING::from(RUN_KEY);
    let value = HSTRING::from(VALUE_NAME);
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
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

fn write_command(data: &str) -> bool {
    let mut key = HKEY(std::ptr::null_mut());
    let subkey = HSTRING::from(RUN_KEY);
    let status = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, &subkey, None, KEY_SET_VALUE, &mut key) };
    if status.0 != 0 {
        return false;
    }
    let wide: Vec<u16> = data.encode_utf16().chain(std::iter::once(0)).collect();
    let bytes = unsafe { std::slice::from_raw_parts(wide.as_ptr() as *const u8, wide.len() * 2) };
    let value = HSTRING::from(VALUE_NAME);
    let status = unsafe { RegSetValueExW(key, &value, None, REG_SZ, Some(bytes)) };
    unsafe {
        let _ = RegCloseKey(key);
    }
    status.0 == 0
}

fn delete_command() -> bool {
    let mut key = HKEY(std::ptr::null_mut());
    let subkey = HSTRING::from(RUN_KEY);
    let status = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, &subkey, None, KEY_SET_VALUE, &mut key) };
    if status.0 != 0 {
        return false;
    }
    let value = HSTRING::from(VALUE_NAME);
    let status = unsafe { RegDeleteValueW(key, &value) };
    unsafe {
        let _ = RegCloseKey(key);
    }
    // Déjà absente : le résultat voulu (désactivé) est atteint.
    status.0 == 0 || status.0 == ERROR_FILE_NOT_FOUND.0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lecture seule du registre réel, sans effet de bord.
    #[test]
    fn lit_letat_de_demarrage() {
        let _enabled = is_enabled();
    }
}
