//! Run explicitly: cargo test -p benilla-app --features port-gates --test net_port
#![cfg(feature = "port-gates")]

#[test]
fn production_target_host_boundary_contract() {
    benilla_app::port_gates::verify_target_host_boundary();
}

#[test]
fn shared_session_entry_contract() {
    benilla_app::port_gates::verify_session_entry();
}

#[test]
fn shared_object_motion_contract() {
    benilla_app::port_gates::verify_object_motion();
}

#[test]
fn shared_speed_change_contract() {
    benilla_app::port_gates::verify_speed_changes();
}

#[test]
fn production_player_lifecycle_contract() {
    benilla_app::port_gates::verify_player_lifecycle();
}

#[test]
fn production_ui_file_delivery_contract() {
    benilla_app::port_gates::verify_ui_file_delivery();
}

#[test]
fn production_ui_vm_entry_boundary_contract() {
    benilla_app::port_gates::verify_ui_vm_entry_boundary();
}

#[test]
fn production_font_host_boundary_contract() {
    benilla_app::port_gates::verify_font_host_boundary();
}

#[test]
fn production_ui_paint_host_boundary_contract() {
    benilla_app::port_gates::verify_ui_paint_host_boundary();
}

#[test]
fn production_async_texture_size_probe_contract() {
    benilla_app::port_gates::verify_async_texture_size_probe();
}
