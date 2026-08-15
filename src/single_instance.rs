//! Instance unique via mutex nommé. La deuxième instance notifie la première et se termine.

use windows::core::w;
use windows::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE, LPARAM, WPARAM,
};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::{RegisterWindowMessageW, SendMessageW, HWND_BROADCAST};

/// Garde l'instance unique en vie. Le mutex est libéré au drop.
pub struct InstanceGuard {
    handle: HANDLE,
    show_grid_message: u32,
}

impl InstanceGuard {
    /// Tente d'acquérir l'instance unique.
    /// Retourne None si une autre instance tourne déjà, après l'avoir notifiée.
    pub fn acquire() -> Option<InstanceGuard> {
        unsafe {
            let handle = CreateMutexW(None, false, w!("Global\\Compono.SingleInstance")).ok()?;
            if GetLastError() == ERROR_ALREADY_EXISTS {
                notify_existing();
                return None;
            }
            let show_grid_message = RegisterWindowMessageW(w!("Compono.ShowGrid"));
            Some(InstanceGuard {
                handle,
                show_grid_message,
            })
        }
    }

    /// Id du message système "afficher la grille", partagé entre instances.
    pub fn show_grid_message(&self) -> u32 {
        self.show_grid_message
    }
}

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

fn notify_existing() {
    let msg = unsafe { RegisterWindowMessageW(w!("Compono.ShowGrid")) };
    if msg != 0 {
        unsafe {
            SendMessageW(HWND_BROADCAST, msg, Some(WPARAM(0)), Some(LPARAM(0)));
        }
    }
}
