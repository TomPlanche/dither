//! The few places this app has to talk to the browser itself.
//!
//! Everything else is plain Rust. What is left here is the three things the
//! platform owns: reading the bytes out of a picked file, pushing finished
//! pixels onto a `<canvas>`, and handing a PNG to the download machinery.

use dither_core::io;
use js_sys::{Array, Uint8Array};
use wasm_bindgen::{Clamped, JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    Blob, BlobPropertyBag, CanvasRenderingContext2d, File, HtmlAnchorElement, HtmlCanvasElement, ImageData, Response,
    Url,
};

use crate::pipeline::Frame;

/// Reads a picked or dropped file into memory.
pub async fn read_file(file: &File) -> Result<Vec<u8>, JsValue> {
    let buffer = JsFuture::from(file.array_buffer()).await?;
    Ok(Uint8Array::new(&buffer).to_vec())
}

/// Resizes the canvas to the frame and paints it.
///
/// `put_image_data` writes the pixels as they are, with no smoothing and no
/// colour management in the way, which is what a dither pattern needs: the
/// canvas is sized to the real output and CSS scales it down to fit.
pub fn draw(canvas: &HtmlCanvasElement, frame: &Frame) -> Result<(), JsValue> {
    canvas.set_width(frame.width);
    canvas.set_height(frame.height);

    let context = canvas
        .get_context("2d")?
        .ok_or_else(|| JsValue::from_str("this browser gave no 2d canvas context"))?
        .dyn_into::<CanvasRenderingContext2d>()?;

    let pixels = ImageData::new_with_u8_clamped_array_and_sh(Clamped(&frame.rgba), frame.width, frame.height)?;
    context.put_image_data(&pixels, 0.0, 0.0)
}

/// Saves `bytes` as `name`, through an object URL and a synthetic click.
pub fn download(bytes: &[u8], name: &str) -> Result<(), JsValue> {
    let options = BlobPropertyBag::new();
    options.set_type("image/png");
    let blob = Blob::new_with_u8_array_sequence_and_options(&Array::of1(&Uint8Array::from(bytes)), &options)?;

    let url = Url::create_object_url_with_blob(&blob)?;
    let document = web_sys::window()
        .and_then(|window| window.document())
        .ok_or_else(|| JsValue::from_str("no document to hang the download off"))?;

    let anchor = document.create_element("a")?.dyn_into::<HtmlAnchorElement>()?;
    anchor.set_href(&url);
    anchor.set_download(name);
    anchor.click();

    // The blob is held alive by the click that already happened, so the URL has
    // done its job and would otherwise leak for the life of the document.
    Url::revoke_object_url(&url)
}

/// Turns a decode failure into something worth putting on screen.
pub fn describe(error: &io::IoError) -> String {
    format!("{error}")
}

/// Fetches a file served beside the page.
///
/// Same origin, so this is the bundle reading its own directory rather than
/// anything leaving the machine.
pub async fn fetch_bytes(url: &str) -> Result<Vec<u8>, JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("no window to fetch from"))?;

    let response: Response = JsFuture::from(window.fetch_with_str(url)).await?.dyn_into()?;
    if !response.ok() {
        return Err(JsValue::from_str(&format!("{url} came back {}", response.status())));
    }

    let buffer = JsFuture::from(response.array_buffer()?).await?;
    Ok(Uint8Array::new(&buffer).to_vec())
}
