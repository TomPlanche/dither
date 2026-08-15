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
///
/// `into_rgb8` rather than `to_rgb8`: a JPEG already decodes to RGB, so the borrowing form would copy the whole buffer
/// again for nothing. On a 45 MP photo that is 136 MB of pointless memcpy.
pub fn load_rgb(path: impl AsRef<Path>) -> Result<RgbImage, IoError> {
    Ok(image::open(path)?.into_rgb8())
}

/// Decodes an encoded image held in memory into an RGB buffer.
///
/// The format is sniffed from the bytes, so an upload does not have to be
/// trusted to name it correctly.
pub fn decode_rgb(bytes: &[u8]) -> Result<RgbImage, IoError> {
    Ok(image::load_from_memory(bytes)?.into_rgb8())
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
///
/// Two settings are deliberately not the crate defaults, because a dithered palette image is not the kind of data they
/// assume.
///
/// PNG's row filters predict a pixel from its neighbours, which pays off on smooth photographic bytes. These bytes are
/// palette slots: the difference between slot 6 and slot 1 means nothing, and filtering them scatters the byte
/// histogram deflate is trying to exploit. Turning filtering off makes the files about a third smaller here.
///
/// Deflate then runs at level 3 rather than the default 6. Levels above 3 spend four times the encode budget on this
/// data for under two percent of size, once filtering is out of the way.
pub fn write_indexed_png<W: Write>(image: &IndexedImage, writer: W) -> Result<(), IoError> {
    let mut encoder = png::Encoder::new(writer, image.width(), image.height());
    encoder.set_color(png::ColorType::Indexed);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_palette(image.palette().plte());
    encoder.set_filter(png::Filter::NoFilter);
    encoder.set_deflate_compression(png::DeflateCompression::Level(3));
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
