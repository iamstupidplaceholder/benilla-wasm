//! Browser sky IO only: original model decoding and sampler-aware texture delivery.
use benilla_assets::RawFile;
use bevy::{asset::LoadState, ecs::system::SystemParam, prelude::*};
use std::collections::HashMap;

#[derive(Clone)]
pub(crate) struct SkyModel {
    pub subs: Vec<benilla_formats::RenderSubmesh>,
    pub spins: HashMap<u16, benilla_formats::BoneSpin>,
}

#[derive(Resource, Default)]
pub(crate) struct Inputs {
    sprites: benilla_assets::SpriteCache,
    sprite_files: benilla_assets::SpriteFiles,
    files: HashMap<String, Handle<RawFile>>,
    decoded: HashMap<String, Result<SkyModel, String>>,
    textures: HashMap<String, Handle<Image>>,
}

#[derive(SystemParam)]
pub(crate) struct Host<'w> {
    inputs: ResMut<'w, Inputs>,
    files: Res<'w, Assets<RawFile>>,
    server: Res<'w, AssetServer>,
}

impl Host<'_> {
    pub(crate) fn sprite(
        &mut self,
        path: &str,
        images: &mut Assets<Image>,
    ) -> Result<Option<Handle<Image>>, String> {
        let Inputs {
            sprites,
            sprite_files,
            ..
        } = &mut *self.inputs;
        sprites.begin_paint();
        let mut source = sprite_files.reader(&self.server, &self.files);
        match sprites.sprite_texture(&mut source, path, images) {
            Some(image) => Ok(Some(image)),
            None if sprites.has_pending() => Ok(None),
            None => Err(format!(
                "sky sprite {path}: original sprite decoder/delivery failed"
            )),
        }
    }
    pub(crate) fn texture(
        &mut self,
        path: &str,
        wrap: (bool, bool),
        images: &Assets<Image>,
    ) -> Result<Option<Handle<Image>>, String> {
        let url = benilla_assets::texture_url(path, wrap);
        let handle = self
            .inputs
            .textures
            .entry(url.clone())
            .or_insert_with(|| self.server.load(&url))
            .clone();
        if images.get(&handle).is_some() {
            return Ok(Some(handle));
        }
        if let Some(LoadState::Failed(error)) = self.server.get_load_state(handle.id()) {
            return Err(format!("sky texture {path}: {error}"));
        }
        Ok(None)
    }

    pub(crate) fn model(
        &mut self,
        path: &str,
        images: &Assets<Image>,
    ) -> Result<Option<SkyModel>, String> {
        let key = benilla_assets::normalize_path(path);
        if !self.inputs.decoded.contains_key(&key) {
            let handle = self
                .inputs
                .files
                .entry(key.clone())
                .or_insert_with(|| self.server.load(benilla_assets::m2_url(path)))
                .clone();
            if let Some(file) = self.files.get(&handle) {
                let dir = path.rfind(['\\', '/']).map(|i| &path[..i]).unwrap_or("");
                let parsed = benilla_formats::parse_m2_render_submeshes(&file.0, dir, &[])
                    .map(|subs| SkyModel {
                        subs,
                        spins: benilla_formats::m2_bone_spins(&file.0),
                    })
                    .map_err(|error| format!("sky model {path}: {error:#}"));
                self.inputs.decoded.insert(key.clone(), parsed);
                self.inputs.files.remove(&key);
            } else if let Some(LoadState::Failed(error)) = self.server.get_load_state(handle.id()) {
                let error = format!("sky model {path}: {error}");
                self.inputs.decoded.insert(key.clone(), Err(error.clone()));
                self.inputs.files.remove(&key);
                return Err(error);
            } else {
                return Ok(None);
            }
        }
        let model = self
            .inputs
            .decoded
            .get(&key)
            .expect("decoded sky state")
            .clone()?;
        let mut pending = false;
        for sub in &model.subs {
            if let Some(texture) = &sub.texture {
                pending |= self
                    .texture(texture, (sub.wrap_x, sub.wrap_y), images)?
                    .is_none();
            }
        }
        if pending {
            Ok(None)
        } else {
            Ok(Some(model))
        }
    }
}
