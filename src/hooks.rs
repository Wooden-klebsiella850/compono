//! Hooks bas niveau installés sur la thread UI : position souris (WH_MOUSE_LL)
//! et début/fin de drag système (WinEventHook). Les événements sont mis en file
//! puis consommés par la boucle de messages, jamais traités dans le callback.

use std::collections::VecDeque;
use std::sync::Mutex;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::WindowsAndMessaging::*;

/// Événement souris bas niveau ou de drag système. Les handles sont stockés en
/// valeur brute (usize) pour que le type reste Send.
#[derive(Debug, Clone, Copy)]
pub enum InputEvent {
    MouseMove { x: i32, y: i32 },
    MouseDown { x: i32, y: i32 },
    MouseUp { x: i32, y: i32 },
    RightDown,
    DragStart { hwnd: Option<usize> },
    DragEnd,
}

static QUEUE: Mutex<VecDeque<InputEvent>> = Mutex::new(VecDeque::new());
static MOUSE_HOOK: Mutex<Option<usize>> = Mutex::new(None);
static EVENT_HOOK: Mutex<Option<usize>> = Mutex::new(None);

/// Installe les deux hooks sur la thread appelante (la thread UI). La boucle de
/// messages doit tourner ensuite pour que les callbacks soient invoqués.
pub fn install() -> windows::core::Result<()> {
    unsafe {
        // hMod NULL : la procédure est dans le code du processus courant.
        let mouse_hook = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), None, 0)?;
        *MOUSE_HOOK.lock().unwrap() = Some(mouse_hook.0 as usize);

        let event_hook = SetWinEventHook(
            EVENT_SYSTEM_MOVESIZESTART,
            EVENT_SYSTEM_MOVESIZEEND,
            None,
            Some(event_proc),
            0,
            0,
            WINEVENT_OUTOFCONTEXT,
        );
        *EVENT_HOOK.lock().unwrap() = Some(event_hook.0 as usize);
        Ok(())
    }
}

/// Retire les hooks (à l'arrêt).
pub fn uninstall() {
    if let Some(raw) = MOUSE_HOOK.lock().unwrap().take() {
        unsafe {
            let _ = UnhookWindowsHookEx(HHOOK(raw as *mut core::ffi::c_void));
        }
    }
    if let Some(raw) = EVENT_HOOK.lock().unwrap().take() {
        unsafe {
            let _ = UnhookWinEvent(HWINEVENTHOOK(raw as *mut core::ffi::c_void));
        }
    }
}

/// Vide la file d'événements.
pub fn drain() -> Vec<InputEvent> {
    let mut queue = QUEUE.lock().unwrap();
    queue.drain(..).collect()
}

fn push(event: InputEvent) {
    if let Ok(mut queue) = QUEUE.lock() {
        queue.push_back(event);
    }
}

unsafe extern "system" fn mouse_proc(ncode: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if ncode >= 0 {
        let info = &*(lparam.0 as *const MSLLHOOKSTRUCT);
        match wparam.0 as u32 {
            WM_MOUSEMOVE => push(InputEvent::MouseMove {
                x: info.pt.x,
                y: info.pt.y,
            }),
            WM_LBUTTONDOWN => push(InputEvent::MouseDown {
                x: info.pt.x,
                y: info.pt.y,
            }),
            WM_LBUTTONUP => push(InputEvent::MouseUp {
                x: info.pt.x,
                y: info.pt.y,
            }),
            WM_RBUTTONDOWN => push(InputEvent::RightDown),
            _ => {}
        }
    }
    CallNextHookEx(None, ncode, wparam, lparam)
}

unsafe extern "system" fn event_proc(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    _idobject: i32,
    _idchild: i32,
    _ideventthread: u32,
    _dwms: u32,
) {
    match event {
        EVENT_SYSTEM_MOVESIZESTART => push(InputEvent::DragStart {
            hwnd: if hwnd.0.is_null() {
                None
            } else {
                Some(hwnd.0 as usize)
            },
        }),
        EVENT_SYSTEM_MOVESIZEEND => push(InputEvent::DragEnd),
        _ => {}
    }
}
