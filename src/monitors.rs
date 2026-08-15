//! Énumération des moniteurs, zone utile (rcWork), DPI et barre des tâches auto-masquée.

use std::sync::Mutex;

use windows::core::BOOL;
use windows::Win32::Foundation::{LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Shell::{
    ABE_BOTTOM, ABE_LEFT, ABE_RIGHT, ABE_TOP, ABM_GETSTATE, ABM_GETTASKBARPOS, ABS_AUTOHIDE,
    APPBARDATA, SHAppBarMessage,
};
use windows::Win32::UI::WindowsAndMessaging::MONITORINFOF_PRIMARY;

use crate::grid::Rect;

/// Réserve de sécurité quand la barre des tâches est en auto-masquage : sans ces
/// 2 px sur le bord, elle ne se réaffiche plus quand une fenêtre couvre le bord.
const AUTOHIDE_RESERVE_PX: u32 = 2;

/// Identifiant opaque d'un écran. HMONITOR est un pointeur jamais déréférencé,
/// partageable entre threads sans danger.
#[derive(Clone, Copy)]
pub struct MonitorHandle(HMONITOR);

// Safety : HMONITOR est un identifiant opaque, jamais déréférencé.
unsafe impl Send for MonitorHandle {}
unsafe impl Sync for MonitorHandle {}

impl std::fmt::Debug for MonitorHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Monitor({:p})", self.0 .0)
    }
}

/// Un écran physique avec sa zone utile, en pixels physiques (écran virtuel).
#[derive(Debug, Clone, Copy)]
pub struct MonitorInfo {
    pub handle: MonitorHandle,
    /// Zone utile : rcWork, la barre des tâches en est déjà exclue.
    pub work: Rect,
    /// Périmètre complet de l'écran, utilisé pour le placement multi-écran (phase 5).
    #[allow(dead_code)]
    pub bounds: Rect,
    pub is_primary: bool,
}

impl MonitorInfo {
    /// DPI effectif de l'écran, 96 si l'appel échoue.
    pub fn dpi(&self) -> u32 {
        unsafe {
            let mut dpix = 0;
            let mut dpiy = 0;
            if GetDpiForMonitor(self.handle.0, MDT_EFFECTIVE_DPI, &mut dpix, &mut dpiy).is_ok() {
                dpix
            } else {
                96
            }
        }
    }
}

/// Bord d'un écran.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenEdge {
    Top,
    Right,
    Bottom,
    Left,
}

/// Cache de la dernière énumération, recalculé sur invalidation.
static CACHE: Mutex<Option<Vec<MonitorInfo>>> = Mutex::new(None);

/// Énumère les moniteurs (EnumDisplayMonitors). Coordonnées d'écran virtuel.
pub fn enumerate() -> Vec<MonitorInfo> {
    let mut list = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(collect_monitor),
            LPARAM(&mut list as *mut Vec<MonitorInfo> as isize),
        );
    }
    list
}

unsafe extern "system" fn collect_monitor(
    handle: HMONITOR,
    _hdc: HDC,
    _clip: *mut RECT,
    lparam: LPARAM,
) -> BOOL {
    let monitors = &mut *(lparam.0 as *mut Vec<MonitorInfo>);
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if GetMonitorInfoW(handle, &mut info).0 != 0 {
        monitors.push(MonitorInfo {
            handle: MonitorHandle(handle),
            work: rect_from_win32(info.rcWork),
            bounds: rect_from_win32(info.rcMonitor),
            is_primary: info.dwFlags & MONITORINFOF_PRIMARY != 0,
        });
    }
    BOOL(1)
}

/// Renumérote les moniteurs et met le cache à jour. À appeler au démarrage et
/// sur invalidation (WM_DISPLAYCHANGE, WM_DPICHANGED, WM_SETTINGCHANGE).
pub fn reload() -> Vec<MonitorInfo> {
    let list = enumerate();
    if let Ok(mut cache) = CACHE.lock() {
        *cache = Some(list.clone());
    }
    list
}

/// Moniteurs actuellement connus, utilisés par l'overlay (phase 3).
#[allow(dead_code)]
pub fn current() -> Vec<MonitorInfo> {
    if let Ok(cache) = CACHE.lock() {
        if let Some(list) = cache.clone() {
            return list;
        }
    }
    reload()
}

/// Bord sur lequel la barre des tâches est en auto-masquage, sinon None.
pub fn taskbar_autohide_edge() -> Option<ScreenEdge> {
    unsafe {
        let mut abd = APPBARDATA {
            cbSize: size_of::<APPBARDATA>() as u32,
            ..Default::default()
        };
        if SHAppBarMessage(ABM_GETSTATE, &mut abd) & ABS_AUTOHIDE as usize == 0 {
            return None;
        }
        SHAppBarMessage(ABM_GETTASKBARPOS, &mut abd);
        match abd.uEdge {
            ABE_TOP => Some(ScreenEdge::Top),
            ABE_RIGHT => Some(ScreenEdge::Right),
            ABE_BOTTOM => Some(ScreenEdge::Bottom),
            ABE_LEFT => Some(ScreenEdge::Left),
            _ => None,
        }
    }
}

/// Réserve 2 px sur le bord auto-masqué pour que la barre des tâches se réaffiche.
pub fn reserve_autohide(work: Rect, edge: Option<ScreenEdge>) -> Rect {
    match edge {
        Some(ScreenEdge::Top) => work.inset_top(AUTOHIDE_RESERVE_PX),
        Some(ScreenEdge::Right) => work.inset_right(AUTOHIDE_RESERVE_PX),
        Some(ScreenEdge::Bottom) => work.inset_bottom(AUTOHIDE_RESERVE_PX),
        Some(ScreenEdge::Left) => work.inset_left(AUTOHIDE_RESERVE_PX),
        None => work,
    }
}

/// Zone utile effective : rcWork avec la réserve du bord auto-masqué.
pub fn effective_work(work: Rect) -> Rect {
    reserve_autohide(work, taskbar_autohide_edge())
}

fn rect_from_win32(r: RECT) -> Rect {
    Rect::from_edges(r.left, r.top, r.right, r.bottom)
}
