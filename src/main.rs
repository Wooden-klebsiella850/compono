//! Point d'entrée. Initialise le logging, l'instance unique, une fenêtre cachée
//! qui sert de support au tray et aux notifications, puis lance la boucle de messages.

mod config;
mod i18n;
mod logging;
mod monitors;
mod single_instance;
mod snap;
mod tray;
// Le modèle de grille est exercé par les tests, son usage dans le binaire arrive
// avec l'overlay (phase 3) et le placement (phase 5).
#[cfg_attr(not(test), allow(dead_code))]
mod grid;

use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};

use log::{error, info};
use windows::core::w;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::WindowsAndMessaging::*;

use i18n::I18n;
use single_instance::InstanceGuard;

/// Traductions chargées, accessibles depuis la procédure de fenêtre.
static TR: OnceLock<I18n> = OnceLock::new();

/// Id du message "afficher la grille" reçu depuis une deuxième instance.
static SHOW_GRID_MSG: AtomicU32 = AtomicU32::new(0);

/// Id du message "TaskbarCreated" pour recréer l'icône après un redémarrage d'explorer.
static TASKBAR_CREATED_MSG: AtomicU32 = AtomicU32::new(0);

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

    let hwnd = match create_core_window() {
        Some(hwnd) => hwnd,
        None => {
            error!("échec de création de la fenêtre principale");
            return;
        }
    };
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
            let result = GetMessageW(&mut msg, None, 0, 0);
            if result.0 <= 0 {
                break;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
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
            LRESULT(0)
        }
        other if other == tray::WM_TRAY => {
            if let Some(tr) = tr {
                handle_tray_action(hwnd, tray::on_callback(hwnd, lparam, tr));
            }
            LRESULT(0)
        }
        other if other == SHOW_GRID_MSG.load(Ordering::Relaxed) => {
            // Phase 3 : basculer l'affichage de la grille. La seconde instance a demandé.
            info!("demande d'affichage de la grille reçue");
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

fn handle_tray_action(hwnd: HWND, action: tray::TrayAction) {
    match action {
        tray::TrayAction::None => {}
        tray::TrayAction::ShowGrid => {
            info!("bascule de la grille demandée (phase 3)");
        }
        tray::TrayAction::Configure => {
            info!("configuration demandée (phase 8)");
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
