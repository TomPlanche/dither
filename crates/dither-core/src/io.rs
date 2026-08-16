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
use std::io::{BufRead, BufWriter, Cursor, Seek, Write};
use std::path::Path;

use image::{DynamicImage, ImageDecoder, ImageReader, RgbImage};

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

/// Decodes any supported image file into an RGB buffer, upright.
pub fn load_rgb(path: impl AsRef<Path>) -> Result<RgbImage, IoError> {
    decode_upright(ImageReader::open(path)?.with_guessed_format()?)
}

/// Decodes an encoded image held in memory into an RGB buffer, upright.
///
/// The format is sniffed from the bytes, so an upload does not have to be
/// trusted to name it correctly.
pub fn decode_rgb(bytes: &[u8]) -> Result<RgbImage, IoError> {
    decode_upright(ImageReader::new(Cursor::new(bytes)).with_guessed_format()?)
}

/// Decodes through a reader, rotating the pixels to match the file's EXIF orientation tag.
///
/// A phone camera writes its sensor readout unrotated and records the turn it wants in EXIF, so a portrait photo
/// arrives as landscape pixels plus a tag. Nothing downstream reads that tag: an aspect ratio, a crop rectangle and
/// `keep_orientation` all work off `dimensions()`, and would read such a photo as landscape and lay a landscape preset
/// over it. Applying the turn here is what makes the rest of the crate right by construction, and it has to happen at
/// decode because the tag does not survive into [`RgbImage`].
///
/// The tag is read from the decoder before the pixels, since `orientation` needs the header and `from_decoder`
/// consumes the decoder whole. Formats that carry no such tag report `NoTransforms`, which costs nothing.
///
/// `into_rgb8` rather than `to_rgb8`: a JPEG already decodes to RGB, so the borrowing form would copy the whole buffer
/// again for nothing. On a 45 MP photo that is 136 MB of pointless memcpy.
fn decode_upright<'a, R: BufRead + Seek + 'a>(reader: ImageReader<R>) -> Result<RgbImage, IoError> {
    let mut decoder = reader.into_decoder()?;
    let orientation = decoder.orientation()?;
    let mut image = DynamicImage::from_decoder(decoder)?;
    image.apply_orientation(orientation);
    Ok(image.into_rgb8())
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A JPEG of `image`, carrying an EXIF orientation tag of `tag`.
    ///
    /// The encoder writes no EXIF of its own, so the APP1 segment is spliced in behind the two byte SOI marker, where a
    /// camera would have put it. The payload is the smallest legal one: a little endian TIFF header, then a single
    /// directory entry holding orientation as a SHORT.
    fn jpeg_with_orientation(image: &RgbImage, tag: u16) -> Vec<u8> {
        let mut tiff = Vec::from(*b"II\x2a\x00\x08\x00\x00\x00");
        tiff.extend_from_slice(&1u16.to_le_bytes()); // one directory entry
        tiff.extend_from_slice(&0x0112u16.to_le_bytes()); // Orientation
        tiff.extend_from_slice(&3u16.to_le_bytes()); // of type SHORT
        tiff.extend_from_slice(&1u32.to_le_bytes()); // one of them
        tiff.extend_from_slice(&tag.to_le_bytes());
        tiff.extend_from_slice(&[0, 0]); // the value field is four bytes wide
        tiff.extend_from_slice(&0u32.to_le_bytes()); // no directory follows

        let mut app1 = Vec::from(*b"Exif\x00\x00");
        app1.extend_from_slice(&tiff);
        let mut segment = vec![0xFF, 0xE1];
        segment.extend_from_slice(&(app1.len() as u16 + 2).to_be_bytes());
        segment.extend_from_slice(&app1);

        let mut jpeg = Vec::new();
        image
            .write_to(&mut Cursor::new(&mut jpeg), image::ImageFormat::Jpeg)
            .expect("an RGB image encodes as JPEG");
        jpeg.splice(2..2, segment);
        jpeg
    }

    #[test]
    fn a_phone_photo_is_turned_upright_before_anything_downstream_sees_it() {
        // What a phone writes for a portrait shot: landscape pixels, plus a tag asking for a quarter turn.
        let sideways = RgbImage::new(400, 300);

        let upright = decode_rgb(&jpeg_with_orientation(&sideways, 6)).expect("the JPEG decodes");
        assert_eq!(upright.dimensions(), (300, 400));

        // The other quarter turn, and the mirrored pair, land the same way round.
        for tag in [5, 7, 8] {
            let turned = decode_rgb(&jpeg_with_orientation(&sideways, tag)).expect("the JPEG decodes");
            assert_eq!(turned.dimensions(), (300, 400), "orientation {tag} was not applied");
        }
    }

    #[test]
    fn an_orientation_that_asks_for_nothing_leaves_the_photo_alone() {
        let landscape = RgbImage::new(400, 300);

        // The upright tag, the mirror-only tags, and a file with no EXIF at all.
        for tag in [1, 2, 3, 4] {
            let decoded = decode_rgb(&jpeg_with_orientation(&landscape, tag)).expect("the JPEG decodes");
            assert_eq!(
                decoded.dimensions(),
                (400, 300),
                "orientation {tag} transposed the photo"
            );
        }

        let mut bare = Vec::new();
        landscape
            .write_to(&mut Cursor::new(&mut bare), image::ImageFormat::Jpeg)
            .expect("an RGB image encodes as JPEG");
        assert_eq!(decode_rgb(&bare).expect("the JPEG decodes").dimensions(), (400, 300));
    }

    #[test]
    fn a_png_still_decodes_at_the_size_it_was_written() {
        let image = RgbImage::from_fn(64, 32, |x, y| image::Rgb([x as u8, y as u8, 0]));
        let decoded = decode_rgb(&encode_rgb_png(&image).expect("the PNG encodes")).expect("the PNG decodes");

        assert_eq!(decoded.dimensions(), (64, 32));
        assert_eq!(decoded.as_raw(), image.as_raw());
    }
}
