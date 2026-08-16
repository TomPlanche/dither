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

Keeping a different part than the middle:

```bash
reframe-dither photo.jpg --crop --crop-from top
reframe-dither photo.jpg --crop --crop-from 0,3800   # starts here, in source pixels: the top 3800 rows are dropped
```

Checking the framing without the dither in the way:

```bash
reframe-dither photo.jpg -m none --crop --crop-from 0,600
# the photo resized and cropped, and nothing else
```

`--verbose` reports the rectangle that was kept, which is how to tell what a corner cost:

```bash
reframe-dither photo.jpg --preset instagram-story --crop --crop-from 0,200 -v
# photo.jpg (1536x2048) -> photo_dithered.png (337x600) in 24ms
#   crop: 1039x1848 from 0,200
```

A shape a platform expects, rather than the panel's, at whatever size `--size` asks for:

```bash
reframe-dither photo.jpg --preset instagram-story --crop
# 337x600, cropped to 9:16 rather than squeezed into it

reframe-dither photo.jpg --preset instagram-story --crop --size 1080x1080
# 607x1080, the same 9:16 with the pixels to post it at
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
| `--size <WxH>` | `600x400` | Working size. With `--preset` it is the box the ratio is fitted inside |
| `--preset <NAME>` | none | Aspect ratio by name, from the table below. Reshapes `--size` rather than replacing it |
| `--no-resize` | off | Dither at the source resolution |
| `--keep-orientation` | off | Resize a portrait photo to the transpose of the working size, so it stays portrait |
| `--crop` | off | Crop to the working size's aspect ratio instead of stretching the photo into it |
| `--crop-from <WHERE>` | `center` | Which part the crop keeps: `center`, `top`, `bottom`, `left`, `right`, or a corner as `X,Y`. Needs `--crop` |
| `--crop-zoom <F>` | `1.0` | `1.0` to `10.0`. Above 1.0 the crop keeps a proportionally smaller rectangle. Needs `--crop`. Not needed with a corner |
| `--upscale-2x` | off | Nearest-neighbour doubling, matching the dashboard export |
| `-f, --format <F>` | `indexed` | `indexed` (palette PNG, as the camera saves) or `rgb` |
| `--buffer` | off | Also write the packed e-paper frame buffer |
| `--dry-run` | off | Report what would be written |

The defaults mirror the `processing` section of `settings.example.json`.

### Presets

`--preset` names an aspect ratio, so the shapes do not have to be worked out by hand. It does not replace `--size`: the largest rectangle of that ratio that fits inside it is what gets dithered, so the preset picks the shape and `--size` still picks the scale. The rest of the table is there for a dithered photo that is going somewhere other than the panel.

| `--preset` value | Ratio | Inside the default 600x400 | What it is |
| --- | --- | --- | --- |
| `panel` | 3:2 | 600x400 | The default working shape, and what `--buffer` expects |
| `panel-portrait` | 2:3 | 400x600 | The panel's own portrait layout |
| `instagram-post` | 1:1 | 400x400 | Square post |
| `instagram-portrait` | 4:5 | 400x500 | The tallest post the feed takes |
| `instagram-landscape` | 191:100 | 600x314 | 1.91:1 |
| `instagram-story` | 9:16 | 337x600 | Stories and reels |
| `iphone` | 4:3 | 533x400 | The iPhone's default photo shape |

`--size` is turned over first when the ratio disagrees with it, so a portrait ratio is not squeezed into the landscape default's short side: `--preset panel-portrait` against 600x400 is the panel's own 400x600, not 266x400. That is what keeps both panel entries packable by `--buffer`.

A preset names one orientation, and `--keep-orientation` transposes whichever one is asked for: `--preset iphone --keep-orientation` dithers a portrait photo at 400x533. Add `--crop` and nothing is stretched.

What a run costs follows `--size` rather than the name, since that is what decides how many pixels get dithered: any preset against the default 600x400 is the same tens of milliseconds, and `--size 4032x3024` is where the seconds go.

### Methods

| `-m` value | Kind | Character |
| --- | --- | --- |
| `floyd-steinberg` | error diffusion | The camera's default. Sharp |
| `atkinson` | error diffusion | Sheds some error: cleaner highlights, more contrast |
| `stucki` | error diffusion | Wider spread, smoother gradients |
| `burkes` | error diffusion | Like Stucki but cheaper, grainier |
| `jarvis` | error diffusion | Widest spread, smoothest |
| `ordered` | Bayer threshold | Structured rather than organic. Strong colour cast |
| `none` | none | Writes the photo resized and cropped, for checking the framing. Always an RGB PNG, and refused with `--buffer` |

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

let fit = FitOptions { keep_orientation: true, crop: true, ..Default::default() };
let sized = resize::resize_to_fit(&photo, resize::DISPLAY_IMAGE_SIZE, fit);
```

Which part the crop keeps is `FitOptions::crop_from`, a `CropOrigin`, and its two forms work from opposite ends. An anchor (`Center` by default, `Top`, `Bottom`, `Left`, `Right`) takes the largest rectangle the target's ratio allows and puts it against a side; since such a rectangle spans the photo's full width or full height, an anchor only moves it along the axis that has slack, and `Top` on a photo losing its sides behaves like `Center`. `At { x, y }` is the other way round: the corner is where the crop starts, so it is kept as given and the rectangle is the largest that fits below and to the right of it. That is what makes `At { x: 0, y: 200 }` drop the top 200 rows of any photo, at the cost of size, and a corner past the last pixel keeps that pixel.

`FitOptions::crop_zoom` shrinks whatever the origin settled on, both sides by the same factor. It is what moves an anchor in from the edges; a corner does not need it. `CropOrigin` parses and prints the same spelling the CLI and the HTTP API use.

`resize::fitted_rect` returns the region `resize_to_fit` will read, and `resize::fitted_size` the size it will produce. Both are what `resize_to_fit` itself uses, so a caller reporting them cannot drift from what the pipeline did. `resize::cover_rect` is the geometry underneath, for a caller that wants to say what a photo will lose before running anything. Nothing is copied to crop: the region is read in place, so cropping costs the same as not cropping.

The named ratios are `resize::RATIO_PRESETS`, a table of `(name, (width, height))` where the pair is a shape rather than a pixel count, with `resize::preset_ratio` for the lookup and `resize::preset_names` for building a picker. `resize::ratio_size` turns one into a size: the largest rectangle of that ratio that fits inside the working size handed to it, which it turns over first when the ratio disagrees with it. So a preset only ever reframes a size a caller already picked, and a caller is free to ignore the table and pass its own.

```rust
let ratio = resize::preset_ratio("instagram-story").expect("a name from the table");
let size = resize::ratio_size(resize::DISPLAY_IMAGE_SIZE, ratio); // 337x600
```

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
