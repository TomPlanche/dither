# port

A Cargo workspace around the reframe e-paper dithering pipeline: the pipeline itself, and an HTTP backend that puts it behind a few routes for a Svelte front end.

```
crates/
  reframe-dither/   library + CLI: the 6-colour dithering pipeline
  dither-server/    library + binary: the HTTP backend
```

`dither-server` is stateless. It takes an uploaded photo, runs it through `reframe-dither`, and hands back either a dithered PNG or the packed e-paper frame buffer. Nothing is written to disk and nothing is kept between requests.

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
  "methods": ["floyd-steinberg", "atkinson", "stucki", "burkes", "jarvis", "ordered"],
  "formats": ["indexed", "rgb"],
  "bayer_sizes": [2, 4, 8],
  "presets": [{ "name": "panel", "size": [600, 400] }, { "name": "instagram-story", "size": [1080, 1920] }, "..."],
  "defaults": { "method": "floyd-steinberg", "saturation": 0.6, "brightness": 1.1, "color": 1.4, "bayer_size": 4, "threshold_scale": 1.0, "width": 600, "height": 400, "resize": true, "keep_orientation": false, "crop": false, "scale": 1, "format": "indexed" },
  "limits": { "max_upload_bytes": 26214400, "max_dimension": 4096, "max_scale": 4, "max_source_pixels": 50000000 },
  "panel": { "image_size": [600, 400], "panel_size": [400, 600], "palette": ["#221c22", "#ffffff", "#e2d82a", "#c32b2d", "#221c22", "#004cff", "#189c22"] }
}
```

The palette is the blend at the default saturation, ready to drop into CSS.

### `POST /api/dither`

Returns the dithered image as `image/png`, with an `x-image-size` header carrying `WIDTHxHEIGHT`.

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
| `method` | `floyd-steinberg` | `floyd-steinberg`, `atkinson`, `stucki`, `burkes`, `jarvis`, `ordered` |
| `saturation` | `0.6` | `0.0` to `1.0`. Blends between the muted and the pure panel palettes. |
| `brightness` | `1.1` | `0.0` to `5.0`. Applied before dithering. |
| `color` | `1.4` | `0.0` to `5.0`. Applied before dithering, after brightness. |
| `bayer_size` | `4` | `2`, `4` or `8`. Ordered dithering only. |
| `threshold_scale` | `1.0` | `0.0` to `5.0`. Ordered dithering only. |
| `width` | `600` | `1` to `4096` |
| `height` | `400` | `1` to `4096` |
| `preset` | none | A named working size, which takes `width` and `height`'s place. See the table below. |
| `resize` | `true` | `false` dithers at the source resolution instead. |
| `keep_orientation` | `false` | `true` transposes `width`x`height` for a photo that disagrees with it, so a portrait upload stays portrait. |
| `crop` | `false` | `true` crops to `width`x`height`'s aspect ratio instead of stretching the photo into it. |
| `crop_from` | `center` | Which part the crop keeps: `center`, `top`, `bottom`, `left`, `right`, or a corner as `X,Y`. Needs `crop=true`. |
| `scale` | `1` | `1` to `4`. Nearest-neighbour upscale of the result. |
| `format` | `indexed` | `indexed` for a palette PNG, `rgb` for a plain one. |

An unknown parameter is an error rather than a silent no-op, so a typo shows up immediately.

`keep_orientation` and `crop` are what an upload of any shape needs to come out undistorted: the first picks the panel layout the photo is closer to, the second trims the long side rather than stretching the short one. Neither changes the size that comes back, so a client can keep reading it off `x-image-size`.

`crop_from` says which part the crop keeps. The kept rectangle's size is fixed by the working size's ratio, so what is left to choose is where it sits, and only along the one axis that has any slack: `top` on a photo that is losing its sides does the same as `center`. A corner past what the photo can offer settles against the far edge rather than failing, because how much slack there is depends on the upload's own dimensions, which the caller does not always know.

Two spellings are a 400 rather than a silent centre crop: one that is neither an anchor nor `X,Y`, and `crop_from` without `crop=true`, which would have nothing to place. That is why `crop_from` is absent from `defaults` when unset, so posting the reported defaults back unchanged stays a valid request.

### Presets

`preset` names a working size instead of spelling out `width` and `height`. `GET /api/options` carries the same table under `presets`, so a picker can be built from the API rather than hardcoded.

| `preset` value | Size | What it is |
| --- | --- | --- |
| `panel` | 600x400 | The default working size, and the one `/api/buffer` expects |
| `panel-portrait` | 400x600 | The panel's own portrait layout |
| `instagram-post` | 1080x1080 | Square post |
| `instagram-portrait` | 1080x1350 | 4:5, the tallest post the feed takes |
| `instagram-landscape` | 1080x566 | 1.91:1 |
| `instagram-story` | 1080x1920 | 9:16, stories and reels |
| `iphone` | 4032x3024 | 4:3, the iPhone's default photo size |

A preset names one orientation, and `keep_orientation=true` transposes it for a photo of the other one, so `preset=iphone&keep_orientation=true&crop=true` returns 3024x4032 undistorted. An unknown name is a 400 listing the ones that work. The size asked for is the size dithered at, so the large presets cost real time: about 750 ms for `iphone` against 60 ms for `panel`. Only the two panel entries can be packed by `/api/buffer`.

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
  preset: DitherPreset;
  resize: boolean;
  keep_orientation: boolean;
  crop: boolean;
  /** Needs `crop: true`. 'center' | 'top' | 'bottom' | 'left' | 'right', or a corner as `${number},${number}` */
  crop_from: string;
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

Note the `format: 'rgb'` in the preview call. Indexed PNGs are what the camera saves and what you want for the panel, but some browsers render them with slightly different colour management, so `rgb` is the safer choice for an on-screen preview.

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
cargo run -p reframe-dither -- photo.jpg   # the CLI
```

The API tests in `crates/dither-server/tests/api.rs` drive the router in-process, so they bind no socket and need no running server.

## Credits

This project takes up the principle of [reframe](https://github.com/kaloyaan/reframe), an e-paper camera. I like the camera but cannot afford one, so I rebuilt its dithering algorithm in Rust for my own needs. The design, the palette handling and the processing pipeline are theirs; everything here is a port of that work, not a new idea.
