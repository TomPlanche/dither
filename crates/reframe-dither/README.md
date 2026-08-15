# reframe-dither

A Rust port of the reframe camera's dithering pipeline: it reduces a full-colour photo to the six colours a Waveshare 4" Spectra 6 e-paper panel can show, and packs the result into the panel's frame buffer.

Ships as both a library (`reframe_dither`) and a CLI (`reframe-dither`). Ported from `ImageProcessor` in `../reframe.py`, from [kaloyaan/reframe](https://github.com/kaloyaan/reframe): the algorithm is theirs, this crate only rewrites it in Rust.

This crate is a member of the workspace at the repository root, alongside `dither-server`, which serves the same pipeline over HTTP. The commands below are run from that root.

## CLI

```bash
cargo build --release -p reframe-dither
./target/release/reframe-dither photo.jpg
```

That writes `photo_dithered.png` next to the input, resized to the panel's 600x400 working size and dithered with the camera's default settings.

Several images at once, into a directory:

```bash
reframe-dither *.jpg -o out/ --verbose
```

A different error-diffusion kernel:

```bash
reframe-dither photo.jpg -m atkinson
```

Ordered dithering with a 2x2 Bayer matrix and softened thresholds:

```bash
reframe-dither photo.jpg -m ordered --bayer-size 2 --threshold-scale 0.5
```

A portrait photo, kept portrait rather than squashed into the landscape working size:

```bash
reframe-dither photo.jpg --keep-orientation
# a 3456x5184 photo comes out 400x600, the panel's own portrait size
```

Undistorted whatever the photo's shape, which is the two flags together:

```bash
reframe-dither photo.jpg --keep-orientation --crop
# a 3:4 photo comes out 400x600 with the sides trimmed, not stretched to 2:3
```

A size a platform expects, rather than the panel's:

```bash
reframe-dither photo.jpg --preset instagram-story --crop
# 1080x1920, cropped to 9:16 rather than squeezed into it
```

Also emit the packed frame buffer, ready to hand to `epd.display()`:

```bash
reframe-dither photo.jpg --buffer
# writes photo_dithered.png and photo_dithered.bin (120000 bytes)
```

### Options

| Flag | Default | Meaning |
| --- | --- | --- |
| `-o, --output <PATH>` | alongside the input | Output file for one input, or a directory for several |
| `-m, --method <M>` | `floyd-steinberg` | See the methods table below |
| `--saturation <F>` | `0.6` | Blend between the pure and the muted panel palettes |
| `--brightness <F>` | `1.1` | Brightness multiplier applied before dithering |
| `--color <F>` | `1.4` | Colour intensity multiplier applied before dithering |
| `--bayer-size <N>` | `4` | Bayer matrix size: 2, 4 or 8. Ordered only |
| `--threshold-scale <F>` | `1.0` | Scales the Bayer threshold amplitude. Ordered only |
| `--size <WxH>` | `600x400` | Working size |
| `--preset <NAME>` | none | Working size by name, from the table below. Cannot be given with `--size` |
| `--no-resize` | off | Dither at the source resolution |
| `--keep-orientation` | off | Resize a portrait photo to the transpose of `--size`, so it stays portrait |
| `--crop` | off | Crop to `--size`'s aspect ratio instead of stretching the photo into it |
| `--upscale-2x` | off | Nearest-neighbour doubling, matching the dashboard export |
| `-f, --format <F>` | `indexed` | `indexed` (palette PNG, as the camera saves) or `rgb` |
| `--buffer` | off | Also write the packed e-paper frame buffer |
| `--dry-run` | off | Report what would be written |

The defaults mirror the `processing` section of `settings.example.json`.

### Presets

`--preset` names a working size, so the pixel counts do not have to be remembered. Only the two panel entries can be packed into a frame buffer; the rest are there for a dithered photo that is going somewhere other than the panel.

| `--preset` value | Size | What it is |
| --- | --- | --- |
| `panel` | 600x400 | The default working size, and what `--buffer` expects |
| `panel-portrait` | 400x600 | The panel's own portrait layout |
| `instagram-post` | 1080x1080 | Square post |
| `instagram-portrait` | 1080x1350 | 4:5, the tallest post the feed takes |
| `instagram-landscape` | 1080x566 | 1.91:1 |
| `instagram-story` | 1080x1920 | 9:16, stories and reels |
| `iphone` | 4032x3024 | 4:3, the iPhone's default photo size |

A preset names one orientation, and `--keep-orientation` transposes whichever one is asked for: `--preset iphone --keep-orientation` dithers a portrait photo at 3024x4032. Add `--crop` and nothing is stretched. The large presets are real work rather than a relabelling, since the dither runs at the size asked for: `iphone` takes about 750 ms against 60 ms for `panel`.

### Methods

| `-m` value | Kind | Character |
| --- | --- | --- |
| `floyd-steinberg` | error diffusion | The camera's default. Sharp |
| `atkinson` | error diffusion | Sheds some error: cleaner highlights, more contrast |
| `stucki` | error diffusion | Wider spread, smoother gradients |
| `burkes` | error diffusion | Like Stucki but cheaper, grainier |
| `jarvis` | error diffusion | Widest spread, smoothest |
| `ordered` | Bayer threshold | Structured rather than organic. Strong colour cast |

Only `floyd-steinberg` and `ordered` exist in the Python original; the rest come free from the shared kernel table.

## Library

```toml
[dependencies]
reframe-dither = { path = "../port" }
```

```rust
use reframe_dither::{ATKINSON, DitherMethod, DitherOptions, apply_dithering, io, resize};

let photo = io::load_rgb("photo.jpg")?;
let sized = resize::resize_image(&photo, resize::DISPLAY_IMAGE_SIZE);

let options = DitherOptions {
    method: DitherMethod::ErrorDiffusion(ATKINSON),
    saturation: 0.6,
    ..Default::default()
};
let dithered = apply_dithering(&sized, &options);
io::save_indexed_png(&dithered, "photo_dithered.png")?;
```

Straight to the panel:

```rust
use reframe_dither::{DitherOptions, dither_to_display_buffer};

let (buffer, dithered, _orientation) = dither_to_display_buffer(&sized, &DitherOptions::default());
// `buffer` is 120000 bytes: two 4-bit colour codes per byte, portrait.
```

`resize::resize_image` always produces the size you name, stretching the photo into it. `resize::resize_to_fit` takes a `FitOptions` instead: `keep_orientation` transposes the size for a photo of the other orientation, so a portrait photo comes out 400x600 and a landscape one 600x400, and `crop` takes the largest centred rectangle that already has the target's ratio rather than stretching what is left over. Both 600x400 and 400x600 are sizes the panel takes, and `dither_to_display_buffer` reports which one it got through its `Orientation`.

```rust
use reframe_dither::FitOptions;

let fit = FitOptions { keep_orientation: true, crop: true };
let sized = resize::resize_to_fit(&photo, resize::DISPLAY_IMAGE_SIZE, fit);
```

`resize::cover_rect` returns the rectangle `crop` would keep, for a caller that wants to say what a photo will lose before running the pipeline. Nothing is copied to crop: the region is read in place, so cropping costs the same as not cropping.

The named sizes are `resize::SIZE_PRESETS`, a table of `(name, (width, height))`, with `resize::preset_size` for the lookup and `resize::preset_names` for building a picker. They are plain data, so a caller is free to ignore them and pass its own size.

Inputs are `image::RgbImage`, so anything that crate can decode works, and `RgbImage::from_raw` takes pixels you already have.

### Features

| Feature | Default | Pulls in |
| --- | --- | --- |
| `cli` | yes | The `reframe-dither` binary (`clap`) |
| `image-io` | yes | `io::load_rgb` and the PNG writers (`png`, plus `image`'s codecs) |

For a library-only build with no codecs:

```toml
reframe-dither = { path = "../port", default-features = false }
```

## What comes from where

Four direct dependencies do most of the work:

| Crate | Used for |
| --- | --- |
| `image` | Buffers, decoding, resampling, rotation, nearest-neighbour scaling |
| `palette` | sRGB to CIELAB and the perceptual colour distance |
| `png` | Palette PNG output, which `image` cannot write |
| `clap` | The CLI |

What is left is the part specific to this panel: its palette and saturation blend (`panel`), the Bayer tables (`bayer`), the frame buffer layout (`display`), the PIL-compatible enhancers (`enhance`), and the error-diffusion loop (`diffusion`).

That last one is a deliberate exception. `image::imageops::dither` clamps the accumulated error back into a `u8` after every diffusion step, which on a seven-colour palette throws away most of the error and visibly drains the colour out of the result. The `dither` crate gets the arithmetic right but hard-depends on `clap 3.0.0-beta` and `image 0.23`, so it cannot sit alongside `image` 0.25. The loop in `diffusion.rs` is about 40 lines and keeps the running error in `f32`; the kernel table it reads is what gives you Atkinson, Stucki, Burkes and Jarvis for free.

## Performance

`benches/pipeline.rs` times every stage over the photos in `assets/`, so the rows add up to what the CLI actually spends:

```bash
cargo bench -p reframe-dither
```

[BENCHMARKS.md](BENCHMARKS.md) logs every measurement, one entry per change that moves the numbers, along with the method and the machine each was taken on.

## Fidelity

`tests/parity.rs` measures agreement against fixtures generated from the real `ImageProcessor`. Regenerate them with:

```bash
python3 tests/generate_golden.py   # needs Pillow and NumPy
cargo test -- --nocapture
```

Current agreement:

| Case | Agreement |
| --- | --- |
| Ordered dithering, all Bayer sizes | 100% of pixels identical |
| Bilinear upscale | identical |
| Bilinear downscale | 0.15 mean abs diff per byte |
| 2x downscale | 2.81 mean abs diff per byte |
| Floyd-Steinberg | 60 to 68% of pixels identical |
| Packed frame buffer | 56% of bytes identical |

Ordered dithering is bit-exact: `palette`'s CIELAB agrees with the NumPy formula to the last argmin. Resampling is near-identical except on exact 2x reductions, where Pillow uses a box average and `image` uses a triangle filter.

Floyd-Steinberg diverges, and the percentage understates how close it is. Error diffusion is chaotic: a one-unit difference in a single pixel propagates across the whole image, so any change at all halves the pixel-level agreement while leaving the result visually indistinguishable. Feeding the same photo through both pipelines and comparing side by side shows matching colour, contrast and texture.

Two smaller differences are deliberate. Pillow pads its palette to 256 entries with pure black and then searches all of them, so dark pixels land on palette slot 7 rather than the blended black in slot 0; this crate matches against the seven real colours only. Both fold onto hardware colour code 0, so the panel sees no difference. And Pillow's `BILINEAR` is used for every resize here, including the exact-2x case the Python special-cases into a box reduction.

### A caveat about JPEG

Pillow decodes JPEG with libjpeg-turbo; the `image` crate uses `zune-jpeg`. They disagree on roughly 0.3% of pixels by a step or two. That is invisible under ordered dithering, and under error diffusion it is enough on its own to move about half the output pixels. Feed both sides a PNG if you want to compare them.
