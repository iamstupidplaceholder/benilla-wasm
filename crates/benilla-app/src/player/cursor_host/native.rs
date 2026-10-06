//! Native hosts can restore the pointer after releasing mouse-look.

pub(super) fn restore_position(window: &mut bevy::window::Window, position: bevy::math::Vec2) {
    window.set_cursor_position(Some(position));
}
