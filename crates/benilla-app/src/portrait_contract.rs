//! Target-neutral bridge between glue UI and the renderer-owned portrait output.
//!
//! Native and browser compositions deliberately differ below this boundary,
//! but the login/character screens must address the same slot and request the
//! same authored background scene.  Keeping these types here prevents a web
//! adapter from inventing a parallel scene contract.

use std::collections::HashMap;

use benilla_assets::materials::WowModelMaterial;
use bevy::prelude::*;

/// The glue render target's key in [`PortraitImages`].
pub(crate) const GLUE_SLOT: &str = "glue";

/// What a portrait slot currently exposes to the UI.
#[derive(Clone, PartialEq)]
pub(crate) enum PortraitSource {
    Live(Handle<Image>),
    File(String),
}

/// Renderer output sampled by the unchanged production UI.
#[derive(Resource, Default)]
pub(crate) struct PortraitImages(pub(crate) HashMap<String, PortraitSource>);

/// The authored background requested by the production glue screens.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum GlueScene {
    /// `UI_MainMenu`, used by the login screen.
    MainMenu,
    /// The race stage used by character create/select.
    Race(u8),
}

impl GlueScene {
    /// Token used by the shipped `UI_<token>` M2 path.
    pub(crate) fn token(self) -> &'static str {
        match self {
            Self::MainMenu => "MainMenu",
            Self::Race(race) => match race {
                2 | 8 => "Orc",   // Troll shares Orc's scene.
                3 | 7 => "Dwarf", // Gnome shares Dwarf's scene.
                4 => "NightElf",
                5 => "Scourge",
                6 => "Tauren",
                _ => "Human",
            },
        }
    }
}

/// Character-create appearance requested by the production glue screen.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct CreateLook {
    pub(crate) race: u8,
    pub(crate) sex: u8,
    pub(crate) class: u8,
    pub(crate) skin: u8,
    pub(crate) face: u8,
    pub(crate) hair_style: u8,
    pub(crate) hair_color: u8,
    pub(crate) facial_hair: u8,
}

/// Character-select appearance, including visible equipment and pet data.
#[derive(Clone, Copy, PartialEq)]
pub(crate) struct SelectLook {
    pub(crate) race: u8,
    pub(crate) sex: u8,
    pub(crate) skin: u8,
    pub(crate) face: u8,
    pub(crate) hair_style: u8,
    pub(crate) hair_color: u8,
    pub(crate) facial_hair: u8,
    pub(crate) flags: u32,
    pub(crate) equipment: [benilla_protocol::CharEnumItem; 19],
    pub(crate) pet_display_id: u32,
    pub(crate) pet_level: u32,
    pub(crate) pet_family: u32,
}

impl From<&benilla_protocol::Character> for SelectLook {
    fn from(c: &benilla_protocol::Character) -> Self {
        Self {
            race: c.race,
            sex: c.gender.min(1),
            skin: c.skin,
            face: c.face,
            hair_style: c.hair_style,
            hair_color: c.hair_color,
            facial_hair: c.facial_hair,
            flags: c.flags,
            equipment: c.equipment,
            pet_display_id: c.pet_display_id,
            pet_level: c.pet_level,
            pet_family: c.pet_family,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct PetLook {
    pub(crate) display_id: u32,
    pub(crate) level: u32,
    pub(crate) family: u32,
}

/// What stands on the authored glue stage.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum GlueLook {
    Create(CreateLook),
    Select(SelectLook),
}

impl GlueLook {
    pub(crate) fn body(&self) -> (u8, u8) {
        match self {
            Self::Create(look) => (look.race, look.sex),
            Self::Select(look) => (look.race, look.sex),
        }
    }

    pub(crate) fn pet(&self) -> Option<PetLook> {
        match self {
            Self::Create(_) => None,
            Self::Select(look) => (look.pet_display_id != 0).then_some(PetLook {
                display_id: look.pet_display_id,
                level: look.pet_level,
                family: look.pet_family,
            }),
        }
    }
}

/// The production glue screens' complete request to the renderer-owned booth.
#[derive(Resource, Default)]
pub(crate) struct GluePreview {
    pub(crate) scene: Option<GlueScene>,
    pub(crate) look: Option<GlueLook>,
    pub(crate) yaw: f32,
}

/// Character half of the atomic glue-stage swap.
///
/// The background renderer only needs to know which requested look has
/// finished assembling and whether it contains drawable parts. Native fills
/// this from the production character builder; login-only browser startup
/// keeps the empty default and can commit `UI_MainMenu` immediately.
#[derive(Resource, Default)]
pub(crate) struct GlueScenePair {
    pub(crate) look: Option<GlueLook>,
    pub(crate) has_parts: bool,
}

/// Render data stamped on every dressed unit-model part by the production entity attach path.
/// Both portrait implementations consume this same marker; it is entity/renderer data, not a
/// platform service.
#[derive(Component)]
pub(crate) struct PortraitPart {
    pub(crate) static_mesh: Handle<Mesh>,
    pub(crate) skinned_mesh: Option<Handle<Mesh>>,
    pub(crate) material: Handle<WowModelMaterial>,
}

/// A mesh that rides one of the dressed body's attachment bones.
#[derive(Component)]
pub(crate) struct PortraitRider {
    pub(crate) static_mesh: Handle<Mesh>,
    pub(crate) material: Handle<WowModelMaterial>,
    pub(crate) bone: u16,
    pub(crate) offset: Vec3,
    pub(crate) attach: Option<u16>,
}

/// Whose model a mirrored camera-facing batch belongs to.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum PortraitSeat {
    Body,
    Rider(Vec3),
}

impl PortraitSeat {
    pub(crate) fn offset(self) -> Vec3 {
        match self {
            Self::Body => Vec3::ZERO,
            Self::Rider(at) => at,
        }
    }
}

/// A camera-facing batch carried by a dressed body or one of its attached models.
#[derive(Component)]
pub(crate) struct PortraitBillboard {
    pub(crate) mesh: Handle<Mesh>,
    pub(crate) material: Handle<WowModelMaterial>,
    pub(crate) bone: u16,
    pub(crate) seat: PortraitSeat,
    pub(crate) kind: benilla_formats::BillboardKind,
    pub(crate) attach: Option<u16>,
}

/// Effect emitters carried by an attached model and mirrored into a portrait booth.
#[derive(Component)]
pub(crate) struct PortraitEffects {
    pub(crate) bone: u16,
    pub(crate) attach: Option<u16>,
    pub(crate) offset: Vec3,
    pub(crate) emitters: Vec<benilla_assets::ModelEmitter>,
}

/// A portrait subject assembled from a display cache rather than a live network object.
#[derive(Component)]
pub(crate) struct PortraitStandIn(pub(crate) u32);

/// The stable pane's selected live unit or display-only pet.
#[derive(Resource)]
pub(crate) struct StableBooth {
    pub(crate) yaw: f32,
    pub(crate) unit: Option<Entity>,
    pub(crate) display_id: Option<u32>,
}

impl Default for StableBooth {
    fn default() -> Self {
        Self {
            yaw: 0.61,
            unit: None,
            display_id: None,
        }
    }
}
