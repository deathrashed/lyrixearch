//! Working in the background: keeping the computer awake during a fetch, a notification when it
//! finishes, and hiding to the menu bar / tray when the window is closed.

use tauri::{AppHandle, Manager};

/// Keeps the computer from sleeping until dropped. Best effort: when the system tool is missing the
/// fetch simply runs without it.
pub struct Awake(#[allow(dead_code)] Inner);

#[cfg(target_os = "macos")]
type Inner = Option<std::process::Child>;
#[cfg(target_os = "linux")]
type Inner = Option<std::process::Child>;
#[cfg(windows)]
type Inner = Option<std::sync::mpsc::Sender<()>>;
#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
type Inner = ();

impl Awake {
    pub fn start() -> Self {
        #[cfg(target_os = "macos")]
        {
            // -i: no idle sleep while it runs; -w: and never outlive this app
            let child = std::process::Command::new("/usr/bin/caffeinate").args(["-i", "-w", &std::process::id().to_string()]).spawn().ok();
            Awake(child)
        }
        #[cfg(target_os = "linux")]
        {
            let child = std::process::Command::new("systemd-inhibit")
                .args(["--what=idle:sleep", "--who=Lyrixearch", "--why=Fetching lyrics", "--mode=block", "sleep", "infinity"])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .ok();
            Awake(child)
        }
        #[cfg(windows)]
        {
            use windows_sys::Win32::System::Power::{SetThreadExecutionState, ES_CONTINUOUS, ES_SYSTEM_REQUIRED};
            // the request belongs to the thread that made it, so a thread holds it until released
            let (tx, rx) = std::sync::mpsc::channel::<()>();
            std::thread::spawn(move || unsafe {
                SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED);
                let _ = rx.recv();
                SetThreadExecutionState(ES_CONTINUOUS);
            });
            Awake(Some(tx))
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
        Awake(())
    }
}

#[cfg(unix)]
impl Drop for Awake {
    fn drop(&mut self) {
        if let Some(child) = self.0.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// True when the main window is hidden, minimised or behind other apps.
pub fn in_background(app: &AppHandle) -> bool {
    match app.get_window("main") {
        Some(w) => !w.is_visible().unwrap_or(true) || w.is_minimized().unwrap_or(false) || !w.is_focused().unwrap_or(true),
        None => true,
    }
}

pub fn notify(app: &AppHandle, title: &str, body: &str) {
    use tauri_plugin_notification::NotificationExt;
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        eprintln!("notification failed: {e}");
    }
}

/// Hide the main window; on macOS the Dock icon goes too, so the app lives in the menu bar.
pub fn hide_main(app: &AppHandle) {
    if let Some(w) = app.get_window("main") {
        let _ = w.hide();
    }
    #[cfg(target_os = "macos")]
    let _ = app.set_activation_policy(tauri::ActivationPolicy::Accessory);
}

/// Bring the main window back (and the Dock icon on macOS).
pub fn show_main(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    let _ = app.set_activation_policy(tauri::ActivationPolicy::Regular);
    if let Some(w) = app.get_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// Height of the app's own header, which on macOS also holds the window buttons.
#[cfg(target_os = "macos")]
const HEADER: f64 = 52.0;

/// macOS: centre the close / minimise / zoom buttons in the app's 52 pt header (the overlay title bar
/// would leave them at the top of a 28 pt strip). AppKit resets them on resize, so this runs again then.
#[cfg(target_os = "macos")]
pub fn place_window_buttons(window: &tauri::Window) {
    use objc2_app_kit::{NSWindow, NSWindowButton};
    use objc2_foundation::NSPoint;
    let Ok(ptr) = window.ns_window() else { return };
    if ptr.is_null() {
        return;
    }
    let ns: &NSWindow = unsafe { &*(ptr as *const NSWindow) };
    let buttons: Vec<_> = [NSWindowButton::CloseButton, NSWindowButton::MiniaturizeButton, NSWindowButton::ZoomButton]
        .into_iter()
        .filter_map(|b| ns.standardWindowButton(b))
        .collect();
    let [close, mini, _zoom] = buttons.as_slice() else { return };
    // the buttons sit in the title bar view, inside the title bar container: make the container as tall as the header
    // SAFETY: called on the main thread (window events and setup), where AppKit's view tree may be read
    let Some(container) = (unsafe { close.superview().and_then(|v| v.superview()) }) else { return };
    let mut frame = container.frame();
    frame.size.height = HEADER;
    frame.origin.y = ns.frame().size.height - HEADER;
    container.setFrame(frame);
    let gap = mini.frame().origin.x - close.frame().origin.x;
    for (i, b) in buttons.iter().enumerate() {
        let y = (HEADER - b.frame().size.height) / 2.0;
        b.setFrameOrigin(NSPoint::new(18.0 + gap * i as f64, y));
    }
}
