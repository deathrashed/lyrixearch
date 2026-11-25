//! Linux: show the web panel inside the window.
//!
//! Tauri packs every child webview of a window into one vertical `GtkBox`, which stacks them and
//! ignores their position and size, so the page ended up under the app instead of in the Web tab.
//! Here the app's own webview and the page are moved into a `GtkOverlay` instead: the app fills it,
//! and the page floats above at the Web tab's place (alignment plus margins). The page's widget only
//! covers its own rectangle, so clicks elsewhere still reach the app.

use gtk::prelude::*;

/// Where the page goes, in logical pixels from the top-left of the app's webview.
#[derive(Clone, Copy, Debug)]
pub struct Place {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// Put the page webview (`browser`) at `place`, moving both webviews into the overlay the first time.
pub fn place(browser: &tauri::Webview, place: Place) {
    let _ = browser.with_webview(move |pw| {
        let page: gtk::Widget = pw.inner().upcast();
        if let Err(e) = ensure_overlay(&page) {
            eprintln!("lyrixearch: web panel: {e}");
            return;
        }
        page.set_halign(gtk::Align::Start);
        page.set_valign(gtk::Align::Start);
        page.set_margin_start(place.x.round().max(0.0) as i32);
        page.set_margin_top(place.y.round().max(0.0) as i32);
        page.set_size_request(place.w.round().max(1.0) as i32, place.h.round().max(1.0) as i32);
        page.queue_resize();
    });
}

/// The page's parent is the window's box on first use; afterwards it is our overlay.
fn ensure_overlay(page: &gtk::Widget) -> Result<(), String> {
    let parent = page.parent().ok_or("the page has no parent widget")?;
    if parent.is::<gtk::Overlay>() {
        return Ok(());
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
    overlay.show();
    app_view.show();
    Ok(())
}
