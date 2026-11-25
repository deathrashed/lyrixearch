//! Linux: show the web panel inside the window.
//!
//! Tauri packs every child webview of a window into one vertical `GtkBox`, which stacks them and
//! ignores their position and size, so the page ended up under the app instead of in the Web tab.
//! Here the app's own webview and the page are moved into a `GtkOverlay` instead: the app fills it,
//! and the page floats above at the Web tab's place. The page fills the overlay minus margins on all
//! four sides, so its size is exact (a size request is only a minimum: WebKit would grow past it),
//! and the margins are worked out again whenever the overlay changes size. The page's widget only
//! covers its own rectangle, so clicks elsewhere still reach the app.

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

/// The last place asked for, applied again when the overlay is resized.
static WANTED: Mutex<Option<Place>> = Mutex::new(None);

/// Put the page webview (`browser`) at `place`, moving both webviews into the overlay the first time.
pub fn place(browser: &tauri::Webview, place: Place) {
    *WANTED.lock().unwrap() = Some(place);
    let _ = browser.with_webview(move |pw| {
        let page: gtk::Widget = pw.inner().upcast();
        match ensure_overlay(&page) {
            Ok(overlay) => apply(&page, &overlay, place),
            Err(e) => eprintln!("lyrixearch: web panel: {e}"),
        }
        if std::env::var_os("LYRIXEARCH_BROWSER_DEBUG").is_some() || std::env::var_os("LYRIXEARCH_SELFTEST").is_some() {
            // where GTK actually put it, once the layout has run
            gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(400), move || {
                let (x, y) = page.toplevel().and_then(|top| page.translate_coordinates(&top, 0, 0)).unwrap_or((-1, -1));
                let base = page.parent().and_then(|o| o.translate_coordinates(&o.toplevel()?, 0, 0)).unwrap_or((0, 0));
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

/// Margins that leave exactly `place` for the page inside an overlay of the current size.
fn apply(page: &gtk::Widget, overlay: &gtk::Overlay, place: Place) {
    let (ow, oh) = (overlay.allocated_width() as f64, overlay.allocated_height() as f64);
    let x = place.x.round().max(0.0);
    let y = place.y.round().max(0.0);
    let w = place.w.round().max(1.0);
    let h = place.h.round().max(1.0);
    page.set_halign(gtk::Align::Fill);
    page.set_valign(gtk::Align::Fill);
    page.set_size_request(1, 1);
    page.set_margin_start(x as i32);
    page.set_margin_top(y as i32);
    // before the overlay has a size, the far margins wait for its first size_allocate
    page.set_margin_end(if ow > 1.0 { (ow - x - w).max(0.0) as i32 } else { 0 });
    page.set_margin_bottom(if oh > 1.0 { (oh - y - h).max(0.0) as i32 } else { 0 });
    // no explicit queue_resize: GTK's setters queue one only when a value changes, which is what
    // stops the size_allocate handler below from looping
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
    vbox.pack_start(&overlay, true, true, 0);
    // the window was resized: keep the page on its rectangle
    let page_for_resize = page.clone();
    overlay.connect_size_allocate(move |overlay, _| {
        if let Some(place) = *WANTED.lock().unwrap() {
            apply(&page_for_resize, overlay, place);
        }
    });
    overlay.show();
    app_view.show();
    Ok(overlay)
}
