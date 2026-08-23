//! The fixed palette everything is reduced to.
//!
//! Six colours, each blended between a pure primary and a muted version of it.
//! The blend is what `saturation` picks, and the two nearest-colour searches
//! over the result are what the dithering stages quantise through.
//!
//! The `palette` crate is referred to as `::palette` throughout, since this
//! module shares its name.

use ::palette::color_difference::EuclideanDistance;
use ::palette::{IntoColor, Lab, Srgb};
use image::Rgb;
use image::imageops::ColorMap;

/// Number of colours in the palette.
pub const PALETTE_COLORS: usize = 6;

/// The pure primaries, fully saturated.
pub const PURE_PALETTE: [[u8; 3]; 6] = [
    [0, 0, 0],       // Black
    [255, 255, 255], // White
    [0, 255, 0],     // Green
    [0, 0, 255],     // Blue
    [255, 0, 0],     // Red
    [255, 255, 0],   // Yellow
];

/// The muted counterparts, which is what those primaries look like on ink.
pub const MUTED_PALETTE: [[u8; 3]; 6] = [
    [57, 48, 57],    // Muted Black
    [255, 255, 255], // White
    [40, 91, 58],    // Muted Green
    [0, 128, 255],   // Muted Blue
    [156, 72, 75],   // Muted Red
    [208, 190, 71],  // Muted Yellow
];

/// Maps each output palette slot to its source primary.
///
/// The slots are ordered black, white, yellow, red, blue, green rather than
/// following the primaries' own order, so slot 0 and slot 1 are the two
/// extremes a nearest-colour search falls back on.
pub const PALETTE_ORDER: [usize; PALETTE_COLORS] = [0, 1, 5, 4, 3, 2];

/// Blends the pure and the muted palettes.
///
/// At `0.0` the result is [`PURE_PALETTE`], at `1.0` it is [`MUTED_PALETTE`].
pub fn palette_blend(saturation: f64) -> [[u8; 3]; PALETTE_COLORS] {
    let mut out = [[0u8; 3]; PALETTE_COLORS];
    for (slot, &src) in PALETTE_ORDER.iter().enumerate() {
        for channel in 0..3 {
            let muted = MUTED_PALETTE[src][channel] as f64 * saturation;
            let pure = PURE_PALETTE[src][channel] as f64 * (1.0 - saturation);
            out[slot][channel] = (muted + pure) as u8;
        }
    }
    out
}

/// Converts an 8-bit sRGB triple to CIELAB.
pub fn to_lab(rgb: [u8; 3]) -> Lab {
    let srgb: Srgb<f32> = Srgb::new(rgb[0], rgb[1], rgb[2]).into_format();
    srgb.into_color()
}

/// The blended palette, with both nearest-colour searches over it.
#[derive(Debug, Clone)]
pub struct Palette {
    colors: [[u8; 3]; PALETTE_COLORS],
    lab: [Lab; PALETTE_COLORS],
}

impl Palette {
    /// Builds the palette for a given saturation.
    pub fn new(saturation: f64) -> Self {
        let colors = palette_blend(saturation);
        let mut lab = [Lab::default(); PALETTE_COLORS];
        for (slot, color) in colors.iter().enumerate() {
            lab[slot] = to_lab(*color);
        }

        Self { colors, lab }
    }

    /// The blended colours, one per slot.
    pub fn colors(&self) -> &[[u8; 3]; PALETTE_COLORS] {
        &self.colors
    }

    /// The slot closest to an arbitrary colour, by squared RGB distance.
    ///
    /// Error diffusion wants this rather than CIELAB. CIELAB weights lightness
    /// heavily, so on a palette this small it pulls midtones toward white and
    /// black and visibly drains the colour out of the result. Diffusing the
    /// error afterwards is what recovers the intermediate tones, and that works
    /// best when the match minimises the error the diffusion has to carry.
    pub fn nearest_rgb(&self, rgb: [u8; 3]) -> usize {
        let mut best = 0;
        let mut best_dist = i32::MAX;
        for (slot, entry) in self.colors.iter().enumerate() {
            let dist: i32 = (0..3)
                .map(|c| {
                    let d = rgb[c] as i32 - entry[c] as i32;
                    d * d
                })
                .sum();
            if dist < best_dist {
                best_dist = dist;
                best = slot;
            }
        }
        best
    }

    /// The slot closest to an arbitrary colour, by CIELAB distance.
    ///
    /// Used by the ordered method, where there is no error term to fall back on
    /// and perceptual matching matters more. It also keeps neutral greys from
    /// matching the palette's green, which sits at a similar RGB luminance.
    pub fn nearest(&self, rgb: [u8; 3]) -> usize {
        self.nearest_lab(to_lab(rgb))
    }

    /// The slot closest to an already-converted colour. Ties go to the lower slot.
    pub fn nearest_lab(&self, color: Lab) -> usize {
        let mut best = 0;
        let mut best_dist = f32::INFINITY;
        for (slot, entry) in self.lab.iter().enumerate() {
            let dist = color.distance_squared(*entry);
            if dist < best_dist {
                best_dist = dist;
                best = slot;
            }
        }
        best
    }

    /// The palette flattened for a PNG `PLTE` chunk, padded to 256 entries.
    pub fn plte(&self) -> Vec<u8> {
        let mut flat: Vec<u8> = self.colors.iter().flatten().copied().collect();
        flat.resize(256 * 3, 0);
        flat
    }
}

/// Lets the palette plug into `image`'s own quantisation helpers, such as
/// `imageops::index_colors`. Matching uses the RGB metric, which is what error
/// diffusion wants.
impl ColorMap for Palette {
    type Color = Rgb<u8>;

    fn index_of(&self, color: &Rgb<u8>) -> usize {
        self.nearest_rgb(color.0)
    }

    fn lookup(&self, index: usize) -> Option<Rgb<u8>> {
        self.colors.get(index).map(|c| Rgb(*c))
    }

    fn has_lookup(&self) -> bool {
        true
    }

    fn map_color(&self, color: &mut Rgb<u8>) {
        *color = Rgb(self.colors[self.index_of(color)]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_blend_lands_between_the_two_palettes() {
        assert_eq!(
            palette_blend(0.6),
            [
                [34, 28, 34],
                [255, 255, 255],
                [226, 216, 42],
                [195, 43, 45],
                [0, 76, 255],
                [24, 156, 34],
            ]
        );
    }

    #[test]
    fn the_ends_of_the_blend_are_the_palettes_themselves() {
        for (slot, &src) in PALETTE_ORDER.iter().enumerate() {
            assert_eq!(palette_blend(0.0)[slot], PURE_PALETTE[src]);
            assert_eq!(palette_blend(1.0)[slot], MUTED_PALETTE[src]);
        }
    }

    #[test]
    fn every_slot_is_a_distinct_colour() {
        let colors = palette_blend(0.6);
        for (slot, color) in colors.iter().enumerate() {
            assert!(
                !colors[..slot].contains(color),
                "slot {slot} repeats an earlier colour, so it can never be picked"
            );
        }
    }

    #[test]
    fn primaries_map_to_their_own_slots() {
        let palette = Palette::new(0.6);
        assert_eq!(palette.nearest([0, 0, 0]), 0);
        assert_eq!(palette.nearest([255, 255, 255]), 1);
        assert_eq!(palette.nearest([255, 255, 0]), 2);
        assert_eq!(palette.nearest([255, 0, 0]), 3);
        assert_eq!(palette.nearest([0, 0, 255]), 4);
        assert_eq!(palette.nearest([0, 255, 0]), 5);
    }

    #[test]
    fn plte_is_padded_to_256_entries() {
        assert_eq!(Palette::new(0.6).plte().len(), 768);
    }
}
