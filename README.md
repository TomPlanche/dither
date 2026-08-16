# dither

A Cargo workspace around a dithering pipeline: the pipeline itself, which reduces a photo to a fixed palette, and an HTTP backend that puts it behind a few routes for a Svelte front end. It takes its inspiration from [kaloyaan/reframe](https://github.com/kaloyaan/reframe), a camera that dithers to the same palette in Python.

```
crates/
  dither-core/   library + CLI: the dithering pipeline itself
  dither-server/    library + binary: the HTTP backend
```

`dither-server` is stateless. It takes an uploaded photo, runs it through `dither-core`, and hands back either a dithered PNG or the packed e-paper frame buffer. Nothing is written to disk and nothing is kept between requests.

## Running the backend

```bash
cargo run -p dither-server
```

It listens on `127.0.0.1:3000` and already allows the Vite dev server at `http://localhost:5173`, so a fresh SvelteKit project can call it with no further setup.

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

Defaults, accepted values, limits and the panel palette. The `defaults` object uses exactly the query parameter names, so a client can feed it straight into `URLSearchParams` and edit from there.

```json
{
  "methods": ["floyd-steinberg", "atkinson", "stucki", "burkes", "jarvis", "ordered", "none"],
  "formats": ["indexed", "rgb"],
  "bayer_sizes": [2, 4, 8],
  "presets": [{ "name": "panel", "ratio": [3, 2] }, { "name": "instagram-story", "ratio": [9, 16] }, "..."],
  "defaults": { "method": "floyd-steinberg", "saturation": 0.6, "brightness": 1.1, "color": 1.4, "bayer_size": 4, "threshold_scale": 1.0, "width": 600, "height": 400, "resize": true, "keep_orientation": false, "crop": false, "scale": 1, "format": "indexed" },
  "limits": { "max_upload_bytes": 26214400, "max_dimension": 4096, "max_scale": 4, "max_source_pixels": 50000000, "max_crop_zoom": 10.0 },
  "panel": { "image_size": [600, 400], "panel_size": [400, 600], "palette": ["#221c22", "#ffffff", "#e2d82a", "#c32b2d", "#221c22", "#004cff", "#189c22"] }
}
```

The palette is the blend at the default saturation, ready to drop into CSS.

### `POST /api/dither`

Returns the dithered image as `image/png`, with an `x-image-size` header carrying `WIDTHxHEIGHT` and an `x-crop-rect` header carrying `X,Y,WIDTH,HEIGHT`, the part of the upload that was read.

### `POST /api/buffer`

Returns the packed frame buffer as `application/octet-stream`: two 4-bit colour codes per byte, 120000 bytes for the 400x600 panel. The `x-panel-orientation` header says whether the image was already portrait (`panel`) or had to be turned a quarter turn (`rotated`).

The panel accepts one layout only, so a request that dithers to something other than 600x400 or 400x600 is refused with a 400. `format` and `scale` do not apply here and are ignored.

With `keep_orientation=true` a portrait upload resizes straight to the panel's own 400x600, so it reports `panel` and reaches the hardware without a quarter turn.

### Sending the image

Both POST routes accept the image two ways:

- a raw body, which is the shortest path from a browser: `fetch(url, { method: 'POST', body: file })`
- `multipart/form-data` with a field named `image` (or `file`), which is what a plain `<form>` submits

### Settings

Settings ride in the query string on both POST routes. Every one is optional.

| Parameter | Default | Accepts |
| --- | --- | --- |
| `method` | `floyd-steinberg` | `floyd-steinberg`, `atkinson`, `stucki`, `burkes`, `jarvis`, `ordered`, or `none` for no dithering at all |
| `saturation` | `0.6` | `0.0` to `1.0`. Blends between the muted and the pure panel palettes. |
| `brightness` | `1.1` | `0.0` to `5.0`. Applied before dithering. |
| `color` | `1.4` | `0.0` to `5.0`. Applied before dithering, after brightness. |
| `bayer_size` | `4` | `2`, `4` or `8`. Ordered dithering only. |
| `threshold_scale` | `1.0` | `0.0` to `5.0`. Ordered dithering only. |
| `width` | `600` | `1` to `4096` |
| `height` | `400` | `1` to `4096` |
| `preset` | none | A named aspect ratio, fitted inside `width`x`height`. See the table below. |
| `resize` | `true` | `true` scales to the working size, `false` keeps the source resolution, and a fraction between 0 and 1 takes that much of each side. It governs the scaling only: `crop` still frames the photo, to `width`x`height`'s shape or the preset's. |
| `keep_orientation` | `false` | `true` transposes `width`x`height` for a photo that disagrees with it, so a portrait upload stays portrait. |
| `crop` | `false` | `true` crops to `width`x`height`'s aspect ratio instead of stretching the photo into it. |
| `crop_from` | `center` | Which part the crop keeps: `center`, `top`, `bottom`, `left`, `right`, or a corner as `X,Y`. Needs `crop=true`. |
| `crop_zoom` | `1.0` | `1.0` to `10.0`. Above 1.0 the crop keeps a proportionally smaller rectangle. Needs `crop=true`. Not needed with a corner. |
| `scale` | `1` | `1` to `4`. Nearest-neighbour upscale of the result. |
| `format` | `indexed` | `indexed` for a palette PNG, `rgb` for a plain one. |

An unknown parameter is an error rather than a silent no-op, so a typo shows up immediately.

`keep_orientation` and `crop` are what an upload of any shape needs to come out undistorted: the first picks the panel layout the photo is closer to, the second trims the long side rather than stretching the short one. Neither changes the size that comes back, so a client can keep reading it off `x-image-size`.

`resize` answers one question, how much smaller the photo should come back, three ways:

| `resize` | what it does |
| --- | --- |
| `true` | Scales to the working size: `width`x`height`, reshaped by any `preset`. |
| `false` | Keeps the source resolution. |
| `0.75` | Takes three quarters of each side of whatever the framing kept, so a quarter off the photo. |

It governs the scaling alone, so `crop` keeps framing under all three: the first says how much smaller, the second says what shape. A 1536x2048 photo with `preset=instagram-story&crop=true` comes back 1152x2048 under `resize=false` and 864x1536 under `resize=0.75`, against 337x600 under `resize=true`. `x-crop-rect` reports the region that was read whichever it is.

`method=none` skips the dither and returns the photo resized and cropped, and nothing else. It is for checking the framing, where the dither pattern is in the way. `resize`, `preset`, `crop`, `crop_from`, `crop_zoom`, `scale` and the `x-crop-rect` header all work the same; the palette settings have nothing to act on, and `format` has no palette to index, so the result is always a plain RGB PNG. `POST /api/buffer` refuses it with a 400, since the panel takes palette slots.

`crop_from` says which part the crop keeps, and the two forms work from opposite ends.

An **anchor** (`center`, `top`, `bottom`, `left`, `right`) asks for the largest rectangle the working size's ratio allows and puts it against a side. Such a rectangle spans the upload's full width or its full height, never neither, so an anchor can only slide it along whichever axis has slack: `top` on a photo that is losing its sides does the same as `center`.

A **corner** (`X,Y` in source pixels, `0,0` being the top-left) is the other way round. The corner is where the crop starts, so it is kept as given, and the rectangle is the largest that fits in what is left below and to the right of it. `crop_from=0,200` therefore drops the top 200 rows of any photo. What it costs is size: a corner far into the upload leaves a small rectangle to blow back up to the working size, and past the last pixel it keeps that pixel. `x-crop-rect` reports what was kept.

Two spellings are a 400 rather than a silent centre crop: one that is neither an anchor nor `X,Y`, and `crop_from` without `crop=true`, which would have nothing to place. Same for `crop_zoom`. That is why both are absent from `defaults` when unset, so posting the reported defaults back unchanged stays a valid request.

`crop_zoom` shrinks whatever the origin settled on, both sides by the same factor so the ratio holds. It is what moves an anchor in from the edges: a 1536x2048 photo into `instagram-story` keeps 1152x2048 centred at `crop_zoom=1.0`, and 576x1024 at `crop_zoom=2`, where `top` and `bottom` then differ. A corner does not need it, since a corner already decides where the rectangle starts.

Both POST routes answer with `x-crop-rect: X,Y,WIDTH,HEIGHT`, the part of the upload that was read, in source pixels. It is the whole photo when `crop` is off, so it always reports what the upload measured, which is what a client needs before it can name a corner. It comes from the pipeline's own geometry rather than being worked out again, so it cannot disagree with the image that came back.

### Presets

`preset` names an aspect ratio instead of working the shape out by hand. It does not replace `width` and `height`: the largest rectangle of that ratio that fits inside the pair is what gets dithered, so the preset picks the shape and the pair still picks the scale. `GET /api/options` carries the same table under `presets`, so a picker can be built from the API rather than hardcoded.

| `preset` value | Ratio | Inside the default 600x400 | What it is |
| --- | --- | --- | --- |
| `panel` | 3:2 | 600x400 | The default working shape, and the one `/api/buffer` expects |
| `panel-portrait` | 2:3 | 400x600 | The panel's own portrait layout |
| `instagram-post` | 1:1 | 400x400 | Square post |
| `instagram-portrait` | 4:5 | 400x500 | The tallest post the feed takes |
| `instagram-landscape` | 191:100 | 600x314 | 1.91:1 |
| `instagram-story` | 9:16 | 337x600 | Stories and reels |
| `iphone` | 4:3 | 533x400 | The iPhone's default photo shape |

The pair is turned over first when the ratio disagrees with it, so a portrait ratio is not squeezed into the landscape default's short side: `preset=panel-portrait` against 600x400 is the panel's own 400x600, not 266x400. That is what keeps both panel entries packable by `/api/buffer`.

Since a preset is fitted inside `width` and `height` rather than carrying its own pixel count, asking for more resolution is a matter of asking for a bigger pair: `preset=instagram-story` alone returns 337x600, and `preset=instagram-story&width=1080&height=1080` returns 607x1080 of the same shape. What a request costs therefore follows the pair, not the name. `x-image-size` reports what it landed on.

A preset names one orientation, and `keep_orientation=true` transposes it for a photo of the other one, so `preset=iphone&keep_orientation=true&crop=true` returns 400x533 undistorted. An unknown name is a 400 listing the ones that work.

### Errors

Every failure is JSON, whatever caused it:

```json
{ "error": "saturation must be between 0.0 and 1.0, got 9", "status": 400 }
```

`400` covers bad settings, an unreadable image and a missing body. `413` means the upload was over `MAX_UPLOAD_BYTES`. `500` means the pipeline itself failed.

## Calling it from Svelte

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
  | 'panel'
  | 'panel-portrait'
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

/** The packed frame buffer, ready to push to the panel. */
export const panelBuffer = async (file: File, params: Partial<DitherParams> = {}): Promise<ArrayBuffer> => {
  const response = await send('/api/buffer', file, params);
  return await response.arrayBuffer();
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

Note the `format: 'rgb'` in the preview call. Indexed PNGs are what the panel wants, but some browsers render them with slightly different colour management, so `rgb` is the safer choice for an on-screen preview.

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
```

The API tests in `crates/dither-server/tests/api.rs` drive the router in-process, so they bind no socket and need no running server.

## Credits

This project is inspired by [reframe](https://github.com/kaloyaan/reframe), an e-paper camera. I like the camera but cannot afford one, so I built my own dithering pipeline, and reframe is where the idea came from. The palette and the frame buffer layout follow it, since those have to match the panel they were made for. What grew around them is mine: the framing, the extra kernels, and the CLI and HTTP front ends. Go and look at the camera, it is lovely work.
