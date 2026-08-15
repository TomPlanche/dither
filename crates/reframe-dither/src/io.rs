//! Loading and saving images. Requires the `image-io` feature.
//!
//! Decoding goes through the [`image`] crate. Encoding uses [`png`] directly,
//! because `image` cannot write palette PNGs.
//!
//! Every operation comes in two flavours: a `load_`/`save_` pair that takes a
//! path, and a `decode_`/`encode_` pair that works on bytes, for callers such
//! as the HTTP server that never touch the filesystem.

use std::error::Error as StdError;
use std::fmt;
use std::fs::File;
use std::io::{BufWriter, Cursor, Write};
use std::path::Path;

use image::RgbImage;

use crate::buffer::IndexedImage;

#[derive(Debug)]
pub enum IoError {
    Decode(image::ImageError),
    Encode(png::EncodingError),
    Io(std::io::Error),
}

impl fmt::Display for IoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IoError::Decode(e) => write!(f, "could not decode image: {e}"),
            IoError::Encode(e) => write!(f, "could not encode PNG: {e}"),
            IoError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl StdError for IoError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            IoError::Decode(e) => Some(e),
            IoError::Encode(e) => Some(e),
            IoError::Io(e) => Some(e),
        }
    }
}

impl From<image::ImageError> for IoError {
    fn from(e: image::ImageError) -> Self {
        IoError::Decode(e)
    }
}

impl From<png::EncodingError> for IoError {
    fn from(e: png::EncodingError) -> Self {
        IoError::Encode(e)
    }
}

impl From<std::io::Error> for IoError {
    fn from(e: std::io::Error) -> Self {
        IoError::Io(e)
    }
}

/// Decodes any supported image file into an RGB buffer.
pub fn load_rgb(path: impl AsRef<Path>) -> Result<RgbImage, IoError> {
    Ok(image::open(path)?.to_rgb8())
}

/// Decodes an encoded image held in memory into an RGB buffer.
///
/// The format is sniffed from the bytes, so an upload does not have to be
/// trusted to name it correctly.
pub fn decode_rgb(bytes: &[u8]) -> Result<RgbImage, IoError> {
    Ok(image::load_from_memory(bytes)?.to_rgb8())
}

/// Writes a palette image as an indexed PNG, the way PIL saves a `P` mode image.
pub fn save_indexed_png(image: &IndexedImage, path: impl AsRef<Path>) -> Result<(), IoError> {
    write_indexed_png(image, BufWriter::new(File::create(path)?))
}

/// Encodes a palette image as an indexed PNG into a byte vector.
pub fn encode_indexed_png(image: &IndexedImage) -> Result<Vec<u8>, IoError> {
    let mut out = Vec::new();
    write_indexed_png(image, &mut out)?;
    Ok(out)
}

/// Encodes a palette image as an indexed PNG into any writer.
pub fn write_indexed_png<W: Write>(image: &IndexedImage, writer: W) -> Result<(), IoError> {
    let mut encoder = png::Encoder::new(writer, image.width(), image.height());
    encoder.set_color(png::ColorType::Indexed);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_palette(image.palette().plte());
    encoder.write_header()?.write_image_data(image.indices())?;
    Ok(())
}

/// Writes an RGB image as a PNG.
pub fn save_rgb_png(image: &RgbImage, path: impl AsRef<Path>) -> Result<(), IoError> {
    image
        .save_with_format(path, image::ImageFormat::Png)
        .map_err(IoError::Decode)
}

/// Encodes an RGB image as a PNG into a byte vector.
pub fn encode_rgb_png(image: &RgbImage) -> Result<Vec<u8>, IoError> {
    let mut out = Vec::new();
    image
        .write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)
        .map_err(IoError::Decode)?;
    Ok(out)
}
