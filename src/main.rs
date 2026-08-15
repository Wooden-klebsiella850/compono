//! Point d'entrée. Initialise le logging, l'instance unique, une fenêtre cachée
//! qui sert de support au tray et aux notifications, puis lance la boucle de messages.

// Masque la console en release ; gardée en debug pour lire les logs directement.
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
use windows::Win32::UI::Input::KeyboardAndMouse::{MOD_ALT, MOD_NOREPEAT, MOD_WIN, RegisterHotKey};
use windows::Win32::UI::WindowsAndMessaging::*;

use i18n::I18n;
use single_instance::InstanceGuard;

/// Traductions chargées, accessibles depuis la procédure de fenêtre.
static TR: OnceLock<I18n> = OnceLock::new();

/// Id du message "afficher la grille" reçu depuis une deuxième instance.
static SHOW_GRID_MSG: AtomicU32 = AtomicU32::new(0);

/// Id du message "TaskbarCreated" pour recréer l'icône après un redémarrage d'explorer.
static TASKBAR_CREATED_MSG: AtomicU32 = AtomicU32::new(0);

/// Conteneur mono-thread : les ressources COM/HWND ne sont pas Send, tous les
/// accès se font sur la thread UI (boucle de messages), sous mutex.
struct UiCell<T>(Mutex<Option<T>>);
unsafe impl<T> Send for UiCell<T> {}
unsafe impl<T> Sync for UiCell<T> {}

impl<T> std::ops::Deref for UiCell<T> {
    type Target = Mutex<Option<T>>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Gestionnaire d'overlays et machine à états du geste, côté procédure de fenêtre.
static OVERLAY: UiCell<overlay::OverlayManager> = UiCell(Mutex::new(None));
static SESSION: UiCell<session::Session> = UiCell(Mutex::new(None));

/// État persistant du placement au clavier Win+Flèche (moitié/quart par fenêtre).
static ARROW_SNAP: UiCell<arrow_snap::ArrowSnap> = UiCell(Mutex::new(None));

/// Id du raccourci global qui bascule l'affichage de la grille.
const HOTKEY_TOGGLE_ID: i32 = 1;
const HOTKEY_LEFT_ID: i32 = 2;
const HOTKEY_RIGHT_ID: i32 = 3;
const HOTKEY_UP_ID: i32 = 4;
const HOTKEY_DOWN_ID: i32 = 5;

// Codes de touches virtuelles fléchées : absents des constantes VK_ importées.
const VK_LEFT: u32 = 0x25;
const VK_UP: u32 = 0x26;
const VK_RIGHT: u32 = 0x27;
const VK_DOWN: u32 = 0x28;

fn main() {
    // Instance unique avant toute initialisation. La seconde instance notifie et s'arrête.
    let guard = match InstanceGuard::acquire() {
        Some(guard) => guard,
        None => return,
    };

    let appdata = appdata_dir();

    if let Err(err) = logging::init(&appdata) {
        eprintln!("impossible d'initialiser le log : {err}");
    }
    // Le manifeste déclare déjà PerMonitorV2, l'appel garde la main si le manifeste est retiré.
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }

    let config = config::load(&config::config_path(&appdata));
    let system_lang = default_lang();
    let lang = config.lang.as_deref().unwrap_or(&system_lang);
    let tr = i18n::I18n::load(lang, &appdata.join("locales"));
    info!("{} v{} démarre", tr.t("app.name"), env!("CARGO_PKG_VERSION"));
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
            error!("échec de création de la fenêtre principale");
            return;
        }
    };

    // Win+Alt+G : bascule l'affichage de la grille. Enregistré sur la fenêtre
    // pour que WM_HOTKEY lui soit adressé directement.
    unsafe {
        let _ = RegisterHotKey(
            Some(hwnd),
            HOTKEY_TOGGLE_ID,
            MOD_WIN | MOD_ALT | MOD_NOREPEAT,
            'G' as u32,
        );
    }

    // Win+Flèche : équivalent de l'ancrage natif, utile pendant qu'il est désactivé.
    unsafe {
        let _ = RegisterHotKey(Some(hwnd), HOTKEY_LEFT_ID, MOD_WIN | MOD_NOREPEAT, VK_LEFT);
        let _ = RegisterHotKey(Some(hwnd), HOTKEY_RIGHT_ID, MOD_WIN | MOD_NOREPEAT, VK_RIGHT);
        let _ = RegisterHotKey(Some(hwnd), HOTKEY_UP_ID, MOD_WIN | MOD_NOREPEAT, VK_UP);
        let _ = RegisterHotKey(Some(hwnd), HOTKEY_DOWN_ID, MOD_WIN | MOD_NOREPEAT, VK_DOWN);
    }

    // Hooks souris et drag, installés sur la thread UI.
    if let Err(err) = hooks::install() {
        error!("échec de l'installation des hooks : {err}");
    }
    SESSION
        .lock()
        .expect("verrou session")
        .get_or_insert_with(|| session::Session::new(grid::GridOptions::default()));
    if let Err(err) = tray::add(hwnd, TR.get().expect("i18n non initialisé")) {
        error!("échec d'ajout de l'icône tray : {err}");
    }

    info!("boucle de messages démarrée");
    run_message_loop();

    // `guard` est détruit ici, le mutex est libéré.
    info!("arrêt propre");
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

/// Énumère les écrans et logge la grille par défaut de chacun.
fn log_monitors() {
    let monitors = monitors::reload();
    info!("{} moniteur(s) détecté(s)", monitors.len());
    let autohide = monitors::taskbar_autohide_edge();
    for monitor in &monitors {
        let work = monitors::reserve_autohide(monitor.work, autohide);
        match grid::Grid::new(work, grid::GridOptions::default()) {
            Some(g) => info!(
                "écran {:?} (primary={}, dpi={}) : zone {}x{}, grille {}x{}",
                monitor.handle,
                monitor.is_primary,
                monitor.dpi(),
                g.area().width,
                g.area().height,
                g.cols(),
                g.rows()
            ),
            None => info!("écran {:?} : zone de travail trop petite", monitor.handle),
        }
    }
}

/// Crée la fenêtre cachée qui recevra les messages du tray et des autres instances.
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
            error!("RegisterClassW a échoué");
            return None;
        }

        match CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("Compono.Core"),
            w!("Compono"),
            WS_OVERLAPPED,
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
            // PeekMessage plutôt que GetMessage : les messages des hooks bas
            // niveau sont consommés à l'intérieur de GetMessage et ne reviennent
            // jamais au code, il faut donc draine la file à chaque itération.
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

/// Consomme les événements souris/drag et applique les actions de la session.
fn process_input_events() {
    for event in hooks::drain() {
        match event {
            hooks::InputEvent::MouseMove { .. } => {}
            _ => info!("entrée : {event:?}"),
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
                    Ok(()) => info!("grille affichée, bord {:?}", edge),
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
            // Masquer temporairement la fenêtre pour libérer la vue pendant le tracé
            if let Some(raw) = target_window {
                let hwnd = HWND(raw as *mut core::ffi::c_void);
                if !hwnd.0.is_null() {
                    unsafe {
                        let _ = ShowWindow(placement::get_top_level_window_for_hwnd(hwnd), SW_HIDE);
                    }
                }
            }
            if let Ok(mut guard) = OVERLAY.lock() {
                let manager = guard.get_or_insert_with(|| {
                    overlay::OverlayManager::new(grid::GridOptions::default())
                });
                let state = overlay::render::OverlayState {
                    halo: None,
                    selection: None,
                };
                if let Err(err) = manager.update(monitor, &state) {
                    error!("mise à jour de la grille : {err}");
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
                    error!("mise à jour de la grille : {err}");
                }
            }
        }
        session::SessionAction::Cancel { target_window } => {
            // Restaurer la fenêtre cible si elle avait été masquée
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
                info!("fenêtre {:?} placée sur {:?} (succès={})", hwnd, rect, placed);
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
            info!("disposition d'écran modifiée, moniteurs renumérotés");
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
            info!("bascule de la grille demandée par une seconde instance");
            toggle_overlay();
            LRESULT(0)
        }
        other if other == TASKBAR_CREATED_MSG.load(Ordering::Relaxed) => {
            if let Some(tr) = tr {
                if let Err(err) = tray::add(hwnd, tr) {
                    error!("réajout de l'icône tray après redémarrage d'explorer : {err}");
                }
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Place la fenêtre active sur Win+Flèche : moitié d'écran, quart si la même
/// flèche est rappuyée, coin si un axe horizontal et un axe vertical sont combinés.
fn handle_arrow_key(key: arrow_snap::ArrowKey) {
    let fg = unsafe { GetForegroundWindow() };
    if !placement::is_placeable_window(fg) {
        return;
    }
    let hwnd = placement::get_top_level_window_for_hwnd(fg);
    if hwnd.0.is_null() {
        return;
    }

    let mut window_rect = RECT::default();
    if unsafe { GetWindowRect(hwnd, &mut window_rect) }.is_err() {
        return;
    }
    let center_x = (window_rect.left + window_rect.right) / 2;
    let center_y = (window_rect.top + window_rect.bottom) / 2;

    let Some(monitor) = monitors::current()
        .into_iter()
        .find(|m| m.bounds.contains(center_x, center_y))
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
    info!("Win+flèche : fenêtre {:?} placée sur {:?} (succès={})", hwnd, rect, placed);
}

/// Bascule l'affichage de la grille (overlay).
fn toggle_overlay() {
    let action = SESSION
        .lock()
        .expect("verrou session")
        .get_or_insert_with(|| session::Session::new(grid::GridOptions::default()))
        .open_manual(None);
    apply_session_action(action);
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
                info!("démarrage avec Windows {}", if was_enabled { "désactivé" } else { "activé" });
            } else {
                error!("échec de la bascule du démarrage avec Windows");
            }
        }
        tray::TrayAction::ToggleSnap => {
            let was_enabled = snap::is_snap_enabled();
            let label = if was_enabled { "désactivé" } else { "activé" };
            if snap::set_snap(!was_enabled) {
                info!("ancrage Windows {label}");
                snap::restart_explorer();
            } else {
                error!("échec de la bascule de l'ancrage Windows");
            }
        }
        tray::TrayAction::Quit => {
            unsafe {
                let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        }
    }
}
