//! Point d'entrée. Initialise le logging, l'instance unique, une fenêtre cachée
//! qui sert de support au tray et aux notifications, puis lance la boucle de messages.

mod config;
mod i18n;
mod logging;
mod single_instance;

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use log::{error, info};
use windows::core::w;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::WindowsAndMessaging::*;

use single_instance::InstanceGuard;

/// Id du message "afficher la grille" reçu depuis une deuxième instance.
static SHOW_GRID_MSG: AtomicU32 = AtomicU32::new(0);

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

    SHOW_GRID_MSG.store(guard.show_grid_message(), Ordering::Relaxed);

    if !create_core_window() {
        error!("échec de création de la fenêtre principale");
        return;
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

/// Crée la fenêtre cachée qui recevra les messages du tray et des autres instances.
fn create_core_window() -> bool {
    unsafe {
        let hinstance = match GetModuleHandleW(None) {
            Ok(h) => h,
            Err(err) => {
                error!("GetModuleHandleW : {err}");
                return false;
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
            return false;
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
            Ok(_) => true,
            Err(err) => {
                error!("CreateWindowExW : {err}");
                false
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
    match msg {
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        WM_QUERYENDSESSION => LRESULT(1),
        WM_ENDSESSION => {
            if wparam.0 != 0 {
                PostQuitMessage(0);
            }
            LRESULT(0)
        }
        other if other == SHOW_GRID_MSG.load(Ordering::Relaxed) => {
            // Phase 3 : basculer l'affichage de la grille. La seconde instance a demandé.
            info!("demande d'affichage de la grille reçue");
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
