# Benchmarks

A running log of `benches/pipeline.rs`, one entry per change that moves the numbers. The point is to make optimisation work auditable: every claim of a speedup should have a row here showing what it was before, what it is after, and what it cost elsewhere.

## Reproducing

```bash
cargo bench -p dither-core
```

The bench reads every `.jpg` in `assets/` at the repository root, sorted by filename, so a run on another machine measures the same work. It needs the `image-io` feature, which is on by default.

The CLI wall clock is a separate cross-check, and it should stay close to the bench's end-to-end figure. If the two drift apart, the bench has stopped covering something the CLI actually does:

```bash
cargo build --release -p dither-core
time ./target/release/dither-core assets/*.jpg -o /tmp/bench-out
du -sk /tmp/bench-out
```

## Method

Each stage is timed over the whole photo set, so one sample is one full batch and the per-stage rows add up to the end-to-end row. Stages run a warmup pass first, then several timed passes.

The reported number is the **best** sample, not the mean. On a laptop the mean mostly measures what else the machine was doing; the best sample is the run least disturbed by the scheduler and is the more stable basis for comparing two versions of the same code. The median is reported alongside it to show the spread. When best and median are more than a few percent apart, the machine was busy and the run is worth repeating.

Two cautions when comparing entries:

- Only compare numbers taken on the same machine. The environment block in each entry records which one.
- Thermal state matters. A resize measurement drifted 25% between two runs of identical code during this work, so treat any single-digit-percent change as noise unless it reproduces.

## Environment

Unless an entry says otherwise, measurements are from:

| | |
| --- | --- |
| Machine | Apple M2 Max, 12 cores (8 performance, 4 efficiency) |
| OS | macOS 15.6.1 |
| Toolchain | rustc 1.97.1 |
| Profile | `release`, `opt-level = 3` |
| Dataset | 10 JPEG photos in `assets/`, 177.0 MP total, 8256x5504 at the largest |

## Trend

| Date | Change | Batch (ms) | vs baseline | CLI wall clock | PNG output |
| --- | --- | ---: | ---: | ---: | ---: |
| 2026-08-15 | Baseline, no optimisations | 1061 | 1.00x | 1.10 s | 532 KiB |
| 2026-08-15 | 1. PNG: no row filter, deflate 3 | 968 | 1.10x | 1.01 s | 392 KiB |
| 2026-08-15 | 2. Decode with `into_rgb8` | 965 | 1.10x | 0.98 s | 392 KiB |
| 2026-08-15 | 3. Two-step resize descent | 551 | 1.93x | 0.56 s | 388 KiB |
| 2026-08-15 | 4. rayon, box pass by rows | 494 | 2.15x | 0.52 s | 388 KiB |
| 2026-08-15 | 5. Error diffusion row window: REJECTED, reverted | 495 | 2.15x | 0.52 s | 388 KiB |
| 2026-08-15 | 6. Ordered mode parallelised | 488 | 2.18x | 0.51 s | 388 KiB |
| 2026-08-15 | 7. Secondary passes: REJECTED, reverted | 488 | 2.18x | 0.51 s | 388 KiB |
| 2026-08-15 | 8. CLI: one photo per core | 488 | 2.18x | **0.13 s** | 388 KiB |

## History

### 2026-08-15: baseline, no optimisations

First measurement, taken before any change to the pipeline. This is the reference every later entry compares against.

| stage | best (ms) | median | per image | share |
| --- | ---: | ---: | ---: | ---: |
| resize to 600x400 | 501.7 | 502.0 | 50.17 | 47.3% |
| decode jpeg | 391.9 | 404.2 | 39.19 | 36.9% |
| encode indexed png | 95.3 | 96.5 | 9.53 | 9.0% |
| dither jarvis | 65.8 | 66.1 | 6.58 | 6.2% |
| dither stucki | 65.5 | 66.1 | 6.55 | 6.2% |
| dither burkes | 63.6 | 65.4 | 6.36 | 6.0% |
| dither floyd-steinberg | 62.8 | 63.3 | 6.28 | 5.9% |
| dither atkinson | 62.1 | 62.6 | 6.21 | 5.8% |
| dither ordered | 26.9 | 27.1 | 2.69 | 2.5% |
| of which: ordered LUT build | 15.1 | 15.2 | 1.51 | 1.4% |
| enhance brightness+colour | 3.8 | 3.9 | 0.38 | 0.4% |
| rotate + pack frame buffer | 2.0 | 2.1 | 0.20 | 0.2% |
| **end to end (decode..encode)** | **1061.2** | **1064.8** | **106.12** | **100%** |

Throughput: 451.6 MP/s decoded, 9.4 photos/s end to end.

The shares only sum past 100% because the dither rows are alternatives to each other; end to end runs Floyd-Steinberg, the default.

Three observations from this baseline:

**Dithering is not the bottleneck.** The default kernel is 5.9% of the run. All five kernels land within 6% of each other despite Floyd-Steinberg having 4 taps and Jarvis having 12, which says the per-pixel cost is dominated by something other than spreading the error, most likely the nearest-colour search and the per-pixel accessor overhead.

**Resize and decode are 84% of the run.** Resize alone costs more than everything else combined, which is surprising for a stage that produces 240000 pixels. The input is what makes it expensive: a triangle filter widens its kernel as it downscales, so on a 45 MP photo it spans roughly a hundred source pixels per output pixel.

**Two smaller oddities.** Building the ordered mode's lookup table is 56% of that mode's cost and is redone per image, although it depends only on the saturation. And encoding a 240 KB indexed PNG takes 9.5 ms, which is a lot for that much data.

### 2026-08-15: 1. PNG written with no row filter and deflate level 3

`io::write_indexed_png` now sets `Filter::NoFilter` and `DeflateCompression::Level(3)`.

| stage | before | after | change |
| --- | ---: | ---: | ---: |
| encode indexed png | 95.3 | 22.0 | 4.3x faster |
| **end to end** | **1061.2** | **968.0** | **1.10x faster** |
| PNG output, whole set | 532 KiB | 392 KiB | 26% smaller |

Faster and smaller at once, which is unusual enough to explain. Two independent settings were wrong for this data.

Row filters predict each byte from its neighbours, which pays off on photographic samples where neighbours correlate. These bytes are palette slots, where the numeric difference between slot 6 and slot 1 carries no meaning. Filtering them turns a byte stream with seven distinct values into one with far more, destroying exactly the skew deflate exploits. Turning filters off was worth 31% of the file size on its own, at identical speed.

With filtering gone the compression level was then re-measured, and levels above 3 bought under two percent of size for four times the encode time. Level 3 keeps almost all the ratio.

Measured alternatives, whole set: level 6 with adaptive filtering (the previous default) 105 ms and 507 KiB, level 6 without filtering 105 ms and 348 KiB, level 3 without filtering 24 ms and 369 KiB, level 1 without filtering 10 ms and 540 KiB, fdeflate ultrafast 2 ms and 1286 KiB. Level 3 is the knee of that curve.

Output pixels are untouched: this changes only how the same palette indices are packed into the file.

### 2026-08-15: 2. Decode with `into_rgb8` instead of `to_rgb8`

`io::load_rgb` and `io::decode_rgb` consume the `DynamicImage` rather than borrowing it.

| stage | before | after | change |
| --- | ---: | ---: | ---: |
| decode jpeg | 378.8 | 356.3 | 22.5 ms faster |
| **end to end** | **968.0** | **965.3** | **2.7 ms faster** |

A JPEG decodes to RGB already, so `to_rgb8` was allocating a second buffer and copying the first into it byte for byte. Across this set that is 531 MB of memcpy for no change in content. `into_rgb8` matches on the variant and hands back the buffer it already has.

The end-to-end row understates this, and the gap is worth reading rather than smoothing over: decode fell 22.5 ms but the total moved only 2.7 ms, because resize drifted from 497.7 to 513.2 ms between the two runs on code that did not change. That is thermal noise, and it is the size of noise to expect on this machine. The CLI wall clock, measured separately, went from 1.01 s to 0.98 s and corroborates the decode figure.

Output is byte-for-byte identical to the previous step, verified with `diff -r` over the whole set.

### 2026-08-15: 3. Two-step descent to the working size

`resize::resize_image` box-averages by an integer factor first, then hands the fractional remainder to the triangle filter. Still single-threaded at this point.

| stage | before | after | change |
| --- | ---: | ---: | ---: |
| resize to 600x400 | 513.2 | 119.0 | 4.3x faster |
| **end to end** | **965.3** | **550.5** | **1.75x faster** |
| CLI wall clock | 0.98 s | 0.56 s | 1.75x faster |

The triangle filter was never the wrong choice, it was being asked the wrong question. Its kernel widens as the scale factor grows, so reducing 8256x5504 to 600x400 in one pass makes it average roughly a hundred source pixels for every pixel it emits, and it does that with floating-point weights. The integer box pass does the bulk of that averaging with integer adds, and leaves the filter a reduction of under 2x where its kernel is small.

The factor is the largest integer that keeps both reduced sides at or above the target, so the triangle pass only ever downscales and never has to invent detail. Remainder rows and columns that do not fill a whole block are dropped, at most `factor - 1` pixels off an edge thousands wide.

This is the first change that moves output pixels, so it was measured two ways.

Against a synthetic gradient, the two-step and single-pass results agree to within 2 levels out of 255, and a test in `resize.rs` now locks that bound.

Against the real photo set, 41.4% of palette slots differ. That number looks alarming and is close to meaningless here. Error diffusion is chaotic: a sub-level change to one input pixel flips which of two slots a pixel takes, and the resulting error propagates to every pixel after it. Any change at all halves the exact-match rate. The measure that tracks what the eye does is the local colour average:

| block | mean drift | p99 | max |
| --- | ---: | ---: | ---: |
| 4x4 | 6.09/255 | 32 | 62 |
| 8x8 | 2.36/255 | 11 | 24 |
| 20x20 | 0.75/255 | 4 | 7 |

Rendered side by side, the two versions are indistinguishable. A less aggressive variant that stops at 2x the target, matching Pillow's `reducing_gap = 2.0`, would cut this drift further at the cost of part of the speedup.

### 2026-08-15: 4. rayon, with the box pass split across rows

Adds `rayon` as a dependency and runs `box_reduce` over output rows in parallel. Each output row reads a disjoint band of the source, so no coordination is needed.

| stage | before | after | change |
| --- | ---: | ---: | ---: |
| resize to 600x400 | 119.0 | 53.0 | 2.2x faster |
| **end to end** | **550.5** | **493.8** | **1.11x faster** |
| CLI wall clock | 0.56 s | 0.52 s | 1.08x faster |

2.2x on eight performance cores, not eight. That is the expected shape rather than a disappointment: the box pass reads 531 MB and writes a few MB, doing three adds per byte read, so it is limited by memory bandwidth long before it runs out of cores. Adding threads past the point where the memory system saturates buys nothing.

The parallel and serial versions produce identical files, verified with `diff -r` over the whole set. This step is pure scheduling, so it stays a fair comparison against the previous entry.

`rayon` is the first dependency this work adds. It is used here and in the steps that follow.

### 2026-08-15: 5. Error diffusion row window. Rejected and reverted

The idea: error diffusion only ever writes into the next `max(dy)` rows, which is two for Floyd-Steinberg and three for the widest kernels. Keeping just those rows instead of one error cell per pixel would shrink the working set from 2.9 MB to about 22 KB and should keep it in cache for the whole pass.

It made things slower, three different ways. The full-image buffer stays.

| variant | dither floyd-steinberg |
| --- | ---: |
| Full-image buffer (kept) | 63.4 to 64.1 |
| Window as a `Vec` of row `Vec`s | 69.5 to 71.1 |
| Full-height buffer, same `Vec` of `Vec`s addressing | 71.5 |
| Window as one flat ring buffer | 71.2 to 71.6 |

Each figure reproduced across repeated runs, and the other stages held steady while these moved, so this is not thermal drift.

The third row is the one that settles it. It keeps the restructured addressing but restores the full-height buffer, changing only the amount of memory live. It measures the same as the windowed version, which means the window was never the problem and the restructuring around it was. Chasing a smaller working set turned a single flat index into a ring position that has to be looked up per tap, and the tap loop is the hottest write in the crate.

The premise was wrong to begin with. 2.9 MB already fits in this machine's caches, so there was no locality left to win, only overhead to add. On a machine with a much smaller cache the trade might go the other way.

`diffusion.rs` carries a comment pointing here, so the next reader sees the buffer looks wasteful on purpose.

### 2026-08-15: 6. Ordered mode parallelised

`OrderedLut::new` builds its 32768 entries in parallel chunks, and `ordered` walks output rows in parallel over raw slices instead of going through `GrayImage::from_fn` and `get_pixel`.

| stage | before | after | change |
| --- | ---: | ---: | ---: |
| dither ordered | 27.1 | 8.4 | 3.2x faster |
| of which: LUT build | 15.2 | 2.6 | 5.8x faster |

This is the one stage where parallelism scales close to core count, because every pixel is independent and the working set is small enough that memory bandwidth is not the limit, unlike the box reduce in step 4.

The end-to-end row does not move, because it runs Floyd-Steinberg. This step only pays off for callers who choose `-m ordered`.

Building the lookup table was 56% of this mode's cost and is still redone per image, although it depends only on the saturation. `OrderedLut` is public precisely so a batch caller can build one and keep it; `apply_dithering` cannot, because it takes only the options. Worth revisiting if ordered mode ever becomes the common path.

Parallelism must not change results, so that was checked rather than assumed: the same run under `RAYON_NUM_THREADS=1` produces byte-identical files, and the Floyd-Steinberg output is unchanged from step 4.

### 2026-08-15: 7. Secondary passes. Rejected and reverted

Two small changes, neither of which survived measurement.

**Brightness as a lookup table.** The blend depends only on the channel value, so all 256 answers can be precomputed and the pass becomes a table lookup. It measured 4.1 ms against 3.8 ms, reproducing across three runs.

Slower, and for a good reason. With `from = 0` the blend collapses to one multiply and a clamp, which the compiler vectorises across the whole buffer. A 256-entry table replaces arithmetic the machine does several lanes at a time with a gather it cannot vectorise at all. Precomputation is only a win when what it replaces is expensive, and this was already close to free.

**`IndexedImage::to_rgb` over raw slices** instead of `from_fn` with `get_pixel`. Reverted for a different reason: it is not on any measured path. `to_rgb` runs only for `-f rgb`, which the bench does not cover, so there was no number to justify it. Keeping unmeasured changes would undermine the point of this log.

That is a real gap: the bench times the indexed output path only. Anyone who cares about `-f rgb` should add a stage for it before touching `to_rgb`.

`enhance.rs` carries a comment pointing here.

### 2026-08-15: 8. CLI processes one photo per core

`main` maps over the inputs with a parallel iterator instead of a `for` loop. `process` returns its report as a `String` rather than printing, and `main` prints them afterwards.

| measure | before | after | change |
| --- | ---: | ---: | ---: |
| CLI wall clock, 10 photos | 0.51 s | 0.13 s | 3.9x faster |
| Single 45 MP photo | 0.13 s | 0.11 s | unchanged, as expected |
| end to end (bench) | 487.5 | 487.8 | unchanged, as expected |

The bench does not move and cannot: it walks the photos serially on purpose, so its rows keep measuring the cost of one photo's work. This step changes how many photos are in flight, not what each costs. The two numbers answer different questions and both are worth keeping.

3.9x on eight performance cores. What caps it is that a batch finishes when its slowest photo finishes, and the largest here is 45 MP against a 7 MP smallest, so the tail dominates once the work is spread out. That the 10-photo batch (0.13 s) now costs about what the single largest photo costs (0.11 s) is the clearest sign the batch is bound by its longest item and there is nothing further to win at this level.

Decode is what makes this worth doing: it is 71% of a photo's cost and `zune-jpeg` is single-threaded per image, so the only way to overlap it is to decode several photos at once.

Two things had to be checked rather than assumed. Output is byte-identical to step 6. And the printed lines stay in input order, because `map` over an indexed parallel iterator collects in order, so the output does not depend on which thread finished first.

This does nothing for `dither-server`, which handles one photo per request. Its concurrency comes from tokio serving several requests at once, and each request already benefits from steps 1 to 6.

### 2026-08-15: where things stand

| stage | baseline | now | change |
| --- | ---: | ---: | ---: |
| decode jpeg | 391.9 | 345.2 | 1.14x |
| resize to 600x400 | 501.7 | 49.7 | 10.1x |
| encode indexed png | 95.3 | 22.3 | 4.3x |
| dither floyd-steinberg | 62.8 | 63.8 | unchanged |
| dither ordered | 26.9 | 9.4 | 2.9x |
| enhance | 3.8 | 3.9 | unchanged |
| rotate + pack | 2.0 | 2.0 | unchanged |
| **end to end** | **1061.2** | **487.8** | **2.18x** |
| **CLI, 10 photos** | **1.10 s** | **0.13 s** | **8.5x** |
| PNG output | 532 KiB | 388 KiB | 27% smaller |

Five of the eight attempts were kept. Three of the five that mattered were in code nobody would have called a hot spot: a filter setting, a compression level, and one method name.

Decode is now 71% of a photo's cost and is the floor. `zune-jpeg` runs at about 500 MP/s and, unlike libjpeg-turbo, cannot decode straight to a reduced DCT scale. Since the target is 600x400 and the sources are tens of megapixels, decoding at 1/8 scale would cut this stage by roughly 5 to 8 times and is by far the largest remaining lever. It needs a different decoder, `mozjpeg` or `jpeg-decoder`, so it is a dependency decision rather than a code change.

Smaller leads, in rough order of value:

- The ordered mode rebuilds its lookup table per image although it depends only on the saturation. Worth 2.5 ms per image, but only for `-m ordered`.
- The bench does not cover the `-f rgb` output path at all, so `to_rgb` and `save_rgb_png` are unmeasured.
- Error diffusion is 13% and resisted three attempts at restructuring. Anything further there probably means specialising the tap loop per kernel so its trip count is known at compile time, which is a real complexity cost for a stage this size.

## Adding an entry

For each change that moves the numbers:

1. Run the bench on an otherwise idle machine, and repeat it if best and median disagree by more than a few percent.
2. Add a row to the trend table and a section to the history, newest last.
3. Record what got slower or larger as well as what got faster. A change that trades output size or fidelity for speed is only judgeable if both sides are written down.
4. If the change alters output pixels, say by how much, and use a perceptual measure rather than an exact-match count. Error diffusion is chaotic: a sub-level change to one input pixel flips a slot choice and cascades, so exact-match percentages collapse while the image stays visually identical. Compare local colour averages over blocks instead.
