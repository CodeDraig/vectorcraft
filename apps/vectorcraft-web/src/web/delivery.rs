//! Browser file downloads, blob URLs and PDF printing.

use super::{PrintJob, PrintService, Printer, fileio, js_err};

/// Trigger a browser download of `bytes` named after the last component of `path`.
pub(super) fn download(path: &str, bytes: &[u8]) -> Result<(), String> {
    let js = |e: wasm_bindgen::JsValue| format!("{e:?}");
    let name = std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "vectorcraft".into());
    let window = web_sys::window().ok_or("no window")?;
    let document = window.document().ok_or("no document")?;
    let url = blob_url(bytes, fileio::format_for_name(&name).map_or("application/octet-stream", |f| f.mime))?;
    let a: web_sys::HtmlAnchorElement = document.create_element("a").map_err(js)?.dyn_into().map_err(|_| "not an anchor")?;
    a.set_href(&url);
    a.set_download(&name);
    a.style().set_property("display", "none").map_err(js)?;
    let body = document.body().ok_or("no body")?;
    body.append_child(&a).map_err(js)?;
    a.click();
    a.remove();
    // Revoke after the click has been dispatched; the download keeps its own reference.
    let revoke = wasm_bindgen::closure::Closure::once_into_js(move || {
        web_sys::Url::revoke_object_url(&url).ok();
    });
    window.set_timeout_with_callback_and_timeout_and_arguments_0(revoke.unchecked_ref(), 10_000).map_err(js)?;
    Ok(())
}

/// An object URL of a blob of `bytes` with media type `mime` (revoke it when done).
fn blob_url(bytes: &[u8], mime: &str) -> Result<String, String> {
    let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(bytes));
    let opts = web_sys::BlobPropertyBag::new();
    opts.set_type(mime);
    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &opts).map_err(js_err)?;
    web_sys::Url::create_object_url_with_blob(&blob).map_err(js_err)
}

/// File → Print in the browser: the job's PDF in a hidden frame, printed through the browser's
/// print dialog (which picks the printer).
pub(super) struct BrowserPrint;

/// The frame holding the last print job (the next job replaces it).
const PRINT_FRAME_ID: &str = "vectorcraft-print-frame";

impl PrintService for BrowserPrint {
    fn printers(&mut self) -> Vec<Printer> {
        // Browsers don't tell pages about printers: their print dialog lists them.
        vec![]
    }

    fn print(&mut self, job: &PrintJob) -> Result<String, String> {
        print_pdf(job.pdf)?;
        Ok(format!("Printing “{}”: pick the printer in the browser's print dialog", job.title))
    }
}

fn print_pdf(pdf: &[u8]) -> Result<(), String> {
    let document = web_sys::window().and_then(|w| w.document()).ok_or("no document")?;
    // The previous job's frame and its blob go first.
    if let Some(old) = document.get_element_by_id(PRINT_FRAME_ID) {
        if let Some(url) = old.get_attribute("src") {
            web_sys::Url::revoke_object_url(&url).ok();
        }
        old.remove();
    }
    let url = blob_url(pdf, "application/pdf")?;
    let frame: web_sys::HtmlIFrameElement = document.create_element("iframe").map_err(js_err)?.dyn_into().map_err(|_| "not a frame")?;
    frame.set_id(PRINT_FRAME_ID);
    // Out of sight but laid out: browsers don't print a frame that isn't.
    let style = frame.style();
    for (k, v) in [("position", "fixed"), ("right", "0"), ("bottom", "0"), ("width", "0"), ("height", "0"), ("border", "0")] {
        style.set_property(k, v).map_err(js_err)?;
    }
    let loaded = frame.clone();
    let onload = wasm_bindgen::closure::Closure::once_into_js(move || {
        if let Some(w) = loaded.content_window() {
            w.focus().ok();
            if let Err(e) = w.print() {
                log::error!(target: "vectorcraft_web::web", "printing failed: {e:?}");
            }
        }
    });
    frame.set_onload(Some(onload.unchecked_ref()));
    frame.set_src(&url);
    document.body().ok_or("no body")?.append_child(&frame).map_err(js_err)?;
    Ok(())
}

use wasm_bindgen::JsCast as _;
