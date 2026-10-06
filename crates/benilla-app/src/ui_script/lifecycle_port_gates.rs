//! Real lifecycle edges and the entry's production binding seed, without IO writes,
//! the complete FrameXML/addon load or a replacement VM/controller implementation.

use super::*;

pub fn verify_ui_vm_entry_boundary() {
    let mut world = World::new();
    world.init_resource::<UiClock>();
    world.init_resource::<crate::cvars::Cvars>();
    setup_script(&mut world);
    let boot = world.non_send_resource::<UiScript>().session();
    arm_entry_ui_load(&mut world);
    assert!(world.contains_resource::<PendingEntryUiLoad>());
    assert!(world.get_non_send_resource::<UiScript>().is_none());
    assert_eq!(world.non_send_resource::<ParkedBootVm>().0.session(), boot);

    // Leaving during the deferral must unpark the boot VM, not create an
    // interface or seed bindings for a session that never entered the world.
    world.insert_resource(State::new(crate::char_select::ClientState::Login));
    run_pending_entry_load(&mut world);
    assert!(!world.contains_resource::<PendingEntryUiLoad>());
    assert_eq!(world.non_send_resource::<UiScript>().session(), boot);

    arm_entry_ui_load(&mut world);
    // Same production mint edge used by load_ingame_ui_on_world_entry. This
    // deliberately stops before the reference-file walk; no reference data is
    // required to assert the VM identity and registration contract.
    unpark_boot_vm(&mut world);
    mint_entry_vm(&mut world);
    let mut entry = world.remove_non_send_resource::<UiScript>().unwrap();
    assert_ne!(
        entry.session(),
        boot,
        "entry cannot inherit the glue VM's memo identity"
    );
    crate::bindings::seed_bindings_for_vm(&mut world, &mut entry);
    entry
        .run("assert(GetNumBindings() > 0); assert(GetCurrentBindingSet() == 1)")
        .unwrap();
    let session = entry.session();
    world.insert_non_send_resource(entry);
    world.remove_resource::<PendingEntryUiLoad>();
    run_pending_boot(&mut world);
    assert_eq!(
        world.non_send_resource::<UiScript>().session(),
        session,
        "a completed entry cannot be overwritten by a deferred boot"
    );
}
