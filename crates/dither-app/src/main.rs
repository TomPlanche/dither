//! Browser front end for the dithering pipeline.
//!
//! Trunk builds this crate for `wasm32-unknown-unknown` and serves `index.html`
//! beside it. There is no server in the loop: [`dither_core`] is linked into
//! the same module, so a photo picked here is decoded, resized and dithered in
//! the tab that picked it and never leaves the machine.

mod app;
mod browser;
mod pipeline;
mod settings;

fn main() {
    // Without this a panic in the wasm module is an unhelpful `unreachable`.
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(app::App);
}
