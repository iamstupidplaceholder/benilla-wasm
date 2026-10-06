//! Browser composition admitting the production login and player subsystems.
//!
//! The UI itself stays in `login/`, `glue/` and `glue_strings.rs`.  Only the
//! Host IO stays in adapters. Production PlayerPlugin now owns the world camera,
//! embodiment, collision and movement commands; production UI/backdrop plugins
//! own its render path. The original FrameXML/bindings lifecycle runs here;
//! Gameplay windows use the original plugins and shared incoming packet drain.
//! Remaining outbound verbs and catalog delivery are tracked in the web roadmap.

use benilla_assets::{LuaSource, LuaSourceLoader};
use bevy::{asset::AssetApp, pbr::PbrPlugin, prelude::*};

use crate::{
    char_create::CharCreatePlugin,
    char_select::{CharSelectPlugin, ClientState},
    entities::EntitiesPlugin,
    glue::GluePlugin,
    login::LoginPlugin,
    net::NetPlugin,
    portrait::{GluePreview, GlueScenePair, PortraitImages},
    realm_select::Realms,
    sound::BrowserSoundPlugin,
    textinput::TextInputPlugin,
    web_world::WebWorldLightPlugin,
    AppExit, BuildId,
};

pub(super) fn run(build: BuildId) -> AppExit {
    let mut app = App::new();
    benilla_assets::register_web_mpq_source(&mut app, &crate::web_config::endpoint("assets"));
    benilla_assets::register_web_presence_source(
        &mut app,
        &crate::web_config::endpoint("presence"),
    );
    benilla_assets::register_web_metadata_source(
        &mut app,
        "locale-catalog",
        crate::web_config::endpoint("locales"),
    );
    app.insert_resource(build)
        .insert_resource(ClearColor(Color::BLACK))
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "benilla".into(),
                        canvas: Some("#benilla".into()),
                        fit_canvas_to_parent: true,
                        ..default()
                    }),
                    ..default()
                })
                .set(PbrPlugin {
                    prepass_enabled: false,
                    add_default_deferred_lighting_plugin: false,
                    use_gpu_instance_buffer_builder: false,
                    ..default()
                }),
        )
        .add_plugins(crate::shaders::plugin)
        // Install the production renderer/streamer once for the whole process. Glue scenes use
        // its ordinary M2/effect lanes before entry; the browser adapter below supplies only the
        // asynchronous catalogs it cannot load synchronously on this target.
        .add_plugins(benilla_world::world_plugins::WorldPlugins)
        .init_resource::<crate::cvars::Cvars>()
        .init_resource::<benilla_assets::SpriteCache>()
        .insert_resource(benilla_assets::SpriteFiles::with_presence_probe(|path| {
            if benilla_interface_files::ready() {
                benilla_interface_files::contains(path).ok()
            } else {
                None
            }
        }))
        .init_resource::<benilla_world::schedule::WorldLive>()
        .init_resource::<PortraitImages>()
        .init_resource::<GluePreview>()
        .init_resource::<GlueScenePair>()
        .init_resource::<Realms>()
        // Genuine producer/consumer state, also initialized by the production UI
        // plugins. No alternate controller or fabricated capsule lives here.
        // Binding dispatch remains dependent on the production VM load edge.
        .init_resource::<crate::bindings::BindingsState>()
        .init_resource::<crate::ui_cast::LocalMoveStart>()
        .init_resource::<crate::ui_loot::LootMoveStart>()
        .init_resource::<crate::ui_script::InspectMode>()
        .init_resource::<crate::ui_script::UiKeyboardCapture>()
        .init_resource::<crate::ui_script::PlayerUiClickConsumed>()
        .init_resource::<crate::ui_script::PointerOverUi>()
        .init_resource::<crate::ui_hide::UiHidden>()
        .add_plugins((
            NetPlugin { connect: true },
            EntitiesPlugin,
            crate::creature_anim::CreatureAnimPlugin,
            crate::weapon_trail::WeaponTrailPlugin,
            BrowserSoundPlugin,
            TextInputPlugin,
            GluePlugin,
            crate::entities::WebAppearancePlugin,
            WebWorldLightPlugin,
            CharSelectPlugin {
                start: ClientState::Login,
            },
            LoginPlugin,
            CharCreatePlugin,
        ))
        .add_plugins((
            crate::player::PlayerPlugin,
            crate::ui_pass::PlayerUiPlugin,
            crate::world_backdrop::WorldBackdropPlugin,
            crate::web_ui_files::UiFilesPlugin,
            crate::ui_game_data::UiGameDataPlugin,
            crate::ui_text::UiTextPlugin,
            crate::ui_asset_probes::UiAssetProbesPlugin,
            crate::target::catalogs::TargetCatalogsPlugin,
            crate::target::WorldPickPlugin,
            crate::ui_action::catalogs::ActionCatalogsPlugin,
            crate::minimap::host::MinimapAssetsPlugin,
            crate::ui_models::UiModelsPlugin,
            crate::minimap::MinimapVmPlugin,
        ))
        .add_plugins((
            crate::ui_script::UiScriptPlugin,
            crate::bindings::BindingsPlugin,
            crate::video::VideoPlugin,
            crate::realmlist::RealmlistPlugin,
            crate::cvars::CvarPlugin,
            crate::console::ConsolePlugin,
            crate::ui_hide::UiHidePlugin,
            crate::ui_logout::UiLogoutPlugin,
        ))
        .add_plugins(GameplayWindows);

    // Fonts, glue BLPs and the two production glue-string Lua files all use
    // the gateway-backed asynchronous AssetServer path in the browser.
    benilla_assets::register_asset_loaders(&mut app);
    app.init_asset::<LuaSource>()
        .register_asset_loader(LuaSourceLoader);
    app.run()
}

/// The original gameplay plugins, with their real producers and session stores.
/// Network semantics now use the same drain as native; only transport IO differs.
struct GameplayWindows;

impl PluginGroup for GameplayWindows {
    fn build(self) -> bevy::app::PluginGroupBuilder {
        bevy::app::PluginGroupBuilder::start::<Self>()
            .add(crate::target::TargetPlugin)
            .add(crate::cursor::CursorPlugin)
            .add(crate::screenshot::ScreenshotPlugin)
            .add(crate::transport::TransportPlugin)
            .add(crate::screen_fade::ScreenFadePlugin)
            .add(crate::cinematic::CinematicPlugin)
            .add(crate::death::DeathPlugin)
            .add(crate::bowstring::BowstringPlugin)
            .add(crate::fishing_line::FishingLinePlugin)
            .add(crate::quest_markers::QuestMarkersPlugin)
            .add(crate::blob_shadow::BlobShadowPlugin)
            .add(crate::camera_shake::CameraShakePlugin)
            .add(crate::footprints::FootprintsPlugin)
            .add(crate::go_anim::plugin)
            .add(crate::doodad_events::plugin)
            .add(crate::pipe_warm::plugin)
            .add(crate::loading_screen::LoadingScreenPlugin)
            .add(crate::area::AreaPlugin)
            .add(crate::area_poi::AreaPoiPlugin)
            .add(crate::world_state_ui::WorldStateUiPlugin)
            .add(crate::area_trigger::AreaTriggerPlugin)
            .add(crate::poi_marker::PoiMarkerPlugin)
            .add(crate::minimap::MinimapPlugin)
            .add(crate::ui_world_map::WorldMapUiPlugin)
            .add(crate::ui_session::UiSessionPlugin)
            .add(crate::portrait::PortraitPlugin)
            .add(crate::ui_unit::UiUnitPlugin)
            .add(crate::ui_party::UiPartyPlugin)
            .add(crate::ui_duel::UiDuelPlugin)
            .add(crate::ui_binder::UiBinderPlugin)
            .add(crate::ui_dialog_verbs::UiDialogVerbsPlugin)
            .add(crate::ui_battlefield_score::BattlefieldScorePlugin)
            .add(crate::ui_battlefield::BattlefieldPlugin)
            .add(crate::ui_battlefield_positions::BattlefieldPositionsPlugin)
            .add(crate::tutorial::TutorialPlugin)
            .add(crate::swing_refusal::SwingRefusalPlugin)
            .add(crate::spell_mods::SpellModsPlugin)
            .add(crate::ui_summon::UiSummonPlugin)
            .add(crate::ui_gm_ticket::UiGmTicketPlugin)
            .add(crate::ui_follow::UiFollowPlugin)
            .add(crate::ui_instance::UiInstancePlugin)
            .add(crate::ui_social::UiSocialPlugin)
            .add(crate::ui_guild::UiGuildPlugin)
            .add(crate::ui_petition::UiPetitionPlugin)
            .add(crate::ui_tooltip::UiTooltipPlugin)
            .add(crate::game_tip::GameTipPlugin)
            .add(crate::text_reshape::TextReshapePlugin)
            .add(crate::text_filter::TextFilterPlugin)
            .add(crate::ui_char::UiCharPlugin)
            .add(crate::ui_reputation::UiReputationPlugin)
            .add(crate::ui_inspect::InspectUiPlugin)
            .add(crate::ui_honor::UiHonorPlugin)
            .add(crate::ui_dressup::DressUpUiPlugin)
            .add(crate::ui_action::UiActionPlugin)
            .add(crate::ui_aura::UiAuraPlugin)
            .add(crate::ui_spellbook::UiSpellbookPlugin)
            .add(crate::ui_macro::UiMacroPlugin)
            .add(crate::ui_talent::UiTalentPlugin)
            .add(crate::ui_talent_wipe::UiTalentWipePlugin)
            .add(crate::ui_shapeshift::UiShapeshiftPlugin)
            .add(crate::ui_pet::UiPetPlugin)
            .add(crate::chr_classes::ChrClassesPlugin)
            .add(crate::ui_pet_book::UiPetBookPlugin)
            .add(crate::ui_pet_stats::UiPetStatsPlugin)
            .add(crate::ui_pet_doll::UiPetDollPlugin)
            .add(crate::ui_net::UiNetPlugin)
            .add(crate::ui_cast::UiCastPlugin)
            .add(crate::ui_mirror::UiMirrorPlugin)
            .add(crate::combat_text::CombatTextPlugin)
            .add(crate::nameplates::NameplatesPlugin)
            .add(crate::raid_marks::RaidMarksPlugin)
            .add(crate::vplates::VPlatesPlugin)
            .add(crate::ui_items::UiItemsPlugin)
            .add(crate::ui_gossip::UiGossipPlugin)
            .add(crate::ui_merchant::UiMerchantPlugin)
            .add(crate::ui_auction::UiAuctionPlugin)
            .add(crate::ui_bank::UiBankPlugin)
            .add(crate::ui_mail::UiMailPlugin)
            .add(crate::ui_trade::UiTradePlugin)
            .add(crate::ui_item_text::UiItemTextPlugin)
            .add(crate::ui_saved::UiSavedPlugin)
            .add(crate::name_persist::NamePersistPlugin)
            .add(crate::ui_stable::UiStablePlugin)
            .add(crate::ui_tabard::TabardUiPlugin)
            .add(crate::ui_trainer::UiTrainerPlugin)
            .add(crate::ui_taxi::UiTaxiPlugin)
            .add(crate::ui_tradeskill::UiTradeSkillPlugin)
            .add(crate::ui_craft::UiCraftPlugin)
            .add(crate::ui_loot::UiLootPlugin)
            .add(crate::ui_loot_roll::UiLootRollPlugin)
            .add(crate::ui_quest::UiQuestPlugin)
            .add(crate::ui_quest_log::UiQuestLogPlugin)
            .add(crate::ui_quest_share::QuestSharePlugin)
            .add(crate::ui_chat::UiChatPlugin)
            .add(crate::chat_bubble::ChatBubblePlugin)
            .add(crate::ui_layout::UiLayoutPlugin)
    }
}
