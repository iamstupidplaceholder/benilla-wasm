# Benilla production bundle

The image contains the optimized original wasm client and gateway. WoW assets
are **not** redistributed: mount your installed 1.12.1 client read-only.

## Build / update on the development machine

```sh
./scripts/build-production.sh
```

Requires Docker (BuildKit / named build contexts), Compose, Python 3.11+, the
project Rust toolchain with `wasm32-unknown-unknown`, and wasm-bindgen CLI **0.2.122**
(`.tools/wasm-bindgen/bin/wasm-bindgen` or PATH; override with `WASM_BINDGEN`).
Wasm uses the existing `ship` profile (opt-level 3, fat LTO, no default features).
Shipping bindgen output omits debugging symbol names and producer telemetry.
The script also packages the small `benilla-audio-web` wasm module, its decode/mix
Workers and AudioWorklet. Browser audio needs HTTPS (localhost is exempt) and a
trusted click/key to unlock. Original UI/world effects, glue/zone/cinematic music,
ambience and reverb are delivered through the browser audio adapters; music
streams bounded PCM blocks rather than decoding whole tracks to PCM.
Gateway builds inside the Rust Docker builder, avoiding host glibc dependencies.
`RUST_IMAGE` can override the pinned builder; `BENILLA_IMAGE` overrides the tag.
The runtime and image architecture are those of the build machine; the destination
must match (default on this machine: Linux amd64). No CPU-native instructions used.

Output is `dist/production/`: `web/` is the separate production package,
`image.tar.gz` is the ready image, plus Compose, launcher, env template,
SHA256SUMS and build metadata. No dev `web/pkg-full` is overwritten. A previous
successful output is renamed, not deleted; failed staging directories are retained
for diagnosis. Source may be dirty; metadata records this rather than pretending
the bundle is a clean commit. The image does not contain the source or MPQs.

## Run on the WoW server machine

Copy **the entire output directory**; no Rust, source checkout or build required:

```sh
cd production
sha256sum -c SHA256SUMS
cp .env.example .env
# Edit .env: at minimum WOW_CLIENT_DIR=/absolute/path/to/WoW
./start.sh
docker compose logs -f
```

Open `http://SERVER:8000/`. The root is the full client, not the
old probe. UID 10001 in the container must be able to read/traverse the mounted
directory and MPQs. `Data` spelling is detected case-insensitively at startup.
Mounting a missing path fails instead of creating an empty client directory.

### Addresses

* `BENILLA_PUBLIC_URL`: the gateway HTTP(S) base the **browser** can reach. Empty
  uses the page origin. HTTPS/proxy example: `https://wow.example.org/benilla`.
  With a path prefix, the proxy must strip that prefix before forwarding.
* `BENILLA_HOST`: optional common host. With no PUBLIC_URL, generates
  `http://HOST:BENILLA_PORT`; also selects the upstream host unless overridden.
* `UPSTREAM_HOST`: server host reachable **from the gateway container**. The
  template uses `host.docker.internal` for auth/world ports published on the host.
  To use the common BENILLA_HOST for both roles, clear UPSTREAM_HOST in `.env`.
* `AUTH_PORT=3724`, `WORLD_PORT=8085`: published WoW TCP ports. Optional
  `AUTH_UPSTREAM=host:port` / `WORLD_UPSTREAM=host:port` override them separately.
  DNS is resolved at startup; restart after server IP changes.
* `BENILLA_PORT=8000`: host HTTP port; container always listens on 8000.
  `BENILLA_BIND_IP=127.0.0.1` limits publishing to a local reverse proxy.

`127.0.0.1` as upstream means **this gateway container**, not the host or the WoW
container. Docker's host-gateway mapping is included for Linux. The WoW ports
must be published on an interface reachable from the gateway bridge, not only
host loopback. Alternatively use a reachable server IP, or join a common Docker
network and set UPSTREAM_HOST to the WoW service name. The public URL never
changes which server the client is permitted to select: fixed gateway destinations
remain server-owned. Realm-list addresses do not override them.

HTTPS is recommended for remote deployment (browser GPU/security requirements).
Terminate TLS at your reverse proxy and forward both HTTP and WebSocket upgrades.
The gateway has no TLS/authentication/rate-limiting layer: do not treat this image
as a hardened public service. Firewall or protect it as appropriate.

## Easy updates

Build again, copy the new bundle, retain your server's `.env`, then run
`./start.sh`. It loads the exported image and recreates the service, so an unchanged
image tag still deploys the new version. Environment-only changes also need a
recreate (`docker compose up -d --force-recreate`), but no Rust build.
Hard-reload the browser afterwards. `docker compose down` stops this deployment;
it does not delete the mounted client or affect the WoW server container.

For an existing deployment, copy the bundle contents into its existing directory
without deleting or replacing `.env` (and preserve any customized Compose file).
Then, from that directory:

```sh
sha256sum -c SHA256SUMS
./start.sh
docker compose logs --tail=100 benilla
```

No `docker compose down` is required. The launcher loads the new image and
recreates the Benilla service; a plain `docker restart` would retain the old
container image. If Compose was customized, checksum differences for that file
are expected and should be reviewed rather than overwritten blindly.
