//! Point d'entrÃ©e. Initialise le logging, l'instance unique, une fenÃªtre cachÃ©e
//! qui sert de support au tray et aux notifications, puis lance la boucle de messages.

// Masque la console en release ; gardÃ©e en debug pour lire les logs directement.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod arrow_snap;
mod config;
mod hooks;
mod i18n;
mod logging;
mod monitors;
mod overlay;
mod placement;
mod session;
mod single_instance;
mod snap;
mod startup;
mod tray;
mod grid;

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::sync::atomic::{AtomicU32, Ordering};

use log::{error, info};
use windows::core::w;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_WIN, RegisterHotKey, UnregisterHotKey,
};
use windows::Win32::UI::WindowsAndMessaging::*;

use i18n::I18n;
use single_instance::InstanceGuard;

/// Traductions chargÃ©es, accessibles depuis la procÃ©dure de fenÃªtre.
static TR: OnceLock<I18n> = OnceLock::new();

/// Id du message "afficher la grille" reÃ§u depuis une deuxiÃ¨me instance.
static SHOW_GRID_MSG: AtomicU32 = AtomicU32::new(0);

/// Id du message "TaskbarCreated" pour recrÃ©er l'icÃ´ne aprÃ¨s un redÃ©marrage d'explorer.
static TASKBAR_CREATED_MSG: AtomicU32 = AtomicU32::new(0);

/// Conteneur mono-thread : les ressources COM/HWND ne sont pas Send, tous les
/// accÃ¨s se font sur la thread UI (boucle de messages), sous mutex.
struct UiCell<T>(Mutex<Option<T>>);
unsafe impl<T> Send for UiCell<T> {}
unsafe impl<T> Sync for UiCell<T> {}

impl<T> std::ops::Deref for UiCell<T> {
    type Target = Mutex<Option<T>>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Gestionnaire d'overlays et machine Ã  Ã©tats du geste, cÃ´tÃ© procÃ©dure de fenÃªtre.
static OVERLAY: UiCell<overlay::OverlayManager> = UiCell(Mutex::new(None));
static SESSION: UiCell<session::Session> = UiCell(Mutex::new(None));

/// Ã‰tat persistant du placement au clavier Ctrl+Alt+FlÃ¨che (moitiÃ©/quart par fenÃªtre).
static ARROW_SNAP: UiCell<arrow_snap::ArrowSnap> = UiCell(Mutex::new(None));

/// Ã‰tat du dÃ©placement direct de fenÃªtre sur la grille au clavier (Alt + FlÃ¨ches).
struct KeyboardNavState {
    hwnd: HWND,
    monitor_idx: usize,
    range: grid::CellRange,
}
static KBD_NAV: UiCell<KeyboardNavState> = UiCell(Mutex::new(None));

/// Id du raccourci global qui bascule l'affichage de la grille.
const HOTKEY_TOGGLE_ID: i32 = 1;
const HOTKEY_LEFT_ID: i32 = 2;
const HOTKEY_RIGHT_ID: i32 = 3;
const HOTKEY_UP_ID: i32 = 4;
const HOTKEY_DOWN_ID: i32 = 5;

// Codes de touches virtuelles flÃ©chÃ©es : absents des constantes VK_ importÃ©es.
const VK_LEFT: u32 = 0x25;
const VK_UP: u32 = 0x26;
const VK_RIGHT: u32 = 0x27;
const VK_DOWN: u32 = 0x28;

fn main() {
    // Instance unique avant toute initialisation. La seconde instance notifie et s'arrÃªte.
    let guard = match InstanceGuard::acquire() {
        Some(guard) => guard,
        None => return,
    };

    let appdata = appdata_dir();

    if let Err(err) = logging::init(&appdata) {
        eprintln!("impossible d'initialiser le log : {err}");
    }
    // Le manifeste dÃ©clare dÃ©jÃ  PerMonitorV2, l'appel garde la main si le manifeste est retirÃ©.
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }

    let config = config::load(&config::config_path(&appdata));
    let system_lang = default_lang();
    let lang = config.lang.as_deref().unwrap_or(&system_lang);
    let tr = i18n::I18n::load(lang, &appdata.join("locales"));
    info!("{} v{} dÃ©marre", tr.t("app.name"), env!("CARGO_PKG_VERSION"));
    let _ = TR.set(tr);

    SHOW_GRID_MSG.store(guard.show_grid_message(), Ordering::Relaxed);
    TASKBAR_CREATED_MSG.store(tray::taskbar_created_message(), Ordering::Relaxed);

    log_monitors();

    OVERLAY
        .lock()
        .expect("verrou overlay")
        .get_or_insert_with(|| overlay::OverlayManager::new(grid::GridOptions::default()));

    let hwnd = match create_core_window() {
        Some(hwnd) => hwnd,
        None => {
            error!("Ã©chec de crÃ©ation de la fenÃªtre principale");
            return;
        }
    };

    // Win+Alt+G : bascule l'affichage de la grille. EnregistrÃ© sur la fenÃªtre
    // pour que WM_HOTKEY lui soit adressÃ© directement.
    unsafe {
        let res = RegisterHotKey(
            Some(hwnd),
            HOTKEY_TOGGLE_ID,
            MOD_WIN | MOD_ALT | MOD_NOREPEAT,
            'G' as u32,
        );
        info!("enregistrement raccourci Win+Alt+G (grille) : {:?}", res.is_ok());
    }

    // Gestion dynamique des raccourcis Ctrl+Alt+FlÃ¨ches selon l'Ã©tat de l'ancrage Windows
    update_arrow_hotkeys(hwnd, snap::is_snap_enabled());

    // Hooks souris, clavier et drag, installÃ©s sur la thread UI.
    if let Err(err) = hooks::install() {
        error!("Ã©chec de l'installation des hooks : {err}");
    }
    SESSION
        .lock()
        .expect("verrou session")
        .get_or_insert_with(|| session::Session::new(grid::GridOptions::default()));
    if let Err(err) = tray::add(hwnd, TR.get().expect("i18n non initialisÃ©")) {
        error!("Ã©chec d'ajout de l'icÃ´ne tray : {err}");
    }

    info!("boucle de messages dÃ©marrÃ©e");
    run_message_loop();

    // `guard` est dÃ©truit ici, le mutex est libÃ©rÃ©.
    info!("arrÃªt propre");
}

fn appdata_dir() -> PathBuf {
    std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("Compono"))
        .join("Compono")
}

fn default_lang() -> String {
    sys_locale::get_locale()
        .map(|loc| loc.split('-').next().unwrap_or("fr").to_string())
        .unwrap_or_else(|| "fr".to_string())
}

/// Ã‰numÃ¨re les Ã©crans et logge la grille par dÃ©faut de chacun.
fn log_monitors() {
    let monitors = monitors::reload();
    info!("{} moniteur(s) dÃ©tectÃ©(s)", monitors.len());
    let autohide = monitors::taskbar_autohide_edge();
    for monitor in &monitors {
        let work = monitors::reserve_autohide(monitor.work, autohide);
        match grid::Grid::new(work, grid::GridOptions::default()) {
            Some(g) => info!(
                "Ã©cran {:?} (primary={}, dpi={}) : zone {}x{}, grille {}x{}",
                monitor.handle,
                monitor.is_primary,
                monitor.dpi(),
                g.area().width,
                g.area().height,
                g.cols(),
                g.rows()
            ),
            None => info!("Ã©cran {:?} : zone de travail trop petite", monitor.handle),
        }
    }
}

/// CrÃ©e la fenÃªtre cachÃ©e qui recevra les messages du tray et des autres instances.
fn create_core_window() -> Option<HWND> {
    unsafe {
        let hinstance = match GetModuleHandleW(None) {
            Ok(h) => h,
            Err(err) => {
                error!("GetModuleHandleW : {err}");
                return None;
            }
        };

        let wc = WNDCLASSW {
            lpfnWndProc: Some(core_wnd_proc),
            hInstance: HINSTANCE(hinstance.0),
            lpszClassName: w!("Compono.Core"),
            ..Default::default()
        };
        if RegisterClassW(&wc) == 0 {
            error!("RegisterClassW a Ã©chouÃ©");
            return None;
        }

        match CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            w!("Compono.Core"),
            w!("Compono.Core"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(HINSTANCE(hinstance.0)),
            None,
        ) {
            Ok(hwnd) => Some(hwnd),
            Err(err) => {
                error!("CreateWindowExW : {err}");
                None
            }
        }
    }
}

fn run_message_loop() {
    unsafe {
        let mut msg = MSG::default();
        loop {
            // PeekMessage plutÃ´t que GetMessage : les messages des hooks bas
            // niveau sont consommÃ©s Ã  l'intÃ©rieur de GetMessage et ne reviennent
            // jamais au code, il faut donc draine la file Ã  chaque itÃ©ration.
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).0 != 0 {
                if msg.message == WM_QUIT {
                    return;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            process_input_events();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }
}

/// Consomme les Ã©vÃ©nements souris/drag/clavier et applique les actions correspondantes.
fn process_input_events() {
    for event in hooks::drain() {
        match event {
            hooks::InputEvent::MouseMove { .. } => {}
            hooks::InputEvent::GridNavigate(key) => {
                handle_grid_navigate(key);
                continue;
            }
            hooks::InputEvent::GridFinish | hooks::InputEvent::GridCancel => {
                if let Ok(mut guard) = KBD_NAV.lock() {
                    *guard = None;
                }
                if let Ok(mut guard) = OVERLAY.lock() {
                    if let Some(manager) = guard.as_mut() {
                        manager.hide();
                    }
                }
                continue;
            }
            _ => info!("entrÃ©e : {event:?}"),
        }
        let action = SESSION
            .lock()
            .expect("verrou session")
            .get_or_insert_with(|| session::Session::new(grid::GridOptions::default()))
            .handle(event);
        apply_session_action(action);
    }
    let timer_action = SESSION
        .lock()
        .expect("verrou session")
        .get_or_insert_with(|| session::Session::new(grid::GridOptions::default()))
        .tick();
    apply_session_action(timer_action);
}

fn handle_grid_navigate(key: arrow_snap::ArrowKey) {
    let mut guard = KBD_NAV.lock().expect("verrou kbd nav");
    let (hwnd, mut monitor_idx, mut range, mut grid) = if let Some(state) = guard.as_ref() {
        let monitors = monitors::current();
        let Some(monitor) = monitors.get(state.monitor_idx) else { return; };
        let area = monitors::effective_work(monitor.work);
        let Some(grid) = grid::Grid::new(area, grid::GridOptions::default()) else { return; };
        (state.hwnd, state.monitor_idx, state.range, grid)
    } else {
        let fg = unsafe { GetForegroundWindow() };
        if fg.0.is_null() { return; }
        let hwnd = placement::get_top_level_window_for_hwnd(fg);
        if hwnd.0.is_null() || !placement::is_placeable_window(hwnd) { return; }
        let mut window_rect = RECT::default();
        if unsafe { GetWindowRect(hwnd, &mut window_rect) }.is_err() { return; }
        let center_x = (window_rect.left + window_rect.right) / 2;
        let center_y = (window_rect.top + window_rect.bottom) / 2;
        let Some((idx, monitor)) = monitors::current()
            .into_iter()
            .enumerate()
            .find(|(_, m)| m.bounds.contains(center_x, center_y))
        else { return; };
        let area = monitors::effective_work(monitor.work);
        let Some(grid) = grid::Grid::new(area, grid::GridOptions::default()) else { return; };
        let logical_rect = grid::Rect::new(
            window_rect.left,
            window_rect.top,
            (window_rect.right - window_rect.left).max(1) as u32,
            (window_rect.bottom - window_rect.top).max(1) as u32,
        );
        let range = grid.range_for_rect(logical_rect);
        (hwnd, idx, range, grid)
    };

    let cols = grid.cols();
    let rows = grid.rows();
    let width_in_cells = range.col1.saturating_sub(range.col0);
    let height_in_cells = range.row1.saturating_sub(range.row0);

    match key {
        arrow_snap::ArrowKey::Left => {
            if range.col0 > 0 {
                range.col0 -= 1;
                range.col1 = (range.col0 + width_in_cells).min(cols - 1);
            } else if let Some((next_idx, next_m)) = monitors::adjacent_monitor(monitor_idx, monitors::ScreenEdge::Left) {
                monitor_idx = next_idx;
                let next_area = monitors::effective_work(next_m.work);
                if let Some(next_grid) = grid::Grid::new(next_area, grid::GridOptions::default()) {
                    let next_cols = next_grid.cols();
                    range.col1 = next_cols.saturating_sub(1);
                    range.col0 = range.col1.saturating_sub(width_in_cells);
                    grid = next_grid;
                }
            }
        }
        arrow_snap::ArrowKey::Right => {
            if range.col1 + 1 < cols {
                range.col0 += 1;
                range.col1 = (range.col0 + width_in_cells).min(cols - 1);
            } else if let Some((next_idx, next_m)) = monitors::adjacent_monitor(monitor_idx, monitors::ScreenEdge::Right) {
                monitor_idx = next_idx;
                let next_area = monitors::effective_work(next_m.work);
                if let Some(next_grid) = grid::Grid::new(next_area, grid::GridOptions::default()) {
                    range.col0 = 0;
                    range.col1 = width_in_cells.min(next_grid.cols().saturating_sub(1));
                    grid = next_grid;
                }
            }
        }
        arrow_snap::ArrowKey::Up => {
            if range.row0 > 0 {
                range.row0 -= 1;
                range.row1 = (range.row0 + height_in_cells).min(rows - 1);
            } else if let Some((next_idx, next_m)) = monitors::adjacent_monitor(monitor_idx, monitors::ScreenEdge::Top) {
                monitor_idx = next_idx;
                let next_area = monitors::effective_work(next_m.work);
                if let Some(next_grid) = grid::Grid::new(next_area, grid::GridOptions::default()) {
                    let next_rows = next_grid.rows();
                    range.row1 = next_rows.saturating_sub(1);
                    range.row0 = range.row1.saturating_sub(height_in_cells);
                    grid = next_grid;
                }
            }
        }
        arrow_snap::ArrowKey::Down => {
            if range.row1 + 1 < rows {
                range.row0 += 1;
                range.row1 = (range.row0 + height_in_cells).min(rows - 1);
            } else if let Some((next_idx, next_m)) = monitors::adjacent_monitor(monitor_idx, monitors::ScreenEdge::Bottom) {
                monitor_idx = next_idx;
                let next_area = monitors::effective_work(next_m.work);
                if let Some(next_grid) = grid::Grid::new(next_area, grid::GridOptions::default()) {
                    range.row0 = 0;
                    range.row1 = height_in_cells.min(next_grid.rows().saturating_sub(1));
                    grid = next_grid;
                }
            }
        }
    }

    let target_rect = grid.cell_range_rect(range);
    placement::place_window(hwnd, target_rect);

    if let Ok(mut overlay_guard) = OVERLAY.lock() {
        let manager = overlay_guard.get_or_insert_with(|| {
            overlay::OverlayManager::new(grid::GridOptions::default())
        });
        let _ = manager.show_with_state(monitor_idx, None);
        let state = overlay::render::OverlayState {
            halo: None,
            selection: Some(target_rect),
        };
        let _ = manager.update(monitor_idx, &state);
    }

    *guard = Some(KeyboardNavState {
        hwnd,
        monitor_idx,
        range,
    });
}

fn apply_session_action(action: session::SessionAction) {
    match action {
        session::SessionAction::None => {}
        session::SessionAction::ShowGrid { monitor, edge } => {
            let halo = edge.map(|e| overlay::render::HaloState {
                edge: e,
                thickness: session::HALO_THICKNESS,
            });
            if let Ok(mut guard) = OVERLAY.lock() {
                let manager = guard.get_or_insert_with(|| {
                    overlay::OverlayManager::new(grid::GridOptions::default())
                });
                match manager.show_with_state(monitor, halo) {
                    Ok(()) => info!("grille affichÃ©e, bord {:?}", edge),
                    Err(err) => error!("affichage de la grille : {err}"),
                }
            }
        }
        session::SessionAction::HideGrid => {
            if let Ok(mut guard) = OVERLAY.lock() {
                if let Some(manager) = guard.as_mut() {
                    manager.hide();
                }
            }
        }
        session::SessionAction::AwaitingSelection {
            monitor,
            target_window,
        } => {
            if let Some(raw) = target_window {
                let hwnd = HWND(raw as *mut core::ffi::c_void);
                if !hwnd.0.is_null() && placement::is_placeable_window(hwnd) {
                    let top = placement::get_top_level_window_for_hwnd(hwnd);
                    if placement::is_placeable_window(top) {
                        unsafe {
                            let _ = ShowWindow(top, SW_HIDE);
                        }
                    }
                }
            }
            if let Ok(mut guard) = OVERLAY.lock() {
                let manager = guard.get_or_insert_with(|| {
                    overlay::OverlayManager::new(grid::GridOptions::default())
                });
                let _ = manager.show_with_state(monitor, None);
                let state = overlay::render::OverlayState {
                    halo: None,
                    selection: None,
                };
                if let Err(err) = manager.update(monitor, &state) {
                    error!("mise Ã  jour de la grille : {err}");
                }
            }
        }
        session::SessionAction::UpdateGrid { monitor, selection } => {
            if let Ok(mut guard) = OVERLAY.lock() {
                let manager = guard.get_or_insert_with(|| {
                    overlay::OverlayManager::new(grid::GridOptions::default())
                });
                let state = overlay::render::OverlayState {
                    halo: None,
                    selection,
                };
                if let Err(err) = manager.update(monitor, &state) {
                    error!("mise Ã  jour de la grille : {err}");
                }
            }
        }
        session::SessionAction::Cancel { target_window } => {
            if let Some(raw) = target_window {
                let hwnd = HWND(raw as *mut core::ffi::c_void);
                if !hwnd.0.is_null() {
                    let hwnd = placement::get_top_level_window_for_hwnd(hwnd);
                    unsafe {
                        let _ = ShowWindow(hwnd, SW_SHOW);
                        let _ = SetForegroundWindow(hwnd);
                    }
                }
            }
            if let Ok(mut guard) = OVERLAY.lock() {
                if let Some(manager) = guard.as_mut() {
                    manager.hide();
                }
            }
        }
        session::SessionAction::Place { target_window, rect } => {
            let hwnd = target_window
                .map(|w| HWND(w as *mut core::ffi::c_void))
                .unwrap_or_else(|| unsafe { GetForegroundWindow() });
            if !hwnd.0.is_null() {
                let placed = placement::place_window(hwnd, rect);
                info!("fenÃªtre {:?} placÃ©e sur {:?} (succÃ¨s={})", hwnd, rect, placed);
            }
            if let Ok(mut guard) = OVERLAY.lock() {
                if let Some(manager) = guard.as_mut() {
                    manager.hide();
                }
            }
        }
    }
}

unsafe extern "system" fn core_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let tr = TR.get();
    match msg {
        WM_DESTROY => {
            hooks::uninstall();
            tray::remove(hwnd);
            unsafe {
                let _ = UnregisterHotKey(Some(hwnd), HOTKEY_TOGGLE_ID);
                let _ = UnregisterHotKey(Some(hwnd), HOTKEY_LEFT_ID);
                let _ = UnregisterHotKey(Some(hwnd), HOTKEY_RIGHT_ID);
                let _ = UnregisterHotKey(Some(hwnd), HOTKEY_UP_ID);
                let _ = UnregisterHotKey(Some(hwnd), HOTKEY_DOWN_ID);
            }
            PostQuitMessage(0);
            LRESULT(0)
        }
        WM_QUERYENDSESSION => LRESULT(1),
        WM_ENDSESSION => {
            tray::remove(hwnd);
            if wparam.0 != 0 {
                PostQuitMessage(0);
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            handle_tray_action(hwnd, tray::on_command(wparam));
            LRESULT(0)
        }
        WM_DISPLAYCHANGE | WM_SETTINGCHANGE => {
            log_monitors();
            info!("disposition d'Ã©cran modifiÃ©e, moniteurs renumÃ©rotÃ©s");
            if let Ok(mut guard) = OVERLAY.lock() {
                if let Some(manager) = guard.as_mut() {
                    if let Err(err) = manager.on_display_change() {
                        error!("reconstruction des overlays : {err}");
                    }
                }
            }
            LRESULT(0)
        }
        WM_HOTKEY => {
            match wparam.0 as i32 {
                HOTKEY_TOGGLE_ID => toggle_overlay(),
                HOTKEY_LEFT_ID => handle_arrow_key(arrow_snap::ArrowKey::Left),
                HOTKEY_RIGHT_ID => handle_arrow_key(arrow_snap::ArrowKey::Right),
                HOTKEY_UP_ID => handle_arrow_key(arrow_snap::ArrowKey::Up),
                HOTKEY_DOWN_ID => handle_arrow_key(arrow_snap::ArrowKey::Down),
                _ => {}
            }
            LRESULT(0)
        }
        other if other == tray::WM_TRAY => {
            if let Some(tr) = tr {
                handle_tray_action(hwnd, tray::on_callback(hwnd, lparam, tr));
            }
            LRESULT(0)
        }
        other if other == SHOW_GRID_MSG.load(Ordering::Relaxed) => {
            info!("bascule de la grille demandÃ©e par une seconde instance");
            toggle_overlay();
            LRESULT(0)
        }
        other if other == TASKBAR_CREATED_MSG.load(Ordering::Relaxed) => {
            if let Some(tr) = tr {
                if let Err(err) = tray::add(hwnd, tr) {
                    error!("rÃ©ajout de l'icÃ´ne tray aprÃ¨s redÃ©marrage d'explorer : {err}");
                }
            }
            update_arrow_hotkeys(hwnd, snap::is_snap_enabled());
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Enregistre ou dÃ©senregistre les raccourcis Ctrl+Alt+FlÃ¨ches selon l'Ã©tat de l'ancrage Windows.
fn update_arrow_hotkeys(hwnd: HWND, snap_enabled: bool) {
    unsafe {
        if snap_enabled {
            let _ = UnregisterHotKey(Some(hwnd), HOTKEY_LEFT_ID);
            let _ = UnregisterHotKey(Some(hwnd), HOTKEY_RIGHT_ID);
            let _ = UnregisterHotKey(Some(hwnd), HOTKEY_UP_ID);
            let _ = UnregisterHotKey(Some(hwnd), HOTKEY_DOWN_ID);
            info!("ancrage Windows actif : raccourcis Ctrl+Alt+FlÃ¨ches libÃ©rÃ©s");
        } else {
            let r1 = RegisterHotKey(Some(hwnd), HOTKEY_LEFT_ID, MOD_CONTROL | MOD_ALT | MOD_NOREPEAT, VK_LEFT);
            let r2 = RegisterHotKey(Some(hwnd), HOTKEY_RIGHT_ID, MOD_CONTROL | MOD_ALT | MOD_NOREPEAT, VK_RIGHT);
            let r3 = RegisterHotKey(Some(hwnd), HOTKEY_UP_ID, MOD_CONTROL | MOD_ALT | MOD_NOREPEAT, VK_UP);
            let r4 = RegisterHotKey(Some(hwnd), HOTKEY_DOWN_ID, MOD_CONTROL | MOD_ALT | MOD_NOREPEAT, VK_DOWN);
            info!(
                "ancrage Windows inactif : raccourcis Ctrl+Alt+FlÃ¨ches enregistrÃ©s (G={:?}, D={:?}, H={:?}, B={:?})",
                r1.is_ok(), r2.is_ok(), r3.is_ok(), r4.is_ok()
            );
        }
    }
}

/// Place la fenÃªtre active sur Ctrl+Alt+FlÃ¨che : moitiÃ©, quart, coin, ou plein Ã©cran.
fn handle_arrow_key(key: arrow_snap::ArrowKey) {
    let fg = unsafe { GetForegroundWindow() };
    if fg.0.is_null() {
        return;
    }
    let hwnd = placement::get_top_level_window_for_hwnd(fg);
    if hwnd.0.is_null() || !placement::is_placeable_window(hwnd) {
        return;
    }

    let mut window_rect = RECT::default();
    if unsafe { GetWindowRect(hwnd, &mut window_rect) }.is_err() {
        return;
    }
    let center_x = (window_rect.left + window_rect.right) / 2;
    let center_y = (window_rect.top + window_rect.bottom) / 2;

    let Some((monitor_idx, monitor)) = monitors::current()
        .into_iter()
        .enumerate()
        .find(|(_, m)| m.bounds.contains(center_x, center_y))
    else {
        return;
    };
    let area = monitors::effective_work(monitor.work);

    let rect = ARROW_SNAP
        .lock()
        .expect("verrou arrow snap")
        .get_or_insert_with(arrow_snap::ArrowSnap::new)
        .press(hwnd.0 as usize, key, area);

    let placed = placement::place_window(hwnd, rect);
    info!("Ctrl+Alt+flÃ¨che ({:?}) : fenÃªtre {:?} placÃ©e sur {:?} (succÃ¨s={})", key, hwnd, rect, placed);

    // Initialiser l'Ã©tat de navigation grille si l'utilisateur enchaÃ®ne avec Alt+FlÃ¨ches
    if let Some(grid) = grid::Grid::new(area, grid::GridOptions::default()) {
        let range = grid.range_for_rect(rect);
        if let Ok(mut nav_guard) = KBD_NAV.lock() {
            *nav_guard = Some(KeyboardNavState {
                hwnd,
                monitor_idx,
                range,
            });
        }
    }
}

/// Bascule l'affichage de la grille (overlay).
fn toggle_overlay() {
    let is_visible = OVERLAY
        .lock()
        .ok()
        .and_then(|g| g.as_ref().map(|m| m.is_visible()))
        .unwrap_or(false);

    if is_visible {
        if let Ok(mut guard) = OVERLAY.lock() {
            if let Some(manager) = guard.as_mut() {
                manager.hide();
            }
        }
        if let Ok(mut session_guard) = SESSION.lock() {
            if let Some(session) = session_guard.as_mut() {
                let action = session.cancel();
                apply_session_action(action);
            }
        }
    } else {
        let action = SESSION
            .lock()
            .expect("verrou session")
            .get_or_insert_with(|| session::Session::new(grid::GridOptions::default()))
            .open_manual(None);
        apply_session_action(action);
    }
}

fn handle_tray_action(hwnd: HWND, action: tray::TrayAction) {
    match action {
        tray::TrayAction::None => {}
        tray::TrayAction::ShowGrid => {
            toggle_overlay();
        }
        tray::TrayAction::ToggleStartup => {
            let was_enabled = startup::is_enabled();
            if startup::set_enabled(!was_enabled) {
                info!("dÃ©marrage avec Windows {}", if was_enabled { "dÃ©sactivÃ©" } else { "activÃ©" });
            } else {
                error!("Ã©chec de la bascule du dÃ©marrage avec Windows");
            }
        }
        tray::TrayAction::ToggleSnap => {
            let was_enabled = snap::is_snap_enabled();
            let new_enabled = !was_enabled;
            let label = if new_enabled { "activÃ©" } else { "dÃ©sactivÃ©" };
            if snap::set_snap(new_enabled) {
                info!("ancrage Windows {label}");
                snap::restart_explorer();
                update_arrow_hotkeys(hwnd, new_enabled);
            } else {
                error!("Ã©chec de la bascule de l'ancrage Windows");
            }
        }
        tray::TrayAction::Quit => {
            unsafe {
                let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        }
    }
}