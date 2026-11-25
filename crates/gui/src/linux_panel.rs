//! Linux: show the web panel inside the window.
//!
//! Tauri packs every child webview of a window into one vertical `GtkBox`, which stacks them and
//! ignores their position and size, so the page ended up under the app instead of in the Web tab.
//! Here the app's own webview and the page are moved into a `GtkOverlay` instead: the app fills it,
//! and the page floats above at the Web tab's place. The overlay asks where each floating child goes
//! on every layout pass (its `get-child-position` signal); the answer is the exact rectangle the UI
//! last reported, so the page keeps its size and place through window resizes. The page's widget
//! only covers its own rectangle, so clicks elsewhere still reach the app.

use gtk::prelude::*;
use std::sync::Mutex;

/// Where the page goes, in logical pixels from the top-left of the app's webview.
#[derive(Clone, Copy, Debug)]
pub struct Place {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// The last place the UI asked for; the overlay reads it on every layout pass.
static WANTED: Mutex<Option<Place>> = Mutex::new(None);

/// Put the page webview (`browser`) at `place`, moving both webviews into the overlay the first time.
pub fn place(browser: &tauri::Webview, place: Place) {
    *WANTED.lock().unwrap() = Some(place);
    let _ = browser.with_webview(move |pw| {
        let page: gtk::Widget = pw.inner().upcast();
        match ensure_overlay(&page) {
            Ok(overlay) => overlay.queue_resize(),
            Err(e) => eprintln!("lyrixearch: web panel: {e}"),
        }
        if std::env::var_os("LYRIXEARCH_BROWSER_DEBUG").is_some() || std::env::var_os("LYRIXEARCH_SELFTEST").is_some() {
            // where GTK actually put it, once the layout has run
            gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(400), move || {
                let at = |w: &gtk::Widget| w.toplevel().and_then(|top| w.translate_coordinates(&top, 0, 0)).unwrap_or((0, 0));
                let (x, y) = at(&page);
                let base = page.parent().map(|o| at(&o)).unwrap_or((0, 0));
                eprintln!(
                    "linux web panel: wanted ({:.0}, {:.0}) {:.0}x{:.0}, at ({}, {}) {}x{} from the app's top-left, visible={}",
                    place.x,
                    place.y,
                    place.w,
                    place.h,
                    x - base.0,
                    y - base.1,
                    page.allocated_width(),
                    page.allocated_height(),
                    page.is_visible()
                );
            });
        }
    });
}

/// The page's parent is the window's box on first use; afterwards it is our overlay.
fn ensure_overlay(page: &gtk::Widget) -> Result<gtk::Overlay, String> {
    let parent = page.parent().ok_or("the page has no parent widget")?;
    if let Ok(overlay) = parent.clone().downcast::<gtk::Overlay>() {
        return Ok(overlay);
    }
    let vbox = parent.downcast::<gtk::Box>().map_err(|_| "the page is not in the window's box")?;
    // the app's webview: the other WebKit view in the same box
    let app_view =
        vbox.children().into_iter().find(|c| c != page && c.type_().name() == "WebKitWebView").ok_or("the app's webview was not found next to the page")?;

    vbox.remove(page);
    vbox.remove(&app_view);
    let overlay = gtk::Overlay::new();
    overlay.add(&app_view);
    overlay.add_overlay(page);
    // the floating page's rectangle, asked for on every layout pass (also after window resizes)
    overlay.connect_get_child_position(|_, _| {
        let p = (*WANTED.lock().unwrap())?;
        Some(gtk::gdk::Rectangle::new(p.x.round().max(0.0) as i32, p.y.round().max(0.0) as i32, p.w.round().max(1.0) as i32, p.h.round().max(1.0) as i32))
    });
    vbox.pack_start(&overlay, true, true, 0);
    overlay.show();
    app_view.show();
    Ok(overlay)
}
