//! The palette image produced by dithering.
//!
//! Pixels are [`image::GrayImage`] values reinterpreted as palette slots, which
//! is the representation `image::imageops::index_colors` returns.

use image::imageops::{self, FilterType};
use image::{GrayImage, RgbImage};

use crate::panel::PanelPalette;

/// An indexed image: one palette slot per pixel, plus the palette itself.
#[derive(Debug, Clone)]
pub struct IndexedImage {
    indices: GrayImage,
    palette: PanelPalette,
}

impl IndexedImage {
    pub fn new(indices: GrayImage, palette: PanelPalette) -> Self {
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

    pub fn palette(&self) -> &PanelPalette {
        &self.palette
    }

    /// Expands the slots back into a full RGB image.
    pub fn to_rgb(&self) -> RgbImage {
        let colors = self.palette.colors();
        RgbImage::from_fn(self.width(), self.height(), |x, y| {
            image::Rgb(colors[self.indices.get_pixel(x, y).0[0] as usize])
        })
    }

    /// Rotates a quarter turn counter-clockwise, matching PIL's
    /// `Image.rotate(90, expand=True)`.
    pub fn rotate90_ccw(&self) -> IndexedImage {
        IndexedImage {
            indices: imageops::rotate270(&self.indices),
            palette: self.palette.clone(),
        }
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
    fn rotate90_ccw_matches_pil() {
        // PIL's rotate(90, expand=True) on [[0,1,2],[3,4,5]] gives [[2,5],[1,4],[0,3]].
        let indices = GrayImage::from_raw(3, 2, vec![0, 1, 2, 3, 4, 5]).unwrap();
        let rotated = IndexedImage::new(indices, PanelPalette::new(0.6)).rotate90_ccw();
        assert_eq!(rotated.size(), (2, 3));
        assert_eq!(rotated.indices(), &[2, 5, 1, 4, 0, 3]);
    }

    #[test]
    fn scale_nearest_replicates_pixels() {
        let indices = GrayImage::from_raw(2, 1, vec![1, 6]).unwrap();
        let scaled = IndexedImage::new(indices, PanelPalette::new(0.6)).scale_nearest(2);
        assert_eq!(scaled.size(), (4, 2));
        assert_eq!(scaled.indices(), &[1, 1, 6, 6, 1, 1, 6, 6]);
    }
}
