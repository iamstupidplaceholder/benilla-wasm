//! Browser output adapter: eagerly encode/decode cursor images, then change CSS
//! synchronously. Selection/compositing remain in the original CursorPlugin.
use bevy::asset::AssetId;
use bevy::window::{CursorOptions, PrimaryWindow};
use bevy::{app::MainScheduleOrder, ecs::schedule::ScheduleLabel, prelude::*};
use wasm_bindgen::prelude::*;

#[wasm_bindgen(inline_js = r#"
const cursors = new Map();
const persistent = globalThis.BENILLA_CONFIG?.persistentPointer === true;
let worldActive = false;
let looking = false;
let x = 0, y = 0;
let initialized = false;
let requesting = null;
let releasing = false;
let wantLock = false;
let overlay = null;
function gameCanvas() { return document.getElementById('benilla'); }
function isLocked() { return document.pointerLockElement === gameCanvas(); }
function reconcileLock(source) {
    const canvas = gameCanvas();
    if (!canvas) return;
    if (!wantLock) {
        if (isLocked() && !releasing) {
            releasing = true;
            document.exitPointerLock();
        }
        return;
    }
    // Browser truth, not CursorOptions' desired mode. Never duplicate an
    // asynchronous request or race a pending unlock with a new request.
    if (isLocked() || requesting || releasing || document.pointerLockElement) return;
    const token = {};
    requesting = token;
    try {
        const request = canvas.requestPointerLock();
        if (request?.then) request.then(() => {
            if (requesting === token) requesting = null;
            // Only undo an obsolete acquisition here. Do not auto-retry a
            // successful request after Escape/blur has already unlocked it.
            if (!wantLock) reconcileLock('request-resolved');
        }, error => {
            if (requesting === token) requesting = null;
            console.warn('pointer lock failed', error);
        });
    } catch (error) {
        if (requesting === token) requesting = null;
        console.warn('pointer lock failed', error);
    }
}
function placeOverlay() {
    if (!overlay) return;
    const rect = gameCanvas().getBoundingClientRect();
    overlay.style.transform = `translate(${rect.left + x}px, ${rect.top + y}px)`;
}
function installPointer() {
    if (initialized) return;
    const canvas = gameCanvas();
    if (!canvas) return;
    initialized = true;
    if (persistent) {
        overlay = document.createElement('img');
        overlay.alt = '';
        Object.assign(overlay.style, { position: 'fixed', left: '0', top: '0', width: '32px', height: '32px',
            pointerEvents: 'none', zIndex: '2147483647', display: 'none' });
        document.body.appendChild(overlay);
    }
    canvas.addEventListener('pointerdown', event => {
        if (!persistent || !worldActive || !event.isTrusted || isLocked() || requesting || releasing) return;
        const rect = canvas.getBoundingClientRect();
        x = Math.max(0, Math.min(rect.width - 1, event.clientX - rect.left));
        y = Math.max(0, Math.min(rect.height - 1, event.clientY - rect.top));
        wantLock = true;
        reconcileLock('persistent-pointerdown');
    });
    window.addEventListener('blur', () => {
        wantLock = false;
        reconcileLock('blur');
    });
    document.addEventListener('pointerlockchange', () => {
        const wasReleasing = releasing;
        requesting = null;
        releasing = false;
        if (!isLocked()) {
            if (overlay) overlay.style.display = 'none';
            // External Escape/tab unlock is not permission to reacquire.
            if (!wasReleasing) wantLock = false;
        }
        reconcileLock('pointerlockchange');
        // Escape/tab-switch unlock is respected. Only a fresh trusted click may reacquire.
    });
    document.addEventListener('pointerlockerror', () => {
        requesting = null;
    });
    document.addEventListener('mousemove', event => {
        if (!persistent || !isLocked() || !worldActive) return;
        const rect = canvas.getBoundingClientRect();
        if (looking) return;
        x = Math.max(0, Math.min(rect.width - 1, x + event.movementX));
        y = Math.max(0, Math.min(rect.height - 1, y + event.movementY));
        placeOverlay();
    });
}
export function setPointerGrab(lock) {
    installPointer();
    if (persistent) return;
    if (wantLock !== lock) {
        wantLock = lock;
    }
    reconcileLock('camera');
}
export function persistentEnabled() { return persistent; }
export function virtualActive() { return persistent && worldActive && isLocked(); }
export function virtualX() { return x / Math.max(gameCanvas().getBoundingClientRect().width, 1); }
export function virtualY() { return y / Math.max(gameCanvas().getBoundingClientRect().height, 1); }
export function setWorldActive(active) {
    installPointer();
    worldActive = active;
    if (!active && wantLock) {
        wantLock = false;
        reconcileLock('world-inactive');
    }
}
export function prepareCursor(key, width, height, rgba) {
    if (cursors.has(key)) return;
    const entry = { ready: false, error: null, css: null, image: null };
    cursors.set(key, entry);
    try {
        const canvas = document.createElement('canvas');
        canvas.width = width;
        canvas.height = height;
        const context = canvas.getContext('2d');
        if (!context) throw new Error('cursor canvas 2D context unavailable');
        context.putImageData(new ImageData(new Uint8ClampedArray(rgba), width, height), 0, 0);
        const url = canvas.toDataURL('image/png');
        const image = new Image();
        entry.image = image;
        image.src = url;
        image.decode().then(() => {
            entry.css = `url("${url}") 0 0, default`;
            entry.ready = true;
        }).catch(error => { entry.error = String(error); });
    } catch (error) { entry.error = String(error); }
}
export function cursorError(key) {
    const entry = cursors.get(key);
    if (!entry || !entry.error) return null;
    const error = entry.error;
    entry.error = null;
    return error;
}
export function displayCursor(key, visible) {
    const canvas = document.getElementById('benilla');
    if (!canvas) return;
    const entry = cursors.get(key);
    looking = !visible;
    if (persistent && isLocked()) {
        canvas.style.cursor = 'none';
        if (entry && entry.ready) {
            if (overlay.src !== entry.image.src) overlay.src = entry.image.src;
            overlay.style.display = visible ? 'block' : 'none';
            placeOverlay();
        } else if (!visible) overlay.style.display = 'none';
        return;
    }
    if (overlay) overlay.style.display = 'none';
    if (!visible) {
        if (canvas.style.cursor !== 'none') canvas.style.cursor = 'none';
    } else if (entry && entry.ready && canvas.style.cursor !== entry.css) {
        canvas.style.cursor = entry.css;
    }
}
"#)]
extern "C" {
    #[wasm_bindgen(js_name = prepareCursor)]
    fn prepare_cursor(key: &str, width: u32, height: u32, rgba: &[u8]);
    #[wasm_bindgen(js_name = cursorError)]
    fn cursor_error(key: &str) -> JsValue;
    #[wasm_bindgen(js_name = displayCursor)]
    fn display_cursor(key: &str, visible: bool);
    #[wasm_bindgen(js_name = persistentEnabled)]
    pub(super) fn persistent_enabled() -> bool;
    #[wasm_bindgen(js_name = virtualActive)]
    fn virtual_active() -> bool;
    #[wasm_bindgen(js_name = virtualX)]
    fn virtual_x() -> f32;
    #[wasm_bindgen(js_name = virtualY)]
    fn virtual_y() -> f32;
    #[wasm_bindgen(js_name = setWorldActive)]
    fn set_world_active(active: bool);
    #[wasm_bindgen(js_name = setPointerGrab)]
    pub(super) fn set_pointer_grab(lock: bool);
}

pub(super) fn virtual_position(window: &bevy::window::Window) -> Option<Vec2> {
    virtual_active().then(|| Vec2::new(virtual_x() * window.width(), virtual_y() * window.height()))
}

#[derive(Component)]
pub(super) struct BrowserCursor(pub Handle<Image>);

#[derive(ScheduleLabel, Debug, Clone, PartialEq, Eq, Hash)]
struct CursorOutput;

pub(super) fn register(app: &mut App) {
    app.init_schedule(CursorOutput);
    app.world_mut()
        .resource_mut::<MainScheduleOrder>()
        .insert_after(Last, CursorOutput);
    // Run after winit's visibility changes, reading the original controller's
    // final CursorOptions. Experimental locking remains in this host adapter;
    // original controller/FrameXML still own look and UI behavior.
    app.add_systems(CursorOutput, (preload, display).chain());
}

fn preload(
    modes: Res<super::other::CursorImages>,
    payloads: Res<super::other::PayloadCursorImages>,
    images: Res<Assets<Image>>,
    mut started: Local<std::collections::HashSet<AssetId<Image>>>,
) {
    for handle in modes.0.values().chain(payloads.0.values()) {
        if !started.contains(&handle.id()) {
            let Some(image) = images.get(handle) else {
                continue;
            };
            let Some(rgba) = image.data.as_deref() else {
                continue;
            };
            prepare_cursor(
                &format!("{:?}", handle.id()),
                image.width(),
                image.height(),
                rgba,
            );
            started.insert(handle.id());
        }
        if let Some(error) = cursor_error(&format!("{:?}", handle.id())).as_string() {
            warn!("browser cursor preload failed: {error}");
        }
    }
}

fn display(
    window: Option<Single<(&BrowserCursor, &CursorOptions), With<PrimaryWindow>>>,
    player: Option<Res<crate::player::Player>>,
) {
    set_world_active(player.as_deref().is_some_and(|p| p.active && !p.settling));
    let Some(window) = window else {
        return;
    };
    let (cursor, options) = *window;
    display_cursor(&format!("{:?}", cursor.0.id()), options.visible);
}
