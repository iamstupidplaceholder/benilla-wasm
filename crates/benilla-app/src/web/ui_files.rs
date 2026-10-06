//! Browser host delivery for the production synchronous interface-file reader.
//! Publish only after every requested install file and XML dependency is resident.

use crate::ui_files::UiFilePlan;
use benilla_assets::RawFile;
use bevy::{asset::LoadState, prelude::*};
use std::collections::HashMap;

#[derive(Resource, Default, Debug)]
pub(crate) enum UiFilesState {
    #[default]
    Loading,
    Ready,
    Failed(String),
}

pub(crate) fn entry_wait_status(world: &World) -> String {
    let state = world.get_resource::<UiFilesState>();
    if let Some(UiFilesState::Failed(reason)) = state {
        return format!("state=Failed, reason={reason}");
    }
    let Some(delivery) = world.get_resource::<Delivery>() else {
        return format!("state={state:?}, no active delivery");
    };
    let server = world.resource::<AssetServer>();
    let pending: Vec<_> = delivery.plan.pending().collect();
    let mut sample = pending.clone();
    sample.sort_unstable();
    let sample: Vec<_> = sample
        .into_iter()
        .take(5)
        .map(|path| {
            (
                path,
                delivery
                    .requests
                    .get(path)
                    .and_then(|h| server.get_load_state(h.id())),
            )
        })
        .collect();
    format!(
        "state={state:?}, presence_ready={}, presence={:?}, addon_catalog_ready={}, addon_catalog={:?}, pending={}, sample={sample:?}",
        delivery.plan.presence_ready(),
        server.get_load_state(delivery.presence_request.id()),
        delivery.addons.is_some(),
        server.get_load_state(delivery.addons_request.id()),
        pending.len()
    )
}

#[derive(Resource)]
struct Delivery {
    started: benilla_time::Instant,
    presence_request: Handle<RawFile>,
    addons_request: Handle<RawFile>,
    addons: Option<benilla_interface_files::AddonCatalog>,
    plan: UiFilePlan,
    requests: HashMap<String, Handle<RawFile>>,
}

pub(crate) struct UiFilesPlugin;

impl Plugin for UiFilesPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, deliver_dynamic_fonts);
        app.init_resource::<UiFilesState>().add_systems(
            Update,
            (begin.run_if(needs_begin), deliver)
                .chain()
                .run_if(crate::loading_screen::startup_presented)
                .before(crate::ui_text::FontInit),
        );
    }
}

fn deliver_dynamic_fonts(
    server: Res<AssetServer>,
    files: Res<Assets<RawFile>>,
    mut requests: Local<HashMap<String, Handle<RawFile>>>,
) {
    for path in benilla_interface_files::take_font_requests() {
        let handle = server.load(format!("mpq://{}", path.replace('\\', "/")));
        requests.insert(path, handle);
    }
    requests.retain(|path, handle| {
        if let Some(file) = files.get(&*handle) {
            benilla_interface_files::finish_font(path, Some(file.0.clone()));
            false
        } else if let Some(LoadState::Failed(error)) = server.get_load_state(handle.id()) {
            warn!("ui_text: dynamic font delivery failed for {path}: {error}");
            benilla_interface_files::finish_font(path, None);
            false
        } else {
            true
        }
    });
}

fn needs_begin(state: Res<UiFilesState>, delivery: Option<Res<Delivery>>) -> bool {
    matches!(*state, UiFilesState::Loading) && delivery.is_none()
}

fn begin(mut commands: Commands, server: Res<AssetServer>, mut state: ResMut<UiFilesState>) {
    let started = benilla_time::Instant::now();
    // The production manifest is the only root list. This is byte admission,
    // not a second manifest loader: execution still belongs to addons/manifest.
    let toc = benilla_ui::toc::Toc::parse(include_str!("../../assets/ui/benilla.toc"));
    let mut plan = UiFilePlan::new(
        toc.files
            .iter()
            .cloned()
            .filter(|path| benilla_interface_files::is_install_path(path))
            .chain(
                crate::ui_text::client_font_files()
                    .iter()
                    .map(|path| (*path).to_owned()),
            ),
    );
    if let Err(error) = plan.discover_builtin(
        toc.files
            .into_iter()
            .filter(|path| !benilla_interface_files::is_install_path(path)),
    ) {
        error!("ui_files: builtin art discovery failed; interface will not be published: {error}");
        *state = UiFilesState::Failed(format!("builtin art discovery: {error}"));
        return;
    }
    commands.insert_resource(Delivery {
        started,
        presence_request: server.load("mpqmeta://presence.bfi"),
        addons_request: server.load("mpqmeta://addons.bai"),
        addons: None,
        plan,
        requests: HashMap::new(),
    });
    info!(
        "startup cost: interface plan/builtin discovery {:.1}ms",
        started.elapsed().as_secs_f64() * 1000.0
    );
}

fn deliver(
    mut commands: Commands,
    server: Res<AssetServer>,
    assets: Res<Assets<RawFile>>,
    delivery: Option<ResMut<Delivery>>,
    mut state: ResMut<UiFilesState>,
    mut timing: Local<(f64, f64, u64)>,
) {
    let Some(mut delivery) = delivery else { return };
    let started = benilla_time::Instant::now();
    if delivery.addons.is_none() {
        let result = if let Some(file) = assets.get(&delivery.addons_request) {
            Some(benilla_interface_files::AddonCatalog::decode(&file.0).map_err(|e| e.to_string()))
        } else if let Some(LoadState::Failed(error)) =
            server.get_load_state(delivery.addons_request.id())
        {
            Some(Err(error.to_string()))
        } else {
            None
        };
        match result {
            Some(Ok(catalog)) => {
                info!(
                    "ui_files: loose addon catalog delivered — {} addons, {} files",
                    catalog.manifests().count(),
                    catalog.files().count()
                );
                delivery.plan.accept_addons(catalog.clone());
                delivery.addons = Some(catalog);
            }
            Some(Err(error)) => {
                error!("ui_files: addon catalog delivery failed; interface will not be published: {error}");
                *state = UiFilesState::Failed(format!("addon catalog delivery: {error}"));
                commands.remove_resource::<Delivery>();
                return;
            }
            None => return,
        }
    }
    if !delivery.plan.presence_ready() {
        let result = if let Some(file) = assets.get(&delivery.presence_request) {
            Some(delivery.plan.accept_presence(&file.0))
        } else if let Some(LoadState::Failed(error)) =
            server.get_load_state(delivery.presence_request.id())
        {
            Some(Err(error.to_string()))
        } else {
            None
        };
        match result {
            Some(Ok(())) => {
                // Bytes are resident before the registry probes these TOCs;
                // execution still waits for the original LoadAddOn path.
                delivery.plan.request_present(
                    crate::ui_script::addons::BLIZZARD_ADDONS
                        .iter()
                        .map(|name| format!("Interface/AddOns/{name}/{name}.toc")),
                );
            }
            Some(Err(error)) => {
                error!(
                    "ui_files: presence delivery failed; interface will not be published: {error}"
                );
                *state = UiFilesState::Failed(format!("presence delivery: {error}"));
                commands.remove_resource::<Delivery>();
                return;
            }
            None => {}
        }
    }
    if !delivery.plan.presence_ready() {
        return;
    }
    let pending: Vec<String> = delivery.plan.pending().map(str::to_owned).collect();
    for path in pending {
        // Preserve the complete snapshot gate, but do not drain every ready
        // XML/image discovery operation into one long browser frame.
        if started.elapsed().as_secs_f64() >= 0.006 {
            break;
        }
        let handle = delivery
            .requests
            .entry(path.clone())
            .or_insert_with(|| server.load::<RawFile>(format!("mpq://{}", path.replace('\\', "/"))))
            .clone();
        let result = if let Some(file) = assets.get(&handle) {
            delivery.plan.accept(&path, file.0.clone())
        } else if let Some(LoadState::Failed(error)) = server.get_load_state(handle.id()) {
            Err(format!("{path}: {error}"))
        } else {
            continue;
        };
        if let Err(error) = result {
            error!("ui_files: delivery failed; interface will not be published: {error}");
            *state = UiFilesState::Failed(error);
            commands.remove_resource::<Delivery>();
            return;
        }
        delivery.requests.remove(&path);
    }
    let ms = started.elapsed().as_secs_f64() * 1000.0;
    timing.0 += ms;
    timing.1 = timing.1.max(ms);
    timing.2 += 1;
    if ms >= 16.0 {
        info!("startup cost: interface delivery/discovery {ms:.1}ms this frame");
    }
    if delivery.plan.pending().next().is_some() || !delivery.plan.presence_ready() {
        return;
    }
    let plan = std::mem::take(&mut delivery.plan);
    let publish_started = benilla_time::Instant::now();
    match plan.into_snapshot() {
        Ok((files, index)) => {
            let count = files.len();
            benilla_interface_files::publish_with_addons(
                files,
                index,
                delivery
                    .addons
                    .take()
                    .expect("addon catalog admitted before publication"),
            );
            *state = UiFilesState::Ready;
            info!("ui_files: {count} production interface files resident; no Lua executed by delivery");
            info!("startup cost: interface elapsed={:.1}ms processing_cpu={:.1}ms max_frame={:.1}ms passes={} publish={:.1}ms",
                delivery.started.elapsed().as_secs_f64() * 1000.0, timing.0, timing.1, timing.2,
                publish_started.elapsed().as_secs_f64() * 1000.0);
        }
        Err(error) => {
            error!("ui_files: {error}");
            *state = UiFilesState::Failed(error);
        }
    }
    commands.remove_resource::<Delivery>();
}
