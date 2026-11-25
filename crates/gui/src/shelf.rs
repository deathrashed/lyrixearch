//! macOS drop shelf: while files are dragged near the menu bar icon, a small panel opens under it
//! with two drop zones, "Add" and "Add and fetch". It never takes focus away from the app the drag
//! started in.
//!
//! AppKit gives no drag events for a status item, so a global monitor watches mouse drags (that needs
//! no Accessibility permission, only key events do). When the pointer comes near the icon and the
//! drag carries files, the panel is shown. It hides again once the mouse button is up.

use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use block2::RcBlock;
use objc2_app_kit::{NSEvent, NSEventMask, NSEventType, NSPasteboard, NSPasteboardNameDrag};
use objc2_foundation::NSString;
use tauri::{AppHandle, DragDropEvent, Emitter, LogicalPosition, Manager, WebviewEvent, WebviewUrl, WebviewWindowBuilder};

const W: f64 = 300.0;
const H: f64 = 132.0;
/// How close (in points) the pointer must come to the icon for the shelf to open.
const NEAR_X: f64 = 140.0;
const NEAR_Y: f64 = 70.0;

#[derive(Default)]
struct Shared {
    shown: AtomicBool,
    /// The drag is over the shelf itself (from the window's drag events), so it must stay open.
    over: AtomicBool,
    /// Last time the drag was near the icon or over the shelf, as ms since `start`.
    last_near: AtomicU64,
    /// Icon centre x, icon bottom, primary screen height (points). Refreshed by the watcher thread,
    /// because asking the tray from the main thread's event handler would wait on itself.
    anchor: Mutex<Option<(f64, f64, f64)>>,
    /// The drag pasteboard's change count when the mouse button went down. The pasteboard keeps the
    /// last drag's contents, so only a count that moved since then means files are being dragged now
    /// (and not, say, a window being moved).
    down_count: AtomicI64,
}

pub fn install(app: &AppHandle) -> tauri::Result<()> {
    let shelf = WebviewWindowBuilder::new(app, "shelf", WebviewUrl::App("shelf.html".into()))
        .title("Lyrixearch drop zone")
        .inner_size(W, H)
        .decorations(false)
        .transparent(true)
        .shadow(true)
        .resizable(false)
        .always_on_top(true)
        .visible_on_all_workspaces(true)
        .skip_taskbar(true)
        .focused(false)
        .visible(false)
        .build()?;

    let shared = Arc::new(Shared::default());
    let start = Instant::now();
    let now_ms = move || start.elapsed().as_millis() as u64;

    // drops on the shelf: the left half adds, the right half adds and fetches
    {
        let app = app.clone();
        let shared = shared.clone();
        shelf.on_webview_event(move |ev| {
            let WebviewEvent::DragDrop(dd) = ev else { return };
            if std::env::var_os("LYRIXEARCH_SHELF_DEBUG").is_some() {
                eprintln!("shelf drag: {dd:?}");
            }
            // macOS reports the drop position in points, although the type says physical pixels
            let half = |x: f64| if x < W / 2.0 { "add" } else { "fetch" };
            match dd {
                DragDropEvent::Enter { position, .. } | DragDropEvent::Over { position } => {
                    shared.over.store(true, Ordering::Relaxed);
                    shared.last_near.store(now_ms(), Ordering::Relaxed);
                    let _ = app.emit_to("shelf", "shelf-hover", half(position.x));
                }
                DragDropEvent::Leave => {
                    shared.over.store(false, Ordering::Relaxed);
                    let _ = app.emit_to("shelf", "shelf-hover", "");
                }
                DragDropEvent::Drop { paths, position } => {
                    shared.over.store(false, Ordering::Relaxed);
                    let fetch = half(position.x) == "fetch";
                    let paths: Vec<String> = paths.iter().map(|p| p.to_string_lossy().into_owned()).collect();
                    let _ = app.emit_to("shelf", "shelf-dropped", fetch);
                    // with "Add" the person wants to look at the list; "Add and fetch" works in the background
                    if !fetch {
                        crate::background::show_main(&app);
                    }
                    let _ = app.emit_to("ui", "shelf-drop", serde_json::json!({ "paths": paths, "fetch": fetch }));
                    let app2 = app.clone();
                    let shared = shared.clone();
                    std::thread::spawn(move || {
                        std::thread::sleep(Duration::from_millis(900));
                        hide(&app2, &shared);
                    });
                }
                _ => {}
            }
        });
    }

    // global drag watcher (runs on the main thread)
    {
        let app = app.clone();
        let shared = shared.clone();
        let handler = RcBlock::new(move |event: NonNull<NSEvent>| {
            let event = unsafe { event.as_ref() };
            if event.r#type() == NSEventType::LeftMouseDown {
                shared.down_count.store(drag_pasteboard().changeCount() as i64, Ordering::Relaxed);
                return;
            }
            if event.r#type() != NSEventType::LeftMouseDragged {
                return;
            }
            let enabled = app.state::<crate::AppState>().0.lock().unwrap().cfg.drop_shelf;
            if !enabled {
                return;
            }
            let Some((icon_x, icon_bottom, screen_h)) = *shared.anchor.lock().unwrap() else { return };
            let p = NSEvent::mouseLocation();
            let from_top = screen_h - p.y;
            let near = (p.x - icon_x).abs() < NEAR_X && from_top < NEAR_Y;
            if !near {
                return;
            }
            shared.last_near.store(now_ms(), Ordering::Relaxed);
            if shared.shown.load(Ordering::Relaxed) || !dragging_files(shared.down_count.load(Ordering::Relaxed)) {
                return;
            }
            if let Some(w) = app.get_webview_window("shelf") {
                let x = (icon_x - W / 2.0).max(8.0);
                let _ = w.set_position(LogicalPosition::new(x, icon_bottom + 6.0));
                let _ = app.emit_to("shelf", "shelf-hover", "");
                show_without_focus(&w);
                shared.shown.store(true, Ordering::Relaxed);
            }
        });
        let monitor = NSEvent::addGlobalMonitorForEventsMatchingMask_handler(NSEventMask::LeftMouseDown | NSEventMask::LeftMouseDragged, &handler);
        // the monitor lives as long as the app
        std::mem::forget(monitor);
        std::mem::forget(handler);
    }

    // close the shelf when the drag is over, or has moved away from the icon
    {
        let app = app.clone();
        std::thread::spawn(move || {
            let mut ticks = 0u32;
            loop {
                std::thread::sleep(Duration::from_millis(250));
                // the icon can move (other menu bar items come and go): look again every two seconds
                if ticks.is_multiple_of(8) {
                    *shared.anchor.lock().unwrap() = icon_anchor(&app);
                }
                ticks = ticks.wrapping_add(1);
                if !shared.shown.load(Ordering::Relaxed) {
                    continue;
                }
                let released = NSEvent::pressedMouseButtons() & 1 == 0;
                let away = now_ms().saturating_sub(shared.last_near.load(Ordering::Relaxed)) > 1500;
                if (released || away) && !shared.over.load(Ordering::Relaxed) {
                    // leave a moment for a drop that is still being delivered
                    std::thread::sleep(Duration::from_millis(300));
                    if !shared.over.load(Ordering::Relaxed) {
                        hide(&app, &shared);
                    }
                }
            }
        });
    }
    Ok(())
}

fn hide(app: &AppHandle, shared: &Shared) {
    if let Some(w) = app.get_webview_window("shelf") {
        let _ = w.hide();
    }
    shared.shown.store(false, Ordering::Relaxed);
}

/// Centre x and bottom of the menu bar icon, and the main screen height, in points with x from the
/// left and y from the top (the tray reports physical pixels).
fn icon_anchor(app: &AppHandle) -> Option<(f64, f64, f64)> {
    let tray = app.tray_by_id("main")?;
    let rect = tray.rect().ok()??;
    // the menu bar's screen is the primary one: AppKit's mouse coordinates start at its bottom-left
    let monitor = app.primary_monitor().ok()??;
    let scale = monitor.scale_factor();
    let screen_h = monitor.size().to_logical::<f64>(scale).height;
    let pos = rect.position.to_logical::<f64>(scale);
    let size = rect.size.to_logical::<f64>(scale);
    Some((pos.x + size.width / 2.0, pos.y + size.height, screen_h))
}

fn drag_pasteboard() -> objc2::rc::Retained<NSPasteboard> {
    NSPasteboard::pasteboardWithName(unsafe { NSPasteboardNameDrag })
}

/// Is a drag that started with this mouse press carrying files (or folders)?
fn dragging_files(down_count: i64) -> bool {
    let pb = drag_pasteboard();
    if pb.changeCount() as i64 == down_count {
        return false;
    }
    let Some(types) = pb.types() else { return false };
    let file_url = NSString::from_str("public.file-url");
    types.iter().any(|t| t.isEqualToString(&file_url))
}

/// Order the panel front without activating Lyrixearch, so the drag's source app keeps focus.
fn show_without_focus(w: &tauri::WebviewWindow) {
    match w.ns_window() {
        Ok(ptr) if !ptr.is_null() => {
            let win: &objc2_app_kit::NSWindow = unsafe { &*(ptr as *const objc2_app_kit::NSWindow) };
            win.orderFrontRegardless();
        }
        _ => {
            let _ = w.show();
        }
    }
}
