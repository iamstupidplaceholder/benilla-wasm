//! Browser page cover for process startup, distinct from world residency/loading art.
//! It observes real producer readiness; it never loads files or executes UI code.

use crate::web_config::startup_text;
use bevy::prelude::*;

#[derive(Resource)]
pub(super) struct StartupCover {
    pub(super) active: bool,
    ready_frames: u32,
    status: String,
    presented_frames: u32,
}

pub(super) fn build(app: &mut App) {
    app.insert_resource(StartupCover {
        active: true,
        ready_frames: 0,
        status: String::new(),
        presented_frames: 0,
    })
    .add_systems(Startup, setup)
    .add_systems(First, count_presented)
    .add_systems(PostUpdate, delivery_status)
    .add_systems(
        PostUpdate,
        update
            .before(bevy::camera::visibility::VisibilitySystems::VisibilityPropagate)
            .before(bevy::ui::UiSystems::Layout),
    );
}

#[derive(Component)]
struct StartupPanel;
#[derive(Component)]
struct StartupMessage;
#[derive(Component)]
struct StartupPulse(u8);

fn delivery_status(world: &mut World) {
    let startup = world.resource::<StartupCover>();
    let covering = world
        .get_resource::<super::LoadingScreen>()
        .is_some_and(|s| s.covering());
    let in_world = world
        .get_resource::<State<crate::char_select::ClientState>>()
        .is_some_and(|s| *s.get() == crate::char_select::ClientState::InWorld);
    let stage = if startup.active {
        startup.status.clone()
    } else if covering {
        startup_text("loading_world")
    } else {
        String::new()
    };
    benilla_assets::delivery_context(!in_world || covering || startup.active, &stage);
}

fn setup(mut commands: Commands, assets: Res<AssetServer>) {
    commands
        .spawn((
            StartupPanel,
            GlobalZIndex(2000),
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                position_type: PositionType::Absolute,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
        ))
        .with_children(|root| {
            root.spawn((
                Node {
                    max_width: Val::Percent(80.0),
                    padding: UiRect::all(Val::Px(28.0)),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px(16.0),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.02, 0.025, 0.04, 0.94)),
            ))
            .with_children(|panel| {
                panel.spawn((
                    StartupMessage,
                    Text::new(startup_text("preparing_client")),
                    TextFont {
                        font: assets.load("mpq://Fonts/FRIZQT__.TTF"),
                        font_size: 22.0,
                        ..default()
                    },
                    TextColor(Color::srgb(1.0, 0.78, 0.0)),
                ));
                panel
                    .spawn(Node {
                        column_gap: Val::Px(8.0),
                        ..default()
                    })
                    .with_children(|dots| {
                        for index in 0..3 {
                            dots.spawn((
                                StartupPulse(index),
                                Node {
                                    width: Val::Px(8.0),
                                    height: Val::Px(8.0),
                                    ..default()
                                },
                                BackgroundColor(Color::srgb(1.0, 0.78, 0.0)),
                            ));
                        }
                    });
            });
        });
}

fn count_presented(mut cover: ResMut<StartupCover>) {
    if cover.active {
        cover.presented_frames = cover.presented_frames.saturating_add(1);
        if cover.presented_frames == 3 {
            crate::web_config::hide_bootstrap_status();
        }
    }
}

pub(crate) fn presented(world: &World) -> bool {
    world
        .get_resource::<StartupCover>()
        .is_none_or(|cover| cover.presented_frames >= 3)
}

fn update(world: &mut World) {
    if !world.resource::<StartupCover>().active {
        return;
    }
    let seconds = world.resource::<Time>().elapsed_secs();
    let mut pulses = world.query::<(&StartupPulse, &mut BackgroundColor)>();
    for (pulse, mut color) in pulses.iter_mut(world) {
        let alpha = 0.25 + 0.75 * ((seconds * 4.0 - pulse.0 as f32 * 1.3).sin() * 0.5 + 0.5);
        color.0 = Color::srgba(1.0, 0.78, 0.0, alpha);
    }
    let status = if let Some(crate::web_ui_files::UiFilesState::Failed(reason)) =
        world.get_resource::<crate::web_ui_files::UiFilesState>()
    {
        Some(format!("{}: {reason}", startup_text("interface_failed")))
    } else if !benilla_interface_files::ready() {
        Some(startup_text("loading_interface"))
    } else if !world
        .get_resource::<crate::glue::art::GlueArt>()
        .is_some_and(|art| art.ready())
    {
        Some(startup_text("preparing_artwork"))
    } else if !world.contains_resource::<crate::ui_text::UiFontAtlas>() {
        Some(startup_text("preparing_fonts"))
    } else if world
        .get_non_send_resource::<benilla_ui::script::UiScript>()
        .is_none()
    {
        Some(startup_text("preparing_ui"))
    } else if !world
        .get_resource::<crate::ui_game_data::UiGameDataReady>()
        .is_some_and(|ready| ready.0)
        || !world
            .get_resource::<crate::ui_action::catalogs::ActionCatalogsReady>()
            .is_some_and(|ready| ready.0)
        || !world.contains_resource::<crate::entities::CharCreate>()
    {
        Some(startup_text("loading_catalogs"))
    } else {
        None
    };
    let mut chrome =
        world.query_filtered::<&mut Visibility, With<crate::glue::widgets::StartupChrome>>();
    for mut visibility in chrome.iter_mut(world) {
        *visibility = Visibility::Hidden;
    }
    let mut cover = world.resource_mut::<StartupCover>();
    cover.ready_frames = if status.is_none() {
        cover.ready_frames.saturating_add(1)
    } else {
        0
    };
    let text = status.unwrap_or_else(|| startup_text("preparing_login"));
    if cover.status != text {
        cover.status = text;
    }
    // Let the original screen tree/layout/render settle before exposing input.
    if cover.ready_frames >= 3 {
        cover.active = false;
        info!("startup cover released: interface, glue art, fonts, boot VM and catalogs ready");
    }
    let active = cover.active;
    let text = cover.status.clone();
    drop(cover);
    let mut messages = world.query_filtered::<&mut Text, With<StartupMessage>>();
    for mut message in messages.iter_mut(world) {
        if message.0 != text {
            message.0.clone_from(&text);
        }
    }
    if !active {
        let mut panels = world.query_filtered::<Entity, With<StartupPanel>>();
        let panels: Vec<_> = panels.iter(world).collect();
        for panel in panels {
            world.entity_mut(panel).despawn();
        }
        let mut chrome =
            world.query_filtered::<&mut Visibility, With<crate::glue::widgets::StartupChrome>>();
        for mut visibility in chrome.iter_mut(world) {
            *visibility = Visibility::Inherited;
        }
    }
}
