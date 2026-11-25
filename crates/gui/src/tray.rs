//! Menu-bar / system-tray icon: the brand's symbolic "LS" mark, with the main actions in its menu.
//!
//! macOS gets a template image (the system tints it with the other menu-bar icons and inverts it
//! while the menu is open). Sized after Bjango's "Designing menu bar extras": no padding, and about
//! the weight of a 16 pt circle, so the letters are 10 pt tall. Other systems get the white or dark
//! mark for the panel. The search state is not drawn in the icon: it is the first line of the menu
//! and the tooltip.

use std::sync::Mutex;
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, Theme, Wry};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayState {
    Idle,
    Fetching,
    Complete,
    Stopped,
    Error,
}

pub struct Tray {
    icon: TrayIcon,
    /// Disabled first menu item that says what is happening ("Ready", "Fetching 3 of 20").
    status: MenuItem<Wry>,
    stop: MenuItem<Wry>,
    now: Mutex<TrayState>,
}

fn image(dark_panel: bool) -> Option<Image<'static>> {
    let bytes: &'static [u8] = if cfg!(target_os = "macos") {
        include_bytes!("../icons/tray/lsTemplate-mac.png")
    } else if dark_panel {
        include_bytes!("../icons/tray/ls-dark-32.png")
    } else {
        include_bytes!("../icons/tray/ls-light-32.png")
    };
    Image::from_bytes(bytes).ok()
}

/// Dark panel (white mark) unless the system is in light mode. Most Linux panels and the Windows
/// taskbar follow the system theme; the main window reports it. (macOS tints the template itself.)
fn dark_panel(app: &AppHandle) -> bool {
    app.get_window("main").and_then(|w| w.theme().ok()).is_none_or(|t| t != Theme::Light)
}

/// Create the tray icon and its menu. A failure (no tray support on this desktop) is not fatal.
pub fn install(app: &AppHandle) -> tauri::Result<()> {
    let status = MenuItem::with_id(app, "status", "Ready", false, None::<&str>)?;
    let show = MenuItem::with_id(app, "show", "Show Lyrixearch", true, None::<&str>)?;
    let find = MenuItem::with_id(app, "find-all", "Fetch", true, None::<&str>)?;
    let stop = MenuItem::with_id(app, "stop", "Stop", false, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Lyrixearch", true, None::<&str>)?;
    let sep = || PredefinedMenuItem::separator(app);
    let menu = Menu::with_items(app, &[&status, &sep()?, &show, &find, &stop, &sep()?, &quit])?;

    let mut builder = TrayIconBuilder::with_id("main")
        .tooltip("Lyrixearch: ready")
        .menu(&menu)
        // a left click shows the window on Windows and Linux; macOS opens the menu, as menu-bar items do
        .show_menu_on_left_click(cfg!(target_os = "macos"))
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => show_main(app),
            // the UI owns the track list, so it decides what "all" means and asks first, as its own button does
            "find-all" => {
                show_main(app);
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
    if let Some(img) = image(dark_panel(app)) {
        builder = builder.icon(img);
    }
    #[cfg(target_os = "macos")]
    {
        builder = builder.icon_as_template(true);
    }
    let icon = builder.build(app)?;
    app.manage(Tray { icon, status, stop, now: Mutex::new(TrayState::Idle) });
    Ok(())
}

fn show_main(app: &AppHandle) {
    crate::background::show_main(app);
}

/// Update the status line and tooltip. Does nothing when there is no tray (unsupported desktop).
pub fn set(app: &AppHandle, state: TrayState, text: impl Into<String>) {
    let Some(tray) = app.try_state::<Tray>() else { return };
    let text = text.into();
    *tray.now.lock().unwrap() = state;
    let mut chars = text.chars();
    let line = chars.next().map(|c| c.to_uppercase().chain(chars).collect::<String>()).unwrap_or_default();
    let _ = tray.status.set_text(&line);
    let _ = tray.stop.set_enabled(state == TrayState::Fetching);
    let _ = tray.icon.set_tooltip(Some(format!("Lyrixearch: {text}")));
}

/// Redraw for the current theme (dark/light panel). macOS needs nothing: the template follows it.
pub fn refresh_theme(app: &AppHandle) {
    if cfg!(target_os = "macos") {
        return;
    }
    let Some(tray) = app.try_state::<Tray>() else { return };
    let _ = tray.icon.set_icon(image(dark_panel(app)));
}
