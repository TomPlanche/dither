//! Bayer threshold matrices for the ordered dither.

/// Supported Bayer matrix sizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BayerSize {
    Two,
    #[default]
    Four,
    Eight,
}

impl BayerSize {
    pub fn side(self) -> usize {
        match self {
            BayerSize::Two => 2,
            BayerSize::Four => 4,
            BayerSize::Eight => 8,
        }
    }

    pub fn from_side_or_default(side: u32) -> Self {
        match side {
            2 => BayerSize::Two,
            8 => BayerSize::Eight,
            _ => BayerSize::Four,
        }
    }
}

const BAYER_2: [u8; 4] = [0, 2, 3, 1];

#[rustfmt::skip]
const BAYER_4: [u8; 16] = [
     0,  8,  2, 10,
    12,  4, 14,  6,
     3, 11,  1,  9,
    15,  7, 13,  5,
];

#[rustfmt::skip]
const BAYER_8: [u8; 64] = [
     0, 32,  8, 40,  2, 34, 10, 42,
    48, 16, 56, 24, 50, 18, 58, 26,
    12, 44,  4, 36, 14, 46,  6, 38,
    60, 28, 52, 20, 62, 30, 54, 22,
     3, 35, 11, 43,  1, 33,  9, 41,
    51, 19, 59, 27, 49, 17, 57, 25,
    15, 47,  7, 39, 13, 45,  5, 37,
    63, 31, 55, 23, 61, 29, 53, 21,
];

/// Builds the normalised threshold matrix, row major, with values in `[0, 1)`.
pub fn matrix(size: BayerSize) -> Vec<f32> {
    let (raw, divisor): (&[u8], f32) = match size {
        BayerSize::Two => (&BAYER_2, 4.0),
        BayerSize::Four => (&BAYER_4, 16.0),
        BayerSize::Eight => (&BAYER_8, 64.0),
    };

    raw.iter().map(|&v| v as f32 / divisor).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrices_are_normalised_permutations() {
        for size in [BayerSize::Two, BayerSize::Four, BayerSize::Eight] {
            let n = size.side();
            let m = matrix(size);

            assert_eq!(m.len(), n * n);

            let mut scaled: Vec<u32> = m.iter().map(|v| (v * (n * n) as f32) as u32).collect();
            scaled.sort_unstable();

            assert_eq!(scaled, (0..(n * n) as u32).collect::<Vec<_>>());
        }
    }

    #[test]
    fn unsupported_sizes_fall_back_to_four() {
        assert_eq!(BayerSize::from_side_or_default(3), BayerSize::Four);
        assert_eq!(BayerSize::from_side_or_default(16), BayerSize::Four);
        assert_eq!(BayerSize::from_side_or_default(2), BayerSize::Two);
        assert_eq!(BayerSize::from_side_or_default(8), BayerSize::Eight);
    }
}
