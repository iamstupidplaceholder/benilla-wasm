# benilla-wasm

A WebAssembly browser preview of [benilla](https://github.com/samwhosung/benilla),
the Rust/Bevy World of Warcraft 1.12.1 client.

This is a standalone source snapshot, not the development repository. It includes
the browser client, shared protocol and asset readers, and the asset/WebSocket
gateway. Native compilation remains available. Mobile support is not included.

## What is included

- Browser login, character selection and the streamed game world.
- The shared Lua/FrameXML interface, rendering, movement and browser audio.
- A Rust gateway for installed-client assets and auth/world WebSocket relays.
- English and Russian locale selection, including the pre-login loading screen.

The browser talks to the gateway; the gateway connects to your configured vanilla
server. Game data stays in your installed client and is not bundled into the WASM.

## Requirements

- A legally obtained WoW 1.12.1 (build 5875) installation.
- A compatible vanilla server, such as vmangos.
- Rust with the `wasm32-unknown-unknown` target.
- `wasm-bindgen-cli` matching the version in `Cargo.lock`.
- Docker with Compose for the packaged deployment.

No game assets, client installation, account credentials or server data are
included. The gateway reads your own installed client at runtime.

## Build

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.122 --locked
cargo build --locked --target wasm32-unknown-unknown --profile ship -p benilla --no-default-features
wasm-bindgen target/wasm32-unknown-unknown/ship/benilla.wasm \
  --target web --out-dir web/pkg-full --out-name benilla --no-typescript
```

The complete deployment builder also packages browser audio and the gateway:

```sh
bash scripts/build-production.sh
```

See [deployment instructions](deploy/README.md) for configuration. Edit a local
`.env` based on `deploy/.env.example`; never commit real credentials or client data.
The build script packages an image but does not start or restart a server.

## Language

Select English (`enUS`) or Russian (`ruRU`) at the login screen. The choice is
saved locally; changing it reloads the page so asset requests, interface resources
and authentication use the same locale. Available choices depend on the language
data in the client installation served by the gateway.

Browser-owned loading messages live in [`web/localization.js`](web/localization.js).
They use the saved locale from the beginning of page loading, before the game UI
exists. Resource paths and filenames are displayed unchanged. Original in-game
translations continue to come from the selected installed-client resources.

## Status

This is an experimental preview, not an official Blizzard client. Expect bugs
and browser/GPU compatibility limitations. Use trusted servers and expose the
gateway only with appropriate access controls and HTTPS.

Desktop browsers are the current target. Mobile play and touch controls are not
supported in this snapshot. There is no compatibility promise for every addon,
server variant or browser/GPU combination.

Development panels, capture/probe tools and browser trace/watch APIs are not part
of this preview. Normal error reporting and the original in-game FPS display are
retained.

## License and attribution

Based on benilla by sam <samuv.eth@gmail.com>, with browser-port changes. Original
copyright notices and third-party licenses are retained. Project code is available
under [MIT](LICENSE-MIT) or [Apache 2.0](LICENSE-APACHE), at your option; vendored
dependencies retain their respective licenses.

World of Warcraft is a trademark of Blizzard Entertainment. This independent fan
project is not affiliated with or endorsed by Blizzard Entertainment.
