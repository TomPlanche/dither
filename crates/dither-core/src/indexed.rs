//! The palette image produced by dithering.
//!
//! Pixels are [`image::GrayImage`] values reinterpreted as palette slots, which
//! is the representation `image::imageops::index_colors` returns.

use image::imageops::{self, FilterType};
use image::{GrayImage, RgbImage};

use crate::palette::Palette;

/// An indexed image: one palette slot per pixel, plus the palette itself.
#[derive(Debug, Clone)]
pub struct IndexedImage {
    indices: GrayImage,
    palette: Palette,
}

impl IndexedImage {
    pub fn new(indices: GrayImage, palette: Palette) -> Self {
        Self { indices, palette }
    }

    pub fn width(&self) -> u32 {
        self.indices.width()
    }

    pub fn height(&self) -> u32 {
        self.indices.height()
    }

    /// `(width, height)`.
    pub fn size(&self) -> (u32, u32) {
        self.indices.dimensions()
    }

    /// The raw palette slots, row major.
    pub fn indices(&self) -> &[u8] {
        self.indices.as_raw()
    }

    pub fn palette(&self) -> &Palette {
        &self.palette
    }

    /// Expands the slots back into a full RGB image.
    pub fn to_rgb(&self) -> RgbImage {
        let colors = self.palette.colors();
        RgbImage::from_fn(self.width(), self.height(), |x, y| {
            image::Rgb(colors[self.indices.get_pixel(x, y).0[0] as usize])
        })
    }

    /// Nearest-neighbour integer upscale, which keeps the dither pattern crisp rather than smearing it.
    pub fn scale_nearest(&self, factor: u32) -> IndexedImage {
        IndexedImage {
            indices: imageops::resize(
                &self.indices,
                self.width() * factor,
                self.height() * factor,
                FilterType::Nearest,
            ),
            palette: self.palette.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_nearest_replicates_pixels() {
        let indices = GrayImage::from_raw(2, 1, vec![1, 6]).unwrap();
        let scaled = IndexedImage::new(indices, Palette::new(0.6)).scale_nearest(2);
        assert_eq!(scaled.size(), (4, 2));
        assert_eq!(scaled.indices(), &[1, 1, 6, 6, 1, 1, 6, 6]);
    }

    #[test]
    fn to_rgb_expands_every_slot_through_the_palette() {
        let palette = Palette::new(0.6);
        let indices = GrayImage::from_raw(3, 1, vec![0, 1, 2]).unwrap();
        let rgb = IndexedImage::new(indices, palette.clone()).to_rgb();
        for (x, slot) in [0usize, 1, 2].into_iter().enumerate() {
            assert_eq!(rgb.get_pixel(x as u32, 0).0, palette.colors()[slot]);
        }
    }
}
