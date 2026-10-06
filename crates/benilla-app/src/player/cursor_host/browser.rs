//! Browser pointer lock restores its own pointer on unlock; web pages cannot
//! warp the OS cursor. Do not enqueue an unsupported winit cursor-position request.

pub(super) fn restore_position(_window: &mut bevy::window::Window, _position: bevy::math::Vec2) {}
