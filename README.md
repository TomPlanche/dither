# dither

A Cargo workspace around a dithering pipeline: the pipeline itself, which reduces a photo to a fixed six-colour palette, an HTTP backend that puts it behind a few routes, and a browser front end that skips the backend entirely and runs the pipeline in WebAssembly.

```
crates/
  dither-core/      library + CLI: the dithering pipeline itself
  dither-server/    library + binary: the HTTP backend
  dither-app/       binary: the browser front end, Leptos on wasm32
```

`dither-server` is stateless. It takes an uploaded photo, runs it through `dither-core`, and hands back a dithered PNG. Nothing is written to disk and nothing is kept between requests.

## Running the front end

```bash
cd crates/dither-app
trunk serve --release
```

It serves the app on `http://127.0.0.1:8080`. Drop a photo on it, or pick one, and the controls on the right reshape it live.

With no photo of your own to hand, the Samples row offers the ones in the repository's `assets/`, labelled with the photographer who took them. Trunk copies that directory beside the bundle rather than into it, since nine megabytes of JPEG would dwarf the WebAssembly module and be downloaded whether or not anyone picked one: a sample is fetched when it is clicked and not before. The list itself is read from the directory by `build.rs`, so adding or removing a photo there is the whole of the change.

A sample arrives with the scale already set to a quarter of each side. They are twenty-megapixel photographs, and dithering one whole is seconds of work for a result nobody asked to wait for; a quarter is a sixteenth of the pixels and turns the sliders back into something you can drag. Set it back to `Photo's own size` under Framing when you want the full thing.

Nothing is uploaded. `dither-app` is Rust like everything else here, so it links `dither-core` into its own WebAssembly module and calls the pipeline directly, with no HTTP and no `wasm-bindgen` boundary in between. The photo is decoded, resized and dithered in the tab that opened it, and the backend is not involved at all. It is a static bundle: `trunk build --release` writes `dist/`, which any file server can host.

The pipeline is split in two so the sliders have a chance of keeping up: a colour change re-dithers the working image rather than resampling the original photo again. `dither-core` is also built without its `parallel` feature here, because a browser tab has no threads to spread the rows over.

How much that buys depends on the working size, and the working size now defaults to the photo's own. A 12-megapixel upload is dithered at 12 megapixels on the tab's only thread, which is seconds per change, not milliseconds. Nothing here picks a smaller size on your behalf for a photo you brought, so a large one wants a size picked under Framing: `Scale to size` with a pair, or one of the fractions down to an eighth. The samples are the exception, since their size is known in advance. The fastest thing to do while hunting for settings is to work small and switch back at the end.

The download button writes a real palette PNG through `dither-core`'s own encoder, which is the one thing the browser cannot do for itself: `canvas.toBlob` only ever writes truecolour RGBA.

What the front end does not offer is `crop_from` as a pair of source-pixel coordinates. That needs a number rather than a menu, so it is left to the API below.

### First-time setup

```bash
rustup target add wasm32-unknown-unknown
brew install trunk
```

## Running the backend

```bash
cargo run -p dither-server
```

It listens on `127.0.0.1:3000` and already allows the Vite dev server at `http://localhost:5173`, so a fresh SvelteKit project can call it with no further setup. The Rust front end above does not need it.

Configuration comes from the environment, all of it optional:

| Variable | Default | Meaning |
| --- | --- | --- |
| `HOST` | `127.0.0.1` | Interface to bind. Use `0.0.0.0` to accept outside connections. |
| `PORT` | `3000` | Port to bind. |
| `CORS_ORIGINS` | `http://localhost:5173,http://127.0.0.1:5173` | Comma separated list of allowed browser origins, or `*` for any. |
| `MAX_UPLOAD_BYTES` | `26214400` (25 MiB) | Largest request body accepted. Anything over is a 413. |
| `RUST_LOG` | `dither_server=info,tower_http=info` | Log filter. `tower_http=debug` logs every request. |

## API

### `GET /health`

```json
{ "status": "ok", "service": "dither-server", "version": "0.1.0" }
```

### `GET /api/options`

Defaults, accepted values, limits and the palette. The `defaults` object uses exactly the query parameter names, so a client can feed it straight into `URLSearchParams` and edit from there.

```json
{
  "methods": ["floyd-steinberg", "atkinson", "stucki", "burkes", "jarvis", "ordered", "none"],
  "formats": ["indexed", "rgb"],
  "bayer_sizes": [2, 4, 8],
  "presets": [{ "name": "instagram-post", "ratio": [1, 1] }, { "name": "instagram-story", "ratio": [9, 16] }, "..."],
  "defaults": { "method": "floyd-steinberg", "saturation": 0.6, "brightness": 1.1, "color": 1.4, "bayer_size": 4, "threshold_scale": 1.0, "keep_orientation": false, "crop": false, "scale": 1, "format": "indexed" },
  "limits": { "max_upload_bytes": 26214400, "max_dimension": 4096, "max_scale": 4, "max_source_pixels": 50000000, "max_crop_zoom": 10.0 },
  "palette": ["#221c22", "#ffffff", "#e2d82a", "#c32b2d", "#004cff", "#189c22"]
}
```

The palette is the blend at the default saturation, ready to drop into CSS.

`width`, `height` and `resize` are absent from `defaults` because the default asks for no resizing at all, the same way `crop_from` is absent until a crop needs one. Posting the defaults back unchanged therefore stays a valid request, and one that hands the photo back at its own size.

### `POST /api/dither`

Returns the dithered image as `image/png`, with an `x-image-size` header carrying `WIDTHxHEIGHT` and an `x-crop-rect` header carrying `X,Y,WIDTH,HEIGHT`, the part of the upload that was read.

### Sending the image

The POST route accepts the image two ways:

- a raw body, which is the shortest path from a browser: `fetch(url, { method: 'POST', body: file })`
- `multipart/form-data` with a field named `image` (or `file`), which is what a plain `<form>` submits

### Settings

Settings ride in the query string on both POST routes. Every one is optional.

| Parameter | Default | Accepts |
| --- | --- | --- |
| `method` | `floyd-steinberg` | `floyd-steinberg`, `atkinson`, `stucki`, `burkes`, `jarvis`, `ordered`, or `none` for no dithering at all |
| `saturation` | `0.6` | `0.0` to `1.0`. Blends between the pure and the muted palettes. |
| `brightness` | `1.1` | `0.0` to `5.0`. Applied before dithering. |
| `color` | `1.4` | `0.0` to `5.0`. Applied before dithering, after brightness. |
| `bayer_size` | `4` | `2`, `4` or `8`. Ordered dithering only. |
| `threshold_scale` | `1.0` | `0.0` to `5.0`. Ordered dithering only. |
| `width` | none | `1` to `4096`. Goes with `height`; naming the pair is itself the request to scale to it. |
| `height` | none | `1` to `4096`. Goes with `width`. |
| `preset` | none | A named aspect ratio, fitted inside `width`x`height`, or inside the photo itself when there is no pair. See the table below. |
| `resize` | unstated | `true` scales to the working size, `false` keeps the source resolution, and a fraction between 0 and 1 takes that much of each side. Left out, a request with a `width` and `height` scales to them and one without keeps the photo's own size. |
| `keep_orientation` | `false` | `true` transposes `width`x`height` for a photo that disagrees with it, so a portrait upload stays portrait. Nothing to do without a pair or a preset, since a photo already has its own orientation. |
| `crop` | `false` | `true` crops to `width`x`height`'s aspect ratio instead of stretching the photo into it. |
| `crop_from` | `center` | Which part the crop keeps: `center`, `top`, `bottom`, `left`, `right`, or a corner as `X,Y`. Needs `crop=true`. |
| `crop_zoom` | `1.0` | `1.0` to `10.0`. Above 1.0 the crop keeps a proportionally smaller rectangle. Needs `crop=true`. Not needed with a corner. |
| `scale` | `1` | `1` to `4`. Nearest-neighbour upscale of the result. |
| `format` | `indexed` | `indexed` for a palette PNG, `rgb` for a plain one. |

An unknown parameter is an error rather than a silent no-op, so a typo shows up immediately.

`keep_orientation` and `crop` are what an upload of any shape needs to come out undistorted: the first follows the photo's own orientation, the second trims the long side rather than stretching the short one. Neither changes the size that comes back, so a client can keep reading it off `x-image-size`.

### Size

Nothing is resized unless something asks for it. A photo posted with no settings comes back the size it went in, dithered and nothing else, which is why there is no default `width` or `height` to report: the pipeline has no size of its own to prefer.

`resize` answers the one question of how much smaller the photo should come back, three ways:

| `resize` | what it does |
| --- | --- |
| unstated | Scales to `width`x`height` when the pair is there, and keeps the photo's own size when it is not. |
| `true` | Scales to the working size: `width`x`height`, reshaped by any `preset`. Refused without the pair, since there would be no size to fit to. |
| `false` | Keeps the source resolution. The pair, if there is one, is then a shape for `crop` rather than a size. |
| `0.75` | Takes three quarters of each side of whatever the framing kept, so a quarter off the photo. |

It governs the scaling alone, so `crop` keeps framing under all of them: the first says how much smaller, the second says what shape. A 1536x2048 photo with `preset=instagram-story&crop=true` comes back 1152x2048 with nothing said about `resize`, 864x1536 under `resize=0.75`, and 337x600 under `width=600&height=400`. `x-crop-rect` reports the region that was read whichever it is.

Since a shape can only be honoured by cropping to it or by scaling to a size, `preset` with neither a pair nor `crop=true` is a `400` rather than a setting that is read and then does nothing.

`method=none` skips the dither and returns the photo resized and cropped, and nothing else. It is for checking the framing, where the dither pattern is in the way. `resize`, `preset`, `crop`, `crop_from`, `crop_zoom`, `scale` and the `x-crop-rect` header all work the same; the palette settings have nothing to act on, and `format` has no palette to index, so the result is always a plain RGB PNG.

`crop_from` says which part the crop keeps, and the two forms work from opposite ends.

An **anchor** (`center`, `top`, `bottom`, `left`, `right`) asks for the largest rectangle the working size's ratio allows and puts it against a side. Such a rectangle spans the upload's full width or its full height, never neither, so an anchor can only slide it along whichever axis has slack: `top` on a photo that is losing its sides does the same as `center`.

A **corner** (`X,Y` in source pixels, `0,0` being the top-left) is the other way round. The corner is where the crop starts, so it is kept as given, and the rectangle is the largest that fits in what is left below and to the right of it. `crop_from=0,200` therefore drops the top 200 rows of any photo. What it costs is size: a corner far into the upload leaves a small rectangle to blow back up to the working size, and past the last pixel it keeps that pixel. `x-crop-rect` reports what was kept.

Two spellings are a 400 rather than a silent centre crop: one that is neither an anchor nor `X,Y`, and `crop_from` without `crop=true`, which would have nothing to place. Same for `crop_zoom`. That is why both are absent from `defaults` when unset, so posting the reported defaults back unchanged stays a valid request.

`crop_zoom` shrinks whatever the origin settled on, both sides by the same factor so the ratio holds. It is what moves an anchor in from the edges: a 1536x2048 photo into `instagram-story` keeps 1152x2048 centred at `crop_zoom=1.0`, and 576x1024 at `crop_zoom=2`, where `top` and `bottom` then differ. A corner does not need it, since a corner already decides where the rectangle starts.

Both POST routes answer with `x-crop-rect: X,Y,WIDTH,HEIGHT`, the part of the upload that was read, in source pixels. It is the whole photo when `crop` is off, so it always reports what the upload measured, which is what a client needs before it can name a corner. It comes from the pipeline's own geometry rather than being worked out again, so it cannot disagree with the image that came back.

### Presets

`preset` names an aspect ratio instead of working the shape out by hand. It does not replace `width` and `height`: the largest rectangle of that ratio that fits inside the pair is what gets dithered, so the preset picks the shape and the pair still picks the scale. `GET /api/options` carries the same table under `presets`, so a picker can be built from the API rather than hardcoded.

| `preset` value | Ratio | Inside 600x400 | What it is |
| --- | --- | --- | --- |
| `instagram-post` | 1:1 | 400x400 | Square post |
| `instagram-portrait` | 4:5 | 400x500 | The tallest post the feed takes |
| `instagram-landscape` | 191:100 | 600x314 | 1.91:1 |
| `instagram-story` | 9:16 | 337x600 | Stories and reels |
| `iphone` | 4:3 | 533x400 | The iPhone's default photo shape |

The pair is turned over first when the ratio disagrees with it, so a portrait ratio is not squeezed into a landscape pair's short side: `preset=instagram-story` against `width=600&height=400` is 337x600, not 225x400.

With no pair, the photo itself is the box, and that one is never turned over: it is already in its own orientation, and turning it over would ask for pixels the upload never had. `preset=instagram-story&crop=true` on a 4000x3000 photo is 1687x3000, the largest 9:16 rectangle actually in there, cut out at full resolution.

Since a preset is fitted inside something rather than carrying its own pixel count, asking for more resolution is a matter of asking for a bigger box: `preset=instagram-story&width=1080&height=1080` returns 607x1080 of the same shape. What a request costs therefore follows the box, not the name. `x-image-size` reports what it landed on.

A preset names one orientation, and `keep_orientation=true` transposes it for a photo of the other one, so `preset=iphone&keep_orientation=true&crop=true` returns 400x533 undistorted. An unknown name is a 400 listing the ones that work.

### Errors

Every failure is JSON, whatever caused it:

```json
{ "error": "saturation must be between 0.0 and 1.0, got 9", "status": 400 }
```

`400` covers bad settings, an unreadable image and a missing body. `413` means the upload was over `MAX_UPLOAD_BYTES`. `500` means the pipeline itself failed.

## Calling it from JavaScript

`src/lib/dither.ts`:

```ts
const API = import.meta.env.VITE_DITHER_API ?? 'http://localhost:3000';

export type DitherMethod =
  | 'floyd-steinberg'
  | 'atkinson'
  | 'stucki'
  | 'burkes'
  | 'jarvis'
  | 'ordered';

export type DitherPreset =
  | 'instagram-post'
  | 'instagram-portrait'
  | 'instagram-landscape'
  | 'instagram-story'
  | 'iphone';

export type DitherParams = {
  method: DitherMethod;
  saturation: number;
  brightness: number;
  color: number;
  bayer_size: number;
  threshold_scale: number;
  width: number;
  height: number;
  /** An aspect ratio, fitted inside `width` x `height` rather than replacing it. */
  preset: DitherPreset;
  /** true fits the working size, false keeps the source resolution, 0 to 1 is a fraction of it. */
  resize: boolean | number;
  keep_orientation: boolean;
  crop: boolean;
  /** Needs `crop: true`. 'center' | 'top' | 'bottom' | 'left' | 'right', or a corner as `${number},${number}` */
  crop_from: string;
  /** Needs `crop: true`. 1.0 keeps as much as the ratio allows, above that keeps less and frees both axes. */
  crop_zoom: number;
  scale: number;
  format: 'indexed' | 'rgb';
};

const query = (params: Partial<DitherParams>): string =>
  new URLSearchParams(
    Object.entries(params).map(([key, value]) => [key, String(value)])
  ).toString();

const send = async (route: string, file: File, params: Partial<DitherParams>): Promise<Response> => {
  const response = await fetch(`${API}${route}?${query(params)}`, {
    method: 'POST',
    headers: { 'content-type': file.type || 'application/octet-stream' },
    body: file
  });

  if (!response.ok) {
    const { error } = (await response.json()) as { error: string };
    throw new Error(error);
  }
  return response;
};

/** The dithered preview, as an object URL you can hand to an `<img>`. */
export const dither = async (file: File, params: Partial<DitherParams> = {}): Promise<string> => {
  const response = await send('/api/dither', file, params);
  return URL.createObjectURL(await response.blob());
};

/** Defaults and accepted values, for building the controls. */
export const options = async (): Promise<unknown> => {
  const response = await fetch(`${API}/api/options`);
  return await response.json();
};
```

A component using it:

```svelte
<script lang="ts">
  import { dither, type DitherMethod } from '$lib/dither';

  let preview = $state<string | null>(null);
  let error = $state<string | null>(null);
  let method = $state<DitherMethod>('floyd-steinberg');

  const onPick = async (event: Event) => {
    const file = (event.target as HTMLInputElement).files?.[0];
    if (!file) return;

    error = null;
    try {
      preview = await dither(file, { method, scale: 2, format: 'rgb' });
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  };
</script>

<input type="file" accept="image/*" onchange={onPick} />
{#if error}<p class="error">{error}</p>{/if}
{#if preview}<img src={preview} alt="Dithered preview" />{/if}
```

Note the `format: 'rgb'` in the preview call. Indexed PNGs are smaller and the default, but some browsers render them with slightly different colour management, so `rgb` is the safer choice for an on-screen preview.

### Skipping CORS in development

If you would rather keep everything same-origin during development, proxy through Vite instead and drop the `API` prefix:

```ts
// vite.config.ts
export default defineConfig({
  plugins: [sveltekit()],
  server: {
    proxy: { '/api': 'http://localhost:3000' }
  }
});
```

## Development

```bash
cargo test --workspace          # unit tests plus the end-to-end API tests
cargo clippy --workspace --all-targets
cargo fmt --all
cargo run -p dither-server      # the backend
cargo run -p dither-core -- photo.jpg   # the CLI

cargo clippy -p dither-app --target wasm32-unknown-unknown   # the front end, on the target it ships to
```

The API tests in `crates/dither-server/tests/api.rs` drive the router in-process, so they bind no socket and need no running server.

A bare `cargo build` or `cargo test` skips `dither-app`: the workspace lists only the two native crates in `default-members`, because the front end is only ever built for `wasm32-unknown-unknown` and Trunk is what builds it. `--workspace` still reaches it, which is why the command above names the target explicitly.

`dither-core` carries a `parallel` feature, on by default through `cli` and asked for by name in `dither-server`. It gates the `rayon` dependency and the row-level threading in `dither`, `resize` and the CLI's batch loop. `dither-app` leaves it off, and `crates/dither-core/src/parallel.rs` swaps in the sequential `chunks_mut` behind the same names so the hot loops read the same either way.

## Credits

The dithering itself leans on two crates: [`image`](https://github.com/image-rs/image) for buffers, decoding and resampling, and [`palette`](https://github.com/Ogeon/palette) for the CIELAB colour science behind the nearest-colour search. What is written here is the palette and its saturation blend, the framing, the Bayer tables and the error-diffusion loop.
