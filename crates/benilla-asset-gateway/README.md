# Benilla gateway

For a transferable production Docker image (gateway + shipping wasm), see
[`deploy/README.md`](../../deploy/README.md) and run `./scripts/build-production.sh`.
The destination only needs Docker/Compose and a read-only mount of its WoW client.

Serves browser packages, logical game assets from the original MPQ patch chain,
and same-origin WebSocket relays to the configured auth/world TCP servers.

## One-command startup

Copy `gateway.toml.example` to `gateway.toml` in the repository root and edit
`data`, `auth_upstream`, and `world_upstream`. Then launch from that root:

```sh
cargo run -p benilla-asset-gateway
```

For an already-built binary: `./target/debug/benilla-asset-gateway`.
Open <http://127.0.0.1:8000/> for the full original client.

Settings precedence: **explicit CLI flags > TOML > built-in defaults**.
Without a config, existing CLI and `WOW_DATA`/installed-client discovery still work.
Use `--config /path/to/settings.toml` to select another file. A missing explicit
file, malformed TOML, invalid endpoint, or unknown key is a startup error.
Paths are relative to the launch directory, not the config's directory.
Endpoints are IP:port, including bracketed IPv6; DNS names are not accepted.

Successful game-asset responses are fresh in the browser HTTP cache for
`asset_max_age_secs = 300` (five minutes) by default, then revalidated with ETag.
Set this to `0` when editing MPQs to preserve per-request revalidation. During
the freshness window an ordinary reconnect need not contact the gateway for
cached bytes. Hard reload or DevTools “Disable cache” can bypass HTTP caching;
after replacing game archives, hard-reload and restart the gateway. Caching bytes
does not remove model decoding, GPU upload or scene/collider assembly costs.
Gateway `304` means browser-cache revalidation; `200 HIT` means gateway RAM-cache
hit with a full response body, not a browser cache hit. The browser client's
separate 64-MiB RAM byte cache is bounded and can evict world assets.

The default bind is loopback. Auth/world TCP destinations are always fixed by
`auth_upstream` / `world_upstream` in gateway config (or CLI). Client upstream
selection has been removed; game-route query parameters are rejected with 400.
Remove obsolete `client_upstream` / `allow_upstream` keys from older TOML files.
This is an unauthenticated development gateway, not a public reverse proxy;
do not expose it blindly. Read-only HTTP responses allow cross-origin access.

## Browser deployment configuration

Edit `web/config.js`:

```js
globalThis.BENILLA_CONFIG = { gateway: "https://gateway.example" };
```

Empty `gateway` uses the page's origin. An HTTP(S) base URL with a reverse-proxy
path prefix is also supported. `index.html` derives all asset/presence HTTP and
auth/world WebSocket routes from this base before starting wasm. Reload the page
after editing; no Rust rebuild is needed. This URL selects a **gateway**, never
the gateway's upstream server. HTTPS pages need an HTTPS gateway to avoid browser
mixed-content blocking; remote WebGL/WebGPU deployment may require HTTPS too.
Browser screenshots are explicitly disabled, and the login realmlist control is
not present in wasm. Native UI and screenshot behavior are unchanged.

`web/config.js` also exposes the temporary `persistentPointer` experiment. It is
currently `true` for evaluation: a trusted in-world click acquires pointer lock,
the original cursor art is shown at a virtual pointer, and releasing camera-look
buttons keeps the lock. **Esc always releases browser lock**; another click may
reacquire. Set `persistentPointer: false` and reload to restore the prior CSS
cursor/press-release locking without rebuilding. Native controls are unchanged.

## Browser audio (first admission)

Original Lua `PlaySound`/`PlaySoundFile` and glue-click cues now feed the original
SoundEntries selection/variation and Kira renderer. A small separate wasm module
(`web/pkg-audio`) runs in decode/mix Workers; `web/audio/output-worklet.js` only
copies prepared PCM to the browser output. UI/GlueXML 2D SFX is the admitted scope;
world SFX and music/streaming were initially pending. Native device/streaming is unchanged.

The next development admission also routes the original `sound/spell.rs`,
`footsteps.rs` and `combat.rs` through this output: actual catalog delivery, WoW
distance math and Kira spatial tracks, listener updates, tracked spell loops and
pending cancellation. Greetings/death/pet/server-object sounds and other world
trigger families, music/ambience/reverb/streaming remain pending. UI audio is
confirmed by the user with zero reported underruns; world hearing still needs
runtime verification. Production refresh is paused by user request.

Audio unlocks on a trusted click/key, needs HTTPS remotely, and retains honest
awaiting-gesture/suspended/failed states. Set `audio: false` in `web/config.js` to
open no audio device, then reload. `globalThis.BENILLA_AUDIO.status()` and `.health`
expose output state/underruns; `.decodes` counts actual successful SFX decodes.
Original sound CVars control master/SFX gain, master enable, limiter and background
gate. Original MPQ audio bytes use the shared bounded asset delivery policy;
decoding uses the same MP3/WAV PCM/ADPCM codecs as native, not browser decodeAudioData.
No file transcoding, invented paths or success on a missing file. Pending first-use
requests expire after 1.5s rather than replaying stale clicks after delivery.

Host admission caps: 32 outstanding plays/file reads, 8 decoding jobs, 8 MiB
compressed per SFX, 16 MiB PCM per file, 64 MiB worker decoded-cache budget,
12 playing voices and 8×512-frame stereo output blocks. The first host backend
refuses excess voices rather than claiming the native voice-stealing policy is
already ported. Session/logout clears pending plays and fades old live voices;
native streamed music is not replaced with whole-track decoded PCM.

## Macro icon metadata

`/assets/v1/macro-icons.bfi` delivers the original archive-enumerated macro icon
list as UTF-8 texture paths separated by newlines. The gateway uses the same
native `load_macro_icons` filter/order/dedup, not a SpellIcon.dbc approximation.
Only metadata is prefetched; icon artwork loads through the usual asset route.
ETag, HEAD and cross-origin headers follow the presence metadata endpoint.
Restart the updated gateway, then hard-reload the browser for this new route.

## Bounded browser asset delivery

Edit `assetDelivery` in `web/config.js`, then reload the page:

```js
assetDelivery: {
  attempts: 3,             // Total attempts, including the first
  attemptTimeoutMs: 10000, // Headers and body together
  totalTimeoutMs: 35000,   // One logical read, including backoff
  retryDelayMs: 250,       // Doubles per retry, capped at 10 seconds
}
```

One browser HTTP adapter applies this to MPQ files and gateway metadata. Network
faults, stalled bodies, HTTP 408/429 and server 5xx are retried within the budget.
404 and other terminal 4xx, invalid logical paths and decoder errors are not
retried. Exhaustion remains a real failed asset, not a placeholder or successful
empty body. Values are clamped: 1–8 attempts, 100–120000 ms attempt timeout,
100–180000 ms total timeout and 0–10000 ms base delay. No gateway restart or Rust
rebuild is required to change this policy. Native file IO is unchanged.

## Case-insensitive game-file lookup

`/assets/v1/file/Interface/Icons/...` and the lowercase spelling refer to the
same asset and cache entry. The common logical-path normalizer folds ASCII case
and treats `/` and `\` alike; MPQ name hashing does the same. Archive mounting
also matches names such as `texture.MPQ` and `TEXTURE.mpq` case-insensitively,
while retaining original patch priority and delete markers.

The configured **host** path to Data must exist with its actual OS spelling;
static web-package paths retain normal filesystem case semantics. Neither is
a logical MPQ path. Absolute game paths, drive prefixes, and traversal remain
rejected; there is no fallback to arbitrary host files or guessed texture paths.
