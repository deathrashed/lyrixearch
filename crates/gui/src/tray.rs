//! Menu-bar / system-tray icon: shows what the search engine is doing (the brand's "LS" mark with a
//! status lamp) and offers the main actions without bringing the window forward.
//!
//! States follow the brand package: idle = blue, fetching = yellow, complete = green,
//! stopped = red, error = red with an exclamation mark. The tooltip always says the same in words.

use std::sync::Mutex;
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, Theme};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayState {
    Idle,
    Fetching,
    Complete,
    Stopped,
    Error,
}

/// Current state and tooltip, kept so the icon can be redrawn when the system theme changes.
pub struct Tray {
    icon: TrayIcon,
    now: Mutex<(TrayState, String)>,
}

/// PNG for a state on a dark or light panel. macOS gets the 44 px image (22 pt at 2x); other systems
/// scale the 32 px one.
fn image(state: TrayState, dark_panel: bool) -> Option<Image<'static>> {
    macro_rules! pick {
        ($panel:literal, $px:literal) => {
            match state {
                TrayState::Idle => &include_bytes!(concat!("../icons/tray/", $panel, "-idle-", $px, ".png"))[..],
                TrayState::Fetching => &include_bytes!(concat!("../icons/tray/", $panel, "-fetching-", $px, ".png"))[..],
                TrayState::Complete => &include_bytes!(concat!("../icons/tray/", $panel, "-complete-", $px, ".png"))[..],
                TrayState::Stopped => &include_bytes!(concat!("../icons/tray/", $panel, "-stopped-", $px, ".png"))[..],
                TrayState::Error => &include_bytes!(concat!("../icons/tray/", $panel, "-error-", $px, ".png"))[..],
            }
        };
    }
    let bytes: &'static [u8] = match (dark_panel, cfg!(target_os = "macos")) {
        (true, true) => pick!("dark", "44"),
        (true, false) => pick!("dark", "32"),
        (false, true) => pick!("light", "44"),
        (false, false) => pick!("light", "32"),
    };
    Image::from_bytes(bytes).ok()
}

/// Dark panel (white letters) unless the system is in light mode. The macOS menu bar and most Linux
/// panels follow the system theme; the main window reports it.
fn dark_panel(app: &AppHandle) -> bool {
    app.get_window("main").and_then(|w| w.theme().ok()).is_none_or(|t| t != Theme::Light)
}

/// Create the tray icon and its menu. A failure (no tray support on this desktop) is not fatal.
pub fn install(app: &AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Show Lyrixearch", true, None::<&str>)?;
    let find = MenuItem::with_id(app, "find-all", "Fetch lyrics", true, None::<&str>)?;
    let stop = MenuItem::with_id(app, "stop", "Stop search", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Lyrixearch", true, None::<&str>)?;
    let sep = || PredefinedMenuItem::separator(app);
    let menu = Menu::with_items(app, &[&show, &sep()?, &find, &stop, &sep()?, &quit])?;

    let tooltip = "Lyrixearch: ready";
    let mut builder = TrayIconBuilder::with_id("main")
        .tooltip(tooltip)
        .menu(&menu)
        // a left click shows the window on Windows and Linux; macOS opens the menu, as menu-bar items do
        .show_menu_on_left_click(cfg!(target_os = "macos"))
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => show_main(app),
            // the UI owns the track list, so it decides what "all" means (same as its own button)
            "find-all" => {
                let _ = app.emit("tray-action", "find-all");
            }
            "stop" => {
                let _ = app.emit("tray-action", "stop");
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                if !cfg!(target_os = "macos") {
                    show_main(tray.app_handle());
                }
            }
        });
    if let Some(img) = image(TrayState::Idle, dark_panel(app)) {
        builder = builder.icon(img);
    }
    #[cfg(target_os = "macos")]
    {
        // coloured status lamps: no automatic template tinting
        builder = builder.icon_as_template(false);
    }
    let icon = builder.build(app)?;
    app.manage(Tray { icon, now: Mutex::new((TrayState::Idle, tooltip.into())) });
    Ok(())
}

fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// Switch the icon and tooltip. Does nothing when there is no tray (unsupported desktop).
pub fn set(app: &AppHandle, state: TrayState, tooltip: impl Into<String>) {
    let Some(tray) = app.try_state::<Tray>() else { return };
    let tooltip = format!("Lyrixearch: {}", tooltip.into());
    let changed = {
        let mut now = tray.now.lock().unwrap();
        let changed = now.0 != state;
        *now = (state, tooltip.clone());
        changed
    };
    if changed {
        let _ = tray.icon.set_icon(image(state, dark_panel(app)));
        #[cfg(target_os = "macos")]
        let _ = tray.icon.set_icon_as_template(false);
    }
    let _ = tray.icon.set_tooltip(Some(&tooltip));
}

/// Redraw for the current theme (dark/light menu bar or panel).
pub fn refresh_theme(app: &AppHandle) {
    let Some(tray) = app.try_state::<Tray>() else { return };
    let state = tray.now.lock().unwrap().0;
    let _ = tray.icon.set_icon(image(state, dark_panel(app)));
}
