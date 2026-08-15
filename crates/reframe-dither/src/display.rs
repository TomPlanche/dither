//! Packing dithered output into the e-paper panel's frame buffer.
//!
//! The panel takes one 4-bit colour code per pixel, two pixels per byte, in
//! portrait orientation. The palette slots and the hardware codes almost agree,
//! so the conversion is a small table plus a nibble pack.

use image::RgbImage;

use crate::buffer::IndexedImage;
use crate::dither::{DitherOptions, apply_dithering};

/// Landscape size the pipeline dithers at.
pub const DISPLAY_IMAGE_SIZE: (u32, u32) = (600, 400);

/// Portrait size the panel expects.
pub const DISPLAY_PANEL_SIZE: (u32, u32) = (400, 600);

/// Hardware colour codes: 0 black, 1 white, 2 yellow, 3 red, 5 blue, 6 green.
///
/// Code 4 is the panel's clear / duplicate-black code and must never be emitted.
pub const HARDWARE_COLORS: [(u8, [u8; 3]); 6] = [
    (0, [0, 0, 0]),
    (1, [255, 255, 255]),
    (2, [255, 255, 0]),
    (3, [255, 0, 0]),
    (5, [0, 0, 255]),
    (6, [0, 255, 0]),
];

/// Maps palette slots to hardware colour codes.
///
/// Slot 4 is the duplicate black, so it folds onto code 0 along with slot 0.
pub const SLOT_TO_HARDWARE: [u8; 256] = {
    let mut table = [0u8; 256];
    table[1] = 1;
    table[2] = 2;
    table[3] = 3;
    table[5] = 5;
    table[6] = 6;
    table
};

/// Orientation of an image relative to the panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    /// Already portrait, sent through unchanged.
    Panel,
    /// Landscape, rotated a quarter turn counter-clockwise.
    Rotated,
    /// Neither expected size. Rotated anyway.
    Unexpected,
}

/// Dithers an image and packs it straight into a panel frame buffer.
///
/// Returns the buffer, the palette image (to save alongside as a PNG) and how
/// the input had to be oriented.
pub fn dither_to_display_buffer(image: &RgbImage, options: &DitherOptions) -> (Vec<u8>, IndexedImage, Orientation) {
    let dithered = apply_dithering(image, options);

    let orientation = match dithered.size() {
        DISPLAY_IMAGE_SIZE => Orientation::Rotated,
        DISPLAY_PANEL_SIZE => Orientation::Panel,
        _ => Orientation::Unexpected,
    };

    let display = match orientation {
        Orientation::Panel => dithered.clone(),
        _ => dithered.rotate90_ccw(),
    };

    (
        pack_indices(display.indices(), &SLOT_TO_HARDWARE),
        dithered,
        orientation,
    )
}

/// Maps palette slots through `table` and packs them two per byte.
///
/// A trailing pixel in an odd-length buffer is dropped.
pub fn pack_indices(indices: &[u8], table: &[u8; 256]) -> Vec<u8> {
    indices
        .chunks_exact(2)
        .map(|pair| (table[pair[0] as usize] << 4) | table[pair[1] as usize])
        .collect()
}

/// Nearest hardware colour code for an arbitrary RGB value.
///
/// Code 4 is excluded from the search, so it can never be produced.
pub fn nearest_hardware_code(rgb: [u8; 3]) -> u8 {
    HARDWARE_COLORS
        .iter()
        .min_by_key(|(_, color)| {
            (0..3)
                .map(|c| {
                    let d = rgb[c] as i32 - color[c] as i32;
                    d * d
                })
                .sum::<i32>()
        })
        .map(|(code, _)| *code)
        .unwrap_or(0)
}

/// Packs an already-dithered palette image, snapping each slot to its nearest
/// hardware colour first.
///
/// Returns `None` when the image is neither the panel size nor its transpose.
pub fn img2buffer(image: &IndexedImage) -> Option<Vec<u8>> {
    let oriented = match image.size() {
        DISPLAY_PANEL_SIZE => image.clone(),
        DISPLAY_IMAGE_SIZE => image.rotate90_ccw(),
        _ => return None,
    };

    let mut table = [0u8; 256];
    let colors = oriented.palette().colors();
    for (slot, entry) in table.iter_mut().enumerate() {
        *entry = nearest_hardware_code(colors.get(slot).copied().unwrap_or([0, 0, 0]));
    }

    Some(pack_indices(oriented.indices(), &table))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hardware_table_never_emits_the_reserved_code() {
        assert!(SLOT_TO_HARDWARE.iter().all(|&code| code != 4));
        assert_eq!(SLOT_TO_HARDWARE[4], 0);
        assert_eq!(SLOT_TO_HARDWARE[6], 6);
    }

    #[test]
    fn packing_puts_the_first_pixel_in_the_high_nibble() {
        assert_eq!(pack_indices(&[1, 6, 3, 0], &SLOT_TO_HARDWARE), vec![0x16, 0x30]);
    }

    #[test]
    fn packing_drops_an_unpaired_trailing_pixel() {
        assert_eq!(pack_indices(&[1, 6, 3], &SLOT_TO_HARDWARE), vec![0x16]);
    }

    #[test]
    fn nearest_hardware_code_avoids_the_reserved_slot() {
        assert_eq!(nearest_hardware_code([250, 250, 250]), 1);
        assert_eq!(nearest_hardware_code([10, 10, 200]), 5);
        assert_eq!(nearest_hardware_code([5, 5, 5]), 0);
    }

    #[test]
    fn landscape_input_is_rotated_to_panel_size() {
        let image = RgbImage::new(DISPLAY_IMAGE_SIZE.0, DISPLAY_IMAGE_SIZE.1);
        let (buffer, dithered, orientation) = dither_to_display_buffer(&image, &DitherOptions::default());
        assert_eq!(orientation, Orientation::Rotated);
        assert_eq!(dithered.size(), DISPLAY_IMAGE_SIZE);
        assert_eq!(buffer.len(), 400 * 600 / 2);
    }

    #[test]
    fn img2buffer_rejects_unexpected_sizes() {
        let image = apply_dithering(&RgbImage::new(10, 10), &DitherOptions::default());
        assert!(img2buffer(&image).is_none());
    }
}
