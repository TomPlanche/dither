//! The two pipeline stages, called straight from the browser.
//!
//! There is no HTTP in here and no `wasm-bindgen` boundary either: the front
//! end is Rust, so it links [`dither_core`] into the same WebAssembly module
//! and calls it the way the CLI does. What crosses into JavaScript is the
//! finished pixels, once, on their way to a `<canvas>`.
//!
//! The split into [`fit`] and [`shade`] is what keeps the sliders responsive.
//! `fit` resamples the whole photo and costs real time on a large upload;
//! `shade` works on the result, which is 600x400 by default, and is quick
//! enough to re-run on every input event.

use dither_core::{IndexedImage, RgbImage, apply_dithering, io, resize};

use crate::settings::{Geometry, Look, Resize};

/// A finished render, ready for both the canvas and the download button.
pub struct Frame {
    /// Straight-to-`ImageData` pixels, four bytes each and fully opaque.
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// The same image in the form it should be saved as.
    pub output: Output,
}

/// What a download would write.
///
/// Dithered output keeps its palette, because a 6-colour PNG is a fraction of
/// the size of the same picture in truecolour and this is the one place a
/// browser cannot help: `canvas.toBlob` only ever writes RGBA.
pub enum Output {
    Indexed(IndexedImage),
    Rgb(RgbImage),
}

impl Output {
    /// Encodes the PNG the download hands to the browser.
    pub fn encode_png(&self) -> Result<Vec<u8>, io::IoError> {
        match self {
            Output::Indexed(image) => io::encode_indexed_png(image),
            Output::Rgb(image) => io::encode_rgb_png(image),
        }
    }

    /// The suffix that says which of the two this is, for the file name.
    pub fn suffix(&self) -> &'static str {
        match self {
            Output::Indexed(_) => "dithered",
            Output::Rgb(_) => "resized",
        }
    }
}

/// Decodes an upload. JPEG and PNG, the two the pipeline is built with.
pub fn decode(bytes: &[u8]) -> Result<RgbImage, io::IoError> {
    io::decode_rgb(bytes)
}

/// Stage one: cut the photo down to the working size.
///
/// Which of the pipeline's resize functions does the work is what `resize`
/// picks. They all take the same target, but only [`Resize::Fit`] reads it as a
/// size: for the other two it is a shape the crop is measured against, and the
/// scaling either comes from the fraction or does not happen at all.
pub fn fit(source: &RgbImage, geometry: Geometry) -> RgbImage {
    let target = geometry.target();
    let fit = geometry.fit();

    match geometry.resize {
        Resize::Fit => resize::resize_to_fit(source, target, fit),
        Resize::Factor(factor) => resize::scale_to_fit(source, target, fit, factor),
        // Keeping the source pixels is no reason to stop framing them.
        Resize::Keep if fit.crop => resize::crop_to_fit(source, target, fit),
        // The one path that dithers at the source resolution, which on a large
        // photo is the slowest thing this app can be asked to do.
        Resize::Keep => source.clone(),
    }
}

/// Stage two: dither the working image, then blow it back up.
///
/// The scale is applied after the dither and by nearest neighbour, so the
/// pattern is enlarged rather than resampled and stays as crisp as it was at
/// the working size.
pub fn shade(sized: &RgbImage, look: Look, scale: u32) -> Frame {
    let output = match look.options() {
        Some(options) => {
            let dithered = apply_dithering(sized, &options);
            Output::Indexed(if scale > 1 {
                dithered.scale_nearest(scale)
            } else {
                dithered
            })
        },
        // `method=none` asks for the framing alone, so the palette settings sit
        // this one out and the photo comes back in full colour.
        None => Output::Rgb(if scale > 1 {
            resize::scale_nearest(sized, scale)
        } else {
            sized.clone()
        }),
    };

    let rgb = match &output {
        Output::Indexed(image) => image.to_rgb(),
        Output::Rgb(image) => image.clone(),
    };

    Frame {
        width: rgb.width(),
        height: rgb.height(),
        rgba: to_rgba(&rgb),
        output,
    }
}

/// Widens RGB to the RGBA a canvas `ImageData` insists on.
fn to_rgba(image: &RgbImage) -> Vec<u8> {
    let source = image.as_raw();
    let mut rgba = Vec::with_capacity(source.len() / 3 * 4);
    for pixel in source.as_chunks::<3>().0 {
        rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], u8::MAX]);
    }
    rgba
}
