//! Host IO only: deliver DBC bytes, then use the original decoders and stores.

use benilla_assets::RawFile;
use benilla_formats::MemoryFileSource;
use bevy::{asset::LoadState, prelude::*};

const FILES: &[&str] = &[
    "DBFilesClient/SkillLine.dbc",
    "DBFilesClient/SkillLineAbility.dbc",
    "DBFilesClient/SkillLineCategory.dbc",
    "DBFilesClient/SkillRaceClassInfo.dbc",
    "DBFilesClient/SpellIcon.dbc",
    "DBFilesClient/ItemSubClass.dbc",
    "DBFilesClient/ItemSubClassMask.dbc",
    "DBFilesClient/ItemClass.dbc",
    "DBFilesClient/ItemSet.dbc",
    "DBFilesClient/ItemBagFamily.dbc",
    "DBFilesClient/DurabilityCosts.dbc",
    "DBFilesClient/DurabilityQuality.dbc",
    "DBFilesClient/ItemRandomProperties.dbc",
    "DBFilesClient/BankBagSlotPrices.dbc",
    "DBFilesClient/StableSlotPrices.dbc",
    "DBFilesClient/PetPersonality.dbc",
    "DBFilesClient/PetLoyalty.dbc",
    "DBFilesClient/CreatureFamily.dbc",
    "DBFilesClient/ItemPetFood.dbc",
    "DBFilesClient/Stationery.dbc",
    "DBFilesClient/PageTextMaterial.dbc",
    "DBFilesClient/AuctionHouse.dbc",
    "DBFilesClient/Exhaustion.dbc",
    "DBFilesClient/GMTicketCategory.dbc",
    "DBFilesClient/CinematicSequences.dbc",
    "DBFilesClient/CinematicCamera.dbc",
    "DBFilesClient/ChrClasses.dbc",
    "DBFilesClient/AreaTable.dbc",
    "DBFilesClient/AreaPOI.dbc",
    "DBFilesClient/WorldStateUI.dbc",
    "DBFilesClient/AreaTrigger.dbc",
    "DBFilesClient/CameraShakes.dbc",
    "DBFilesClient/SpellEffectCameraShakes.dbc",
    "DBFilesClient/GameTips.dbc",
    "DBFilesClient/SpellFocusObject.dbc",
    "DBFilesClient/QuestSort.dbc",
    "DBFilesClient/QuestInfo.dbc",
    "DBFilesClient/FootprintTextures.dbc",
    "DBFilesClient/GroundEffectTexture.dbc",
    "DBFilesClient/TerrainType.dbc",
    "DBFilesClient/FootstepTerrainLookup.dbc",
    "DBFilesClient/TaxiNodes.dbc",
    "DBFilesClient/TaxiPath.dbc",
    "DBFilesClient/TaxiPathNode.dbc",
    "DBFilesClient/TransportAnimation.dbc",
    "DBFilesClient/Talent.dbc",
    "DBFilesClient/TalentTab.dbc",
    "DBFilesClient/GameObjectDisplayInfo.dbc",
    "DBFilesClient/Lock.dbc",
    "DBFilesClient/LockType.dbc",
    "DBFilesClient/ChrRaces.dbc",
    "DBFilesClient/CreatureType.dbc",
    "DBFilesClient/Languages.dbc",
    "DBFilesClient/ChatChannels.dbc",
    "DBFilesClient/EmotesText.dbc",
    "DBFilesClient/Emotes.dbc",
    "DBFilesClient/EmotesTextSound.dbc",
    "DBFilesClient/EmotesTextData.dbc",
    "DBFilesClient/ServerMessages.dbc",
    "DBFilesClient/LanguageWords.dbc",
    "DBFilesClient/Map.dbc",
    "DBFilesClient/LoadingScreens.dbc",
    "DBFilesClient/WorldMapArea.dbc",
    "DBFilesClient/WorldMapContinent.dbc",
    "DBFilesClient/WorldMapOverlay.dbc",
    "DBFilesClient/Resistances.dbc",
];

#[derive(Resource)]
struct Delivery(Vec<(String, Handle<RawFile>)>, bool, Option<String>);

pub(crate) fn entry_wait_status(world: &World) -> String {
    let spells = world
        .get_resource::<crate::ui_action::catalogs::ActionCatalogsReady>()
        .map(|r| r.0);
    let Some(delivery) = world.get_resource::<Delivery>() else {
        return format!("spells_ready={spells:?}, no active delivery");
    };
    if let Some(error) = &delivery.2 {
        return format!("spells_ready={spells:?}, mandatory_catalog_failed={error}");
    }
    let files = world.resource::<Assets<RawFile>>();
    let server = world.resource::<AssetServer>();
    let pending: Vec<_> = delivery
        .0
        .iter()
        .filter(|(_, h)| {
            files.get(h).is_none()
                && !matches!(server.get_load_state(h.id()), Some(LoadState::Failed(_)))
        })
        .map(|(path, h)| (path, server.get_load_state(h.id())))
        .collect();
    format!(
        "spells_ready={spells:?}, bitmap_paths_discovered={}, pending={}, sample={:?}",
        delivery.1,
        pending.len(),
        &pending[..pending.len().min(5)]
    )
}

pub(crate) struct UiGameDataPlugin;

impl Plugin for UiGameDataPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<super::UiGameDataReady>()
            .add_systems(Startup, request)
            .add_systems(Update, publish_macro_icons.before(crate::ui_script::UiFeed))
            .add_systems(Update, publish.before(crate::ui_script::UiFeed));
    }
}

fn request(mut commands: Commands, server: Res<AssetServer>) {
    commands.insert_resource(MacroIconDelivery(server.load("mpqmeta://macro-icons.bfi")));
    commands.insert_resource(Delivery(
        FILES
            .iter()
            .map(|&path| (path.to_owned(), server.load(format!("mpq://{path}"))))
            .collect(),
        false,
        None,
    ));
}

#[derive(Resource)]
struct MacroIconDelivery(Handle<RawFile>);

fn publish_macro_icons(
    mut commands: Commands,
    delivery: Option<Res<MacroIconDelivery>>,
    files: Res<Assets<RawFile>>,
    server: Res<AssetServer>,
) {
    let Some(delivery) = delivery else { return };
    if let Some(file) = files.get(&delivery.0) {
        match std::str::from_utf8(&file.0) {
            Ok(text) => {
                let icons: Vec<String> = text.lines().map(str::to_owned).collect();
                info!(
                    "ui_macro: delivered {} original archive-enumerated icons",
                    icons.len()
                );
                commands.insert_resource(crate::ui_macro::MacroIcons(icons));
            }
            Err(error) => warn!("ui_macro: invalid icon metadata: {error}"),
        }
    } else if !matches!(
        server.get_load_state(delivery.0.id()),
        Some(LoadState::Failed(_))
    ) {
        return;
    } else {
        warn!("ui_macro: icon enumeration delivery failed; restart the updated gateway");
    }
    commands.remove_resource::<MacroIconDelivery>();
}

fn publish(
    mut commands: Commands,
    delivery: Option<ResMut<Delivery>>,
    files: Res<Assets<RawFile>>,
    server: Res<AssetServer>,
    spell_delivery: Res<crate::ui_action::catalogs::ActionCatalogsReady>,
    mut ready: ResMut<super::UiGameDataReady>,
    mut channels: ResMut<crate::ui_chat::ChannelState>,
    mut languages: ResMut<crate::ui_chat::ChatLanguages>,
    mut tips: ResMut<crate::game_tip::GameTips>,
    mut timetables: ResMut<crate::transport::Timetables>,
) {
    let Some(mut delivery) = delivery else { return };
    if delivery.2.is_some() {
        return;
    }
    if delivery.0.iter().any(|(_, h)| {
        files.get(h).is_none()
            && !matches!(server.get_load_state(h.id()), Some(LoadState::Failed(_)))
    }) {
        return;
    }
    // The action adapter publishes the spell join separately. Do not load
    // FrameXML against a temporarily absent spell catalog.
    if !spell_delivery.0 {
        return;
    }
    let mut source =
        MemoryFileSource::new(delivery.0.iter().filter_map(|(p, h)| match files.get(h) {
            Some(file) => Some((p.as_str(), file.0.clone())),
            None => {
                warn!(
                    "ui_game_data: delivery failed for {p}: {:?}",
                    server.get_load_state(h.id())
                );
                None
            }
        }));
    if !delivery.1 {
        if let Ok(catalog) = benilla_formats::load_cinematics(&mut source) {
            for camera in catalog.cameras() {
                let path = benilla_formats::camera_model_path(&camera.model).replace('\\', "/");
                let handle = server.load(format!("mpq://{path}"));
                delivery.0.push((path, handle));
            }
        }
        // Discover zone-bitmaps from the original continent rows, rather than
        // guessing install filenames. Optional misses become real decode errors
        // only after AssetServer delivery has finished.
        if let Ok(areas) = benilla_formats::load_world_map_area_catalog(&mut source) {
            for (_, area) in areas.iter().filter(|(_, area)| area.area_id == 0) {
                let path = format!("Interface/WorldMap/{}.zmp", area.name);
                let handle = server.load(format!("mpq://{path}"));
                delivery.0.push((path, handle));
            }
        }
        delivery.1 = true;
        return;
    }
    // Critical inputs have no usable substitute. Optional native catalogs below
    // retain their original logged miss policy; readiness certifies admission,
    // not merely that HTTP handles reached a terminal state.
    macro_rules! required {
        ($load:path) => {
            match $load(&mut source) {
                Ok(value) => value,
                Err(error) => {
                    let reason = format!("{}: {error:#}", stringify!($load));
                    error!("ui_game_data: mandatory catalog not admitted: {reason}");
                    delivery.2 = Some(reason);
                    return;
                }
            }
        };
    }
    let skill_lines = required!(benilla_formats::load_skill_line_catalog);
    let classes = required!(benilla_formats::load_chr_classes);
    let areas = required!(benilla_formats::load_area_table_catalog);
    let maps = required!(benilla_formats::load_map_catalog);
    let schools = required!(benilla_formats::load_resistance_catalog);
    let unit_names = required!(benilla_formats::load_unit_names);
    commands.insert_resource(crate::ui_unit::UnitNamesRes(unit_names));
    commands.insert_resource(crate::ui_spellbook::SkillLines {
        catalog: skill_lines,
    });
    commands.insert_resource(crate::chr_classes::ChrClassTable(classes));
    commands.insert_resource(crate::ui_chat::DamageSchools(schools));
    macro_rules! catalog {
        ($load:path, $wrap:expr) => {
            match $load(&mut source) {
                Ok(value) => {
                    commands.insert_resource(($wrap)(value));
                }
                Err(error) => warn!("ui_game_data: {}: {error:#}", stringify!($load)),
            }
        };
    }
    catalog!(
        benilla_formats::load_languages,
        crate::ui_unit::LanguagesRes
    );
    catalog!(
        benilla_formats::load_default_languages,
        crate::ui_unit::DefaultLanguagesRes
    );
    catalog!(
        benilla_formats::load_loading_screens,
        crate::loading_screen::LoadingScreenCatalogRes
    );
    crate::ui_chat::publish_catalogs(&mut commands, &mut source, &mut channels, &mut languages);
    catalog!(
        benilla_formats::load_item_sub_classes,
        crate::ui_items::ItemSubClasses
    );
    catalog!(
        benilla_formats::load_item_classes,
        crate::ui_items::ItemClasses
    );
    catalog!(benilla_formats::load_item_sets, crate::ui_items::ItemSets);
    catalog!(
        benilla_formats::load_random_property_catalog,
        crate::items::RandomProperties
    );
    catalog!(
        benilla_formats::load_bank_bag_slot_prices,
        crate::ui_bank::BankPrices
    );
    catalog!(
        benilla_formats::load_stable_slot_prices,
        crate::ui_stable::StableSlotPrices
    );
    catalog!(
        benilla_formats::load_stationery_catalog,
        crate::ui_mail::Stationery
    );
    catalog!(
        benilla_formats::load_page_text_material_catalog,
        crate::ui_item_text::PageMaterials
    );
    match (
        benilla_formats::load_pet_personalities(&mut source),
        benilla_formats::load_pet_loyalty_names(&mut source),
    ) {
        (Ok(personalities), Ok(loyalty)) => {
            commands.insert_resource(crate::ui_pet_stats::PetStatTables {
                personalities,
                loyalty,
            });
        }
        (personalities, loyalty) => warn!(
            "ui_game_data: pet stat tables unavailable: personality={:?}, loyalty={:?}",
            personalities.err(),
            loyalty.err()
        ),
    }
    match (
        benilla_formats::load_creature_families(&mut source),
        benilla_formats::load_pet_food_names(&mut source),
    ) {
        (Ok(families), Ok(foods)) => {
            commands.insert_resource(crate::ui_pet_stats::PetFamilyTables { families, foods });
        }
        (families, foods) => warn!(
            "ui_game_data: pet family tables unavailable: family={:?}, food={:?}",
            families.err(),
            foods.err()
        ),
    }
    catalog!(benilla_formats::load_durability_tables, |tables| {
        crate::ui_merchant::RepairTables(std::sync::Arc::new(crate::ui_merchant::RepairCatalog(
            tables,
        )))
    });
    catalog!(
        benilla_formats::load_item_bag_families,
        crate::ui_items::ItemBagFamilies
    );
    catalog!(
        benilla_formats::load_auction_houses,
        crate::ui_auction::AuctionHouses
    );
    crate::area_poi::publish_catalog(&mut commands, &mut source);
    catalog!(
        benilla_formats::load_cinematics,
        crate::cinematic::Cinematics
    );
    commands.insert_resource(crate::cinematic::CinematicFiles(MemoryFileSource::new(
        delivery
            .0
            .iter()
            .filter(|(p, _)| p.to_ascii_lowercase().starts_with("cameras/"))
            .filter_map(|(p, h)| files.get(h).map(|f| (p.as_str(), f.0.clone()))),
    )));
    catalog!(
        benilla_formats::load_exhaustion,
        crate::ui_unit::ExhaustionRows
    );
    catalog!(
        benilla_formats::load_gm_ticket_categories,
        |cat: benilla_formats::GmTicketCategoryCatalog| {
            crate::ui_gm_ticket::GmTicketCategories(
                cat.categories()
                    .iter()
                    .map(|c| (c.id, c.name.clone()))
                    .collect(),
            )
        }
    );
    catalog!(
        benilla_formats::load_world_state_ui_catalog,
        crate::world_state_ui::WorldStateUiRes
    );
    catalog!(
        benilla_formats::load_area_trigger_catalog,
        crate::area_trigger::AreaTriggers
    );
    catalog!(
        benilla_formats::load_camera_shakes,
        crate::camera_shake::Shakes
    );
    catalog!(benilla_formats::load_spell_focus_catalog, |catalog| {
        crate::ui_tradeskill::SpellFocus { catalog }
    });
    catalog!(
        benilla_formats::load_quest_header_names,
        crate::ui_quest_log::QuestHeaderNamesRes
    );
    catalog!(
        benilla_formats::load_quest_tag_names,
        crate::ui_quest_log::QuestTagNamesRes
    );
    crate::game_tip::publish_catalog(&mut tips, &mut source);
    catalog!(
        benilla_formats::load_footstep_catalog,
        crate::sound::footsteps::Footsteps
    );
    crate::footprints::publish_ink(&mut commands, &mut source, &server);
    crate::ui_taxi::publish_catalogs(&mut commands, &mut source);
    crate::transport::publish_catalogs(&mut timetables, &mut source);
    catalog!(
        benilla_formats::load_gameobject_catalog,
        crate::entities::GameObjects::from_catalog
    );
    catalog!(
        benilla_formats::load_lock_catalog,
        crate::go_templates::Locks
    );
    catalog!(
        benilla_formats::load_lock_type_catalog,
        crate::go_templates::LockTypes
    );
    catalog!(benilla_formats::load_talent_catalog, |catalog| {
        crate::ui_talent::Talents { catalog }
    });
    if let Some((views, data)) = crate::ui_world_map::build_catalog(
        &mut source,
        &areas,
        &benilla_assets::MapCatalogRes(maps),
    ) {
        info!(
            "world map: delivered {} continents with original zone bitmaps",
            views.len()
        );
        commands.insert_resource(crate::ui_world_map::WorldMapCatalog(views));
        commands.insert_resource(data);
    } else {
        let reason = "world map catalog/zone bitmap decode failed".to_owned();
        error!("ui_game_data: mandatory catalog not admitted: {reason}");
        delivery.2 = Some(reason);
        return;
    }
    commands.insert_resource(crate::area::AreaTableRes(areas));
    ready.0 = true;
    commands.remove_resource::<Delivery>();
    info!("ui_game_data: production unit, skill-line and chat catalogs admitted");
}
