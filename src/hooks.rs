//! Hooks bas niveau installÃ©s sur la thread UI : position souris (WH_MOUSE_LL),
//! navigation clavier (WH_KEYBOARD_LL) et dÃ©but/fin de drag systÃ¨me (WinEventHook).
//! Les Ã©vÃ©nements sont mis en file puis consommÃ©s par la boucle de messages.

use std::collections::VecDeque;
use std::sync::Mutex;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, VK_DOWN, VK_ESCAPE, VK_LEFT, VK_LMENU, VK_MENU, VK_RIGHT, VK_RMENU, VK_UP,
    VK_CONTROL,
};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::arrow_snap::ArrowKey;

/// Ã‰vÃ©nement souris, clavier ou drag systÃ¨me.
#[derive(Debug, Clone, Copy)]
pub enum InputEvent {
    MouseMove { x: i32, y: i32 },
    MouseDown { x: i32, y: i32 },
    MouseUp { x: i32, y: i32 },
    RightDown,
    DragStart { hwnd: Option<usize> },
    DragEnd,
    GridNavigate(ArrowKey),
    GridFinish,
    GridCancel,
}

static QUEUE: Mutex<VecDeque<InputEvent>> = Mutex::new(VecDeque::new());
static MOUSE_HOOK: Mutex<Option<usize>> = Mutex::new(None);
static KEYBOARD_HOOK: Mutex<Option<usize>> = Mutex::new(None);
static EVENT_HOOK: Mutex<Option<usize>> = Mutex::new(None);

/// Installe les hooks sur la thread appelante (la thread UI).
pub fn install() -> windows::core::Result<()> {
    unsafe {
        let mouse_hook = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), None, 0)?;
        *MOUSE_HOOK.lock().unwrap() = Some(mouse_hook.0 as usize);

        let kbd_hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), None, 0)?;
        *KEYBOARD_HOOK.lock().unwrap() = Some(kbd_hook.0 as usize);

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

/// Retire les hooks (Ã  l'arrÃªt).
pub fn uninstall() {
    if let Some(raw) = MOUSE_HOOK.lock().unwrap().take() {
        unsafe {
            let _ = UnhookWindowsHookEx(HHOOK(raw as *mut core::ffi::c_void));
        }
    }
    if let Some(raw) = KEYBOARD_HOOK.lock().unwrap().take() {
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

/// Vide la file d'Ã©vÃ©nements.
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

unsafe extern "system" fn keyboard_proc(ncode: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if ncode >= 0 {
        let info = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
        let vk = info.vkCode;
        let is_key_down = wparam.0 == WM_KEYDOWN as usize || wparam.0 == WM_SYSKEYDOWN as usize;
        let is_key_up = wparam.0 == WM_KEYUP as usize || wparam.0 == WM_SYSKEYUP as usize;

        if is_key_down {
            let alt_down = (GetAsyncKeyState(VK_MENU.0 as i32) as u16 & 0x8000) != 0;
            let ctrl_down = (GetAsyncKeyState(VK_CONTROL.0 as i32) as u16 & 0x8000) != 0;

            // Si Alt est maintenu et Ctrl est relÃ¢chÃ© pendant l'appui sur une flÃ¨che directionnelle :
            if alt_down && !ctrl_down {
                match vk {
                    c if c == VK_LEFT.0 as u32 => {
                        push(InputEvent::GridNavigate(ArrowKey::Left));
                        return LRESULT(1);
                    }
                    c if c == VK_RIGHT.0 as u32 => {
                        push(InputEvent::GridNavigate(ArrowKey::Right));
                        return LRESULT(1);
                    }
                    c if c == VK_UP.0 as u32 => {
                        push(InputEvent::GridNavigate(ArrowKey::Up));
                        return LRESULT(1);
                    }
                    c if c == VK_DOWN.0 as u32 => {
                        push(InputEvent::GridNavigate(ArrowKey::Down));
                        return LRESULT(1);
                    }
                    c if c == VK_ESCAPE.0 as u32 => {
                        push(InputEvent::GridCancel);
                        return LRESULT(1);
                    }
                    _ => {}
                }
            }
        } else if is_key_up {
            if vk == VK_MENU.0 as u32 || vk == VK_LMENU.0 as u32 || vk == VK_RMENU.0 as u32 {
                push(InputEvent::GridFinish);
            }
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