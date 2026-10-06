use bevy::prelude::*;

/// The production lifecycle enum is shared in shape so the existing login
/// policy and state transitions compile unchanged.  Character-select systems
/// join the browser composition after the login transport lands.
#[derive(States, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub(crate) enum ClientState {
    #[default]
    Login,
    CharSelect,
    CharCreate,
    InWorld,
}

pub(crate) fn wow_font(assets: &AssetServer) -> Handle<Font> {
    assets.load("mpq://Fonts/FRIZQT__.ttf")
}
