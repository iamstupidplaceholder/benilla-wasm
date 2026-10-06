//! Address Modified mesh events to their actual 3D users, not every resident mesh.
//! The other Bevy asset consumers remain authoritative and unchanged.

use bevy::asset::AssetEventSystems;
use bevy::ecs::schedule::ScheduleCleanupPolicy;
use bevy::mesh::mark_3d_meshes_as_changed_if_their_assets_changed as original;
use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::*;

#[derive(Default)]
struct Users {
    by_entity: HashMap<Entity, AssetId<Mesh>>,
    by_mesh: HashMap<AssetId<Mesh>, HashSet<Entity>>,
}

impl Users {
    fn remove(&mut self, entity: Entity) {
        if let Some(id) = self.by_entity.remove(&entity) {
            if let Some(users) = self.by_mesh.get_mut(&id) {
                users.remove(&entity);
                if users.is_empty() {
                    self.by_mesh.remove(&id);
                }
            }
        }
    }
}

pub(super) fn install(app: &mut App) {
    if !app.is_plugin_added::<bevy::mesh::MeshPlugin>() {
        return;
    }
    // Initializing system parameters can initialize other schedules. Keep the
    // Schedules resource in World, taking only this schedule out while editing.
    // resource_scope<Schedules> would let initialization recreate the resource
    // and then overwrite it on scope exit (and panic in debug builds).
    let mut schedule = app
        .world_mut()
        .resource_mut::<Schedules>()
        .remove(PostUpdate)
        .expect("MeshPlugin PostUpdate");
    let result = schedule.remove_systems_in_set(
        original,
        app.world_mut(),
        ScheduleCleanupPolicy::RemoveSystemsOnly,
    );
    if matches!(result, Ok(1)) {
        // Retain the empty original function set as an ordering bridge, not
        // a container: arbitrary system-type-set membership is forbidden.
        schedule.add_systems(mark_users.before(original).after(AssetEventSystems));
    }
    app.world_mut().resource_mut::<Schedules>().insert(schedule);
    let removed = result.expect("replace mesh change propagation");
    assert_eq!(
        removed, 1,
        "expected one Bevy mesh change propagation system"
    );
}

fn mark_users(
    mut users: Local<Users>,
    mut removed: RemovedComponents<Mesh3d>,
    mut meshes: ParamSet<(
        Query<(Entity, &Mesh3d), Changed<Mesh3d>>,
        Query<&mut Mesh3d>,
    )>,
    mut events: MessageReader<AssetEvent<Mesh>>,
) {
    // Removal then admission also handles remove/reinsert in the same frame.
    for entity in removed.read() {
        users.remove(entity);
    }
    for (entity, mesh) in &meshes.p0() {
        let id = mesh.0.id();
        if users.by_entity.get(&entity) == Some(&id) {
            continue;
        }
        users.remove(entity);
        users.by_entity.insert(entity, id);
        users.by_mesh.entry(id).or_default().insert(entity);
    }
    let mut q = meshes.p1();
    for event in events.read() {
        // Exactly the original selector: Added/Removed/Loaded are NOT marked.
        if let AssetEvent::Modified { id } = event {
            if let Some(entities) = users.by_mesh.get(id) {
                for &entity in entities {
                    if let Ok(mut mesh) = q.get_mut(entity) {
                        // Check the live handle too: never dirty a reassigned user.
                        if mesh.0.id() == *id {
                            mesh.set_changed();
                        }
                    }
                }
            }
        }
    }
}
