#!/usr/bin/env bash
# Builds a transferable deployment; no client MPQs or dev web packages copied.
set -euo pipefail
ROOT=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$ROOT"
IMAGE=${BENILLA_IMAGE:-benilla-web:local}
OUT="$ROOT/dist/production"
TOOLCHAINS="$ROOT/../../.toolchains"
if [[ -x "$TOOLCHAINS/benilla-cargo/bin/cargo" ]]; then
  export PATH="$TOOLCHAINS/benilla-cargo/bin:$PATH"
  export CARGO_HOME=${CARGO_HOME:-"$TOOLCHAINS/benilla-cargo"}
  export RUSTUP_HOME=${RUSTUP_HOME:-"$TOOLCHAINS/benilla-rustup"}
fi
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-"$ROOT/target"}
export CARGO_INCREMENTAL=0
BINDGEN=${WASM_BINDGEN:-"$ROOT/.tools/wasm-bindgen/bin/wasm-bindgen"}
if [[ ! -x "$BINDGEN" ]]; then BINDGEN=$(command -v wasm-bindgen); fi
command -v cargo >/dev/null
command -v python3 >/dev/null
docker info >/dev/null
docker compose version >/dev/null
if [[ -e "$OUT" && ! -f "$OUT/.benilla-production" ]]; then
  echo "Refusing to replace unrecognized directory: $OUT" >&2
  exit 1
fi
mkdir -p "$ROOT/dist"
STAGE=$(mktemp -d "$ROOT/dist/.production-XXXXXXXX")
echo "Production staging: $STAGE"
echo "Building shipping wasm (fat LTO); the first build can take a while."
cargo build --locked --target wasm32-unknown-unknown --profile ship -p benilla --no-default-features
mkdir -p "$STAGE/web/pkg-full"
"$BINDGEN" "$CARGO_TARGET_DIR/wasm32-unknown-unknown/ship/benilla.wasm" \
  --target web --out-dir "$STAGE/web/pkg-full" --out-name benilla --no-typescript \
  --remove-name-section --remove-producers-section
cp web/index.html "$STAGE/web/index.html"
cp web/config.js "$STAGE/web/config.js"
cp web/localization.js "$STAGE/web/localization.js"
mkdir -p "$STAGE/web/pkg-audio"
cargo build --locked --target wasm32-unknown-unknown --profile ship -p benilla-audio-web
"$BINDGEN" "$CARGO_TARGET_DIR/wasm32-unknown-unknown/ship/benilla_audio_web.wasm" \
  --target web --out-dir "$STAGE/web/pkg-audio" --out-name benilla_audio --no-typescript \
  --remove-name-section --remove-producers-section
cp -R web/audio "$STAGE/web/audio"
docker build --build-context "web=$STAGE/web" --file deploy/Dockerfile \
  --build-arg "RUST_IMAGE=${RUST_IMAGE:-rust:1.98.1-bookworm}" --tag "$IMAGE" .
docker save "$IMAGE" | gzip > "$STAGE/image.tar.gz"
cp deploy/compose.yaml deploy/.env.example deploy/README.md deploy/start.sh "$STAGE/"
chmod +x "$STAGE/start.sh"
python3 - "$STAGE" "$IMAGE" <<'PY'
import hashlib, json, pathlib, subprocess, sys
root = pathlib.Path(sys.argv[1])
image = sys.argv[2]
env = root / '.env.example'
env.write_text(env.read_text().replace('BENILLA_IMAGE=benilla-web:local', 'BENILLA_IMAGE=' + image))
manifest = {
    'image': image,
    'image_id': subprocess.check_output(['docker', 'image', 'inspect', image, '--format', '{{.Id}}'], text=True).strip(),
    'source_commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip(),
    'source_dirty': bool(subprocess.check_output(['git', 'status', '--porcelain'], text=True)),
    'wasm_profile': 'ship',
}
(root / '.benilla-production').write_text(json.dumps(manifest, indent=2) + '\n')
with (root / 'SHA256SUMS').open('w') as sums:
    for path in sorted(root.rglob('*')):
        if path.is_file() and path.name != 'SHA256SUMS':
            digest = hashlib.file_digest(path.open('rb'), 'sha256').hexdigest()
            sums.write(f'{digest}  {path.relative_to(root)}\n')
PY
# Keep prior delivery intact until every build/export succeeds; do not delete it.
if [[ -d "$OUT" ]]; then
  BACKUP="$ROOT/dist/production-previous-$(date +%Y%m%d-%H%M%S)"
  mv "$OUT" "$BACKUP"
  echo "Previous delivery retained: $BACKUP"
fi
mv "$STAGE" "$OUT"
echo "Ready: $OUT"
echo 'Copy this directory to the server, edit .env, then ./start.sh.'
