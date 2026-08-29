#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")"

if [[ -f .env ]]; then
	set -a
	source ./.env
	set +a
else
	echo "No .env — copy .env.example and fill it in." >&2
	exit 1
fi

: "${HOST:?HOST is missing from .env}"
: "${DEST:?DEST is missing from .env}"
PUBLIC_URL=${PUBLIC_URL:-/dither/}

command -v trunk >/dev/null || { echo "trunk not found: brew install trunk" >&2; exit 1; }
rustup target add wasm32-unknown-unknown

cd crates/dither-app
trunk build --release --public-url "$PUBLIC_URL"

rsync -az --delete --chmod=D755,F644 dist/ "$HOST:$DEST/"
