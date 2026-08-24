# dither-core

A dithering pipeline: it reduces a full-colour photo to a fixed palette, a handful of colours chosen once and reused for every pixel, and hands back a picture made of nothing else.

Ships as both a library (`dither_core`) and a CLI (`dither-core`).

The palette is six colours, each blended between a pure primary and a muted version of it. How far that blend goes is `--saturation`, so the same pipeline covers everything from poster-flat primaries to something closer to ink on paper.

This crate is a member of the workspace at the repository root, alongside `dither-server`, which serves the same pipeline over HTTP. The commands below are run from that root.

## CLI

```bash
cargo build --release -p dither-core
./target/release/dither-core photo.jpg
```

That writes `photo_dithered.png` next to the input, dithered with the default settings and at the size it came in. Nothing is resized until `--size`, `--preset` or `--resize` asks for it.

Several images at once, into a directory:

```bash
dither-core *.jpg -o out/ --verbose
```

A different error-diffusion kernel:

```bash
dither-core photo.jpg -m atkinson
```

Ordered dithering with a 2x2 Bayer matrix and softened thresholds:

```bash
dither-core photo.jpg -m ordered --bayer-size 2 --threshold-scale 0.5
```

Resized, which takes asking:

```bash
dither-core photo.jpg --size 600x400
# a 3456x5184 photo is squashed into 600x400
```

A portrait photo, kept portrait rather than squashed into a landscape working size:

```bash
dither-core photo.jpg --size 600x400 --keep-orientation
# a 3456x5184 photo comes out 400x600, the working size transposed
```

Undistorted whatever the photo's shape, which is the two flags together:

```bash
dither-core photo.jpg --size 600x400 --keep-orientation --crop
# a 3:4 photo comes out 400x600 with the sides trimmed, not stretched to 2:3
```

Keeping a different part than the middle:

```bash
dither-core photo.jpg --crop --crop-from top
dither-core photo.jpg --crop --crop-from 0,3800   # starts here, in source pixels: the top 3800 rows are dropped
```

Checking the framing without the dither in the way:

```bash
dither-core photo.jpg -m none --crop --crop-from 0,600
# the photo resized and cropped, and nothing else
```

`--verbose` reports the rectangle that was kept, which is how to tell what a corner cost:

```bash
dither-core photo.jpg --preset instagram-story --crop --crop-from 0,200 -v
# photo.jpg (1536x2048) -> photo_dithered.png (337x600) in 24ms
#   crop: 1039x1848 from 0,200
```

A shape a platform expects. With no `--size` it is cut out of the photo at full resolution, and `--size` is what asks for a particular number of pixels instead:

```bash
dither-core photo.jpg --preset instagram-story --crop
# a 4000x3000 photo comes out 1687x3000: the largest 9:16 rectangle actually in it

dither-core photo.jpg --preset instagram-story --crop --size 1080x1080
# 607x1080, the same 9:16 with the pixels to post it at
```

A shape has to have something to change, so `--preset` on its own, with neither `--size` to be fitted inside nor `--crop` to be cut out with, is an error rather than a flag that quietly does nothing.

### Options

| Flag | Default | Meaning |
| --- | --- | --- |
| `-o, --output <PATH>` | alongside the input | Output file for one input, or a directory for several |
| `-m, --method <M>` | `floyd-steinberg` | See the methods table below |
| `--saturation <F>` | `0.6` | Blend between the pure and the muted palettes |
| `--brightness <F>` | `1.1` | Brightness multiplier applied before dithering |
| `--color <F>` | `1.4` | Colour intensity multiplier applied before dithering |
| `--bayer-size <N>` | `4` | Bayer matrix size: 2, 4 or 8. Ordered only |
| `--threshold-scale <F>` | `1.0` | Scales the Bayer threshold amplitude. Ordered only |
| `--size <WxH>` | the photo's own | Working size, and naming it is what asks for the resize. With `--preset` it is the box the ratio is fitted inside |
| `--preset <NAME>` | none | Aspect ratio by name, from the table below. Reshapes `--size`, or the photo itself when there is no `--size` |
| `--no-resize` | off | Read `--size` as a shape only and dither at the photo's own resolution. Says nothing without a `--size` |
| `--resize <F>` | off | Scale by a fraction of what the framing kept: `0.75` takes a quarter off, `0.125` an eighth of each side |
| `--keep-orientation` | off | Resize a portrait photo to the transpose of the working size, so it stays portrait. Needs a `--size` or a `--preset` to have a size to transpose |
| `--crop` | off | Crop to the working size's aspect ratio instead of stretching the photo into it |
| `--crop-from <WHERE>` | `center` | Which part the crop keeps: `center`, `top`, `bottom`, `left`, `right`, or a corner as `X,Y`. Needs `--crop` |
| `--crop-zoom <F>` | `1.0` | `1.0` to `10.0`. Above 1.0 the crop keeps a proportionally smaller rectangle. Needs `--crop`. Not needed with a corner |
| `--upscale-2x` | off | Nearest-neighbour doubling of the result |
| `-f, --format <F>` | `indexed` | `indexed` (palette PNG, smaller) or `rgb` |
| `--dry-run` | off | Report what would be written |

### Presets

`--preset` names an aspect ratio, so the shapes do not have to be worked out by hand. It does not replace `--size`: the largest rectangle of that ratio that fits inside it is what gets dithered, so the preset picks the shape and `--size` still picks the scale.

| `--preset` value | Ratio | Inside `--size 600x400` | What it is |
| --- | --- | --- | --- |
| `instagram-post` | 1:1 | 400x400 | Square post |
| `instagram-portrait` | 4:5 | 400x500 | The tallest post the feed takes |
| `instagram-landscape` | 191:100 | 600x314 | 1.91:1 |
| `instagram-story` | 9:16 | 337x600 | Stories and reels |
| `iphone` | 4:3 | 533x400 | The iPhone's default photo shape |

`--size` is turned over first when the ratio disagrees with it, so a portrait ratio is not squeezed into a landscape pair's short side: `--preset instagram-story --size 600x400` is 337x600, not 225x400.

With no `--size` the photo itself is the box, and that one is never turned over: it is already in its own orientation, and turning it over would ask for pixels the file never had. `--preset instagram-story --crop` on a 4000x3000 photo is 1687x3000, cut out at full resolution.

A preset names one orientation, and `--keep-orientation` transposes whichever one is asked for: `--preset iphone --keep-orientation --size 600x400` dithers a portrait photo at 400x533. Add `--crop` and nothing is stretched.

What a run costs follows the box rather than the name, since that is what decides how many pixels get dithered: any preset against `--size 600x400` is the same tens of milliseconds, and a preset cut out of a 4032x3024 photo is where the seconds go.

### Methods

| `-m` value | Kind | Character |
| --- | --- | --- |
| `floyd-steinberg` | error diffusion | The default. The sharpest of them |
| `atkinson` | error diffusion | Sheds some error: cleaner highlights, more contrast |
| `stucki` | error diffusion | Wider spread, smoother gradients |
| `burkes` | error diffusion | Like Stucki but cheaper, grainier |
| `jarvis` | error diffusion | Widest spread, smoothest |
| `ordered` | Bayer threshold | Structured rather than organic. Strong colour cast |
| `none` | none | Writes the photo resized and cropped, for checking the framing. Always an RGB PNG |

Every error-diffusion entry is the same loop reading a different kernel, so the four beyond Floyd-Steinberg come free from the shared table.

## Library

```toml
[dependencies]
dither-core = { path = "crates/dither-core" }
```

```rust
use dither_core::{ATKINSON, DitherMethod, DitherOptions, apply_dithering, io, resize};

let photo = io::load_rgb("photo.jpg")?;
// No size and no preset, so the photo keeps its own. `Some((600, 400))` would resize it.
let target = resize::working_size(photo.dimensions(), None, None);
let sized = resize::resize_image(&photo, target);

let options = DitherOptions {
    method: DitherMethod::ErrorDiffusion(ATKINSON),
    saturation: 0.6,
    ..Default::default()
};
let dithered = apply_dithering(&sized, &options);
io::save_indexed_png(&dithered, "photo_dithered.png")?;
```

`apply_dithering` hands back an `IndexedImage`: one palette slot per pixel plus the palette itself. `io::save_indexed_png` writes it as a palette PNG, `to_rgb` expands it back to full colour, and `scale_nearest` upscales it without leaving the palette.

`resize::working_size` is what turns "a size, a preset, or neither" into the pair to dither at, and it is the one place that rule lives: the CLI, the HTTP backend and the browser front end all call it, so none of the three can drift from the others. A `None` size means the photo's own dimensions, which is why nothing here resizes unless it is asked to.

`resize::resize_image` always produces the size you name, stretching the photo into it. `resize::resize_to_fit` takes a `FitOptions` instead: `keep_orientation` transposes the size for a photo of the other orientation, so against 600x400 a portrait photo comes out 400x600 and a landscape one 600x400, and `crop` takes the largest centred rectangle that already has the target's ratio rather than stretching what is left over.

```rust
use dither_core::FitOptions;

let fit = FitOptions { keep_orientation: true, crop: true, ..Default::default() };
let sized = resize::resize_to_fit(&photo, (600, 400), fit);
```

Which part the crop keeps is `FitOptions::crop_from`, a `CropOrigin`, and its two forms work from opposite ends. An anchor (`Center` by default, `Top`, `Bottom`, `Left`, `Right`) takes the largest rectangle the target's ratio allows and puts it against a side; since such a rectangle spans the photo's full width or full height, an anchor only moves it along the axis that has slack, and `Top` on a photo losing its sides behaves like `Center`. `At { x, y }` is the other way round: the corner is where the crop starts, so it is kept as given and the rectangle is the largest that fits below and to the right of it. That is what makes `At { x: 0, y: 200 }` drop the top 200 rows of any photo, at the cost of size, and a corner past the last pixel keeps that pixel.

`FitOptions::crop_zoom` shrinks whatever the origin settled on, both sides by the same factor. It is what moves an anchor in from the edges; a corner does not need it. `CropOrigin` parses and prints the same spelling the CLI and the HTTP API use.

`resize::fitted_rect` returns the region `resize_to_fit` will read, and `resize::fitted_size` the size it will produce. Both are what `resize_to_fit` itself uses, so a caller reporting them cannot drift from what the pipeline did. `resize::cover_rect` is the geometry underneath, for a caller that wants to say what a photo will lose before running anything. Nothing is copied to crop: the region is read in place, so cropping costs the same as not cropping.

The named ratios are `resize::RATIO_PRESETS`, a table of `(name, (width, height))` where the pair is a shape rather than a pixel count, with `resize::preset_ratio` for the lookup and `resize::preset_names` for building a picker. `resize::ratio_size` turns one into a size: the largest rectangle of that ratio that fits inside the working size handed to it, which it turns over first when the ratio disagrees with it. So a preset only ever reframes a size a caller already picked, and a caller is free to ignore the table and pass its own.

`resize::ratio_inside` is the same fit without the turn-over, for a box that is a real photo rather than a working size: a photo is already in its own orientation, so reorienting it would ask for pixels the file never had. That is the one difference between the two halves of `working_size`.

```rust
let ratio = resize::preset_ratio("instagram-story").expect("a name from the table");
let size = resize::ratio_size((600, 400), ratio); // 337x600, the pair turned over first
let cut = resize::ratio_inside((4000, 3000), ratio); // 1687x3000, what is actually in the photo
```

Inputs are `image::RgbImage`, so anything that crate can decode works, and `RgbImage::from_raw` takes pixels you already have.

### Features

| Feature | Default | Pulls in |
| --- | --- | --- |
| `cli` | yes | The `dither-core` binary (`clap`) |
| `image-io` | yes | `io::load_rgb` and the PNG writers (`png`, plus `image`'s codecs) |

For a library-only build with no codecs:

```toml
dither-core = { path = "crates/dither-core", default-features = false }
```

## What comes from where

Four direct dependencies do most of the work:

| Crate | Used for |
| --- | --- |
| `image` | Buffers, decoding, resampling, rotation, nearest-neighbour scaling |
| `palette` | sRGB to CIELAB and the perceptual colour distance |
| `png` | Palette PNG output, which `image` cannot write |
| `clap` | The CLI |

What is left is written here: the palette and its saturation blend (`palette`), the indexed image the dither produces (`indexed`), the Bayer tables (`bayer`), the enhancers (`enhance`), and the error-diffusion loop (`diffusion`).

That last one is a deliberate exception. `image::imageops::dither` clamps the accumulated error back into a `u8` after every diffusion step, which on a palette this small throws away most of the error and visibly drains the colour out of the result. The `dither` crate gets the arithmetic right but hard-depends on `clap 3.0.0-beta` and `image 0.23`, so it cannot sit alongside `image` 0.25. The loop in `diffusion.rs` is about 40 lines and keeps the running error in `f32`; the kernel table it reads is what gives you Atkinson, Stucki, Burkes and Jarvis for free.

## Performance

`benches/pipeline.rs` times every stage over the photos in `assets/`, so the rows add up to what the CLI actually spends:

```bash
cargo bench -p dither-core
```

[BENCHMARKS.md](BENCHMARKS.md) logs every measurement, one entry per change that moves the numbers, along with the method and the machine each was taken on.

### A caveat about JPEG

JPEG decoders disagree slightly: Pillow uses libjpeg-turbo, the `image` crate uses `zune-jpeg`, and they differ on roughly 0.3% of pixels by a step or two. That is invisible under ordered dithering, but error diffusion is chaotic enough that a one-unit difference in a single pixel propagates across the whole image. Feed a PNG in if you need output that reproduces exactly.
