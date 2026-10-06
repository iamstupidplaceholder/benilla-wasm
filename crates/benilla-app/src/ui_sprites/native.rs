//! Preserve the original native WorldAssets API and startup availability.

use benilla_assets::WorldAssets;
use bevy::{ecs::system::SystemParam, prelude::*};

#[derive(SystemParam)]
pub(crate) struct UiSprites<'w> {
    assets: Option<ResMut<'w, WorldAssets>>,
}

impl UiSprites<'_> {
    pub(crate) fn as_mut(&mut self) -> Option<&mut WorldAssets> {
        self.assets.as_deref_mut()
    }
    pub(crate) fn has_pending(&self) -> bool {
        false
    }
    pub(crate) fn begin_paint(&mut self) {}

    pub(crate) fn sprite_texture_ready(
        &mut self,
        path: &str,
        images: &mut Assets<Image>,
    ) -> Option<Option<Handle<Image>>> {
        Some(
            self.assets
                .as_deref_mut()
                .and_then(|assets| assets.sprite_texture(path, images)),
        )
    }

    /// Outer None is host unavailability; inner None is a completed miss.
    pub(crate) fn sprite_texture_tiled_ready(
        &mut self,
        path: &str,
        images: &mut Assets<Image>,
    ) -> Option<Option<Handle<Image>>> {
        self.assets
            .as_deref_mut()
            .map(|assets| assets.sprite_texture_tiled(path, images))
    }

    /// Outer None is host unavailability; inner None is a completed miss.
    pub(crate) fn mask_texture_ready(
        &mut self,
        path: &str,
        images: &mut Assets<Image>,
    ) -> Option<Option<Handle<Image>>> {
        self.assets
            .as_deref_mut()
            .map(|assets| assets.mask_texture(path, images))
    }
}
