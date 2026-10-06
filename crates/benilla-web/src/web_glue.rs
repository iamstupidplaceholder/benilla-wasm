//! First browser glue milestone: the stock AccountLogin.xml chrome, without
//! pretending its Lua callbacks or ModelFFX background work yet.

use benilla_assets::{
    m2_url, web_materials::WebModelMaterial, BlpLoaderSettings, BlpVariant, FrameXmlSource, M2Model,
};
use benilla_ui::framexml::{self, Element, TopLevel};
use bevy::{
    asset::LoadState,
    camera::{ClearColorConfig, PerspectiveProjection},
    prelude::*,
};

use crate::original_glue::{
    art::GlueArt,
    widgets::{glue_button, glue_edit_box, GlueBtnKind},
};

const LOGIN_XML: &str = "mpq://interface/gluexml/accountlogin.xml";
const MENU_MODEL: &str = "Interface\\Glues\\Models\\UI_MainMenu\\UI_MainMenu.mdx";
const SCENE_BATCHES_PER_FRAME: usize = 8;

#[derive(Resource)]
pub(super) struct MenuScene {
    model: Handle<M2Model>,
    next_batch: usize,
    camera_ready: bool,
    failed: bool,
}

#[derive(Component)]
pub(super) struct MenuCamera;

#[derive(Resource)]
pub(super) struct LoginSource {
    xml: Handle<FrameXmlSource>,
    button_up: Handle<Image>,
    button_down: Handle<Image>,
    done: bool,
}

#[derive(Component, Clone, Copy)]
enum LoginAction {
    Account,
    Password,
    Login,
    Quit,
}

#[derive(Component, Clone)]
struct LoginField;

#[derive(Component)]
pub(super) struct LoginCanvas;

#[derive(Clone, Copy)]
struct Rect {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

struct LoginChrome {
    logo_file: String,
    logo: Rect,
    account: Rect,
    password: Rect,
    login_top: f32,
    exit_right: f32,
    exit_bottom: f32,
}

pub(super) fn setup(mut commands: Commands, assets: Res<AssetServer>) {
    commands.spawn((
        Camera2d,
        Camera {
            order: 1,
            clear_color: ClearColorConfig::None,
            ..default()
        },
        bevy::ui::IsDefaultUiCamera,
    ));
    commands.spawn((
        Camera3d::default(),
        Camera {
            order: 0,
            clear_color: ClearColorConfig::Custom(Color::BLACK),
            ..default()
        },
        MenuCamera,
    ));
    commands.insert_resource(MenuScene {
        model: assets.load(m2_url(MENU_MODEL)),
        next_batch: 0,
        camera_ready: false,
        failed: false,
    });
    commands.insert_resource(LoginSource {
        xml: assets.load(LOGIN_XML),
        button_up: load_sprite(&assets, "interface/glues/common/glue-panel-button-up.blp"),
        button_down: load_sprite(&assets, "interface/glues/common/glue-panel-button-down.blp"),
        done: false,
    });
}

/// The shipped ModelFFX's M2 and its authored camera 0, using the same web
/// model batches as A3. This is a static first frame: bone/material animation,
/// scene lights and particles still require the production booth path.
pub(super) fn drive_scene(
    mut commands: Commands,
    assets: Res<AssetServer>,
    models: Res<Assets<M2Model>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<WebModelMaterial>>,
    mut scene: ResMut<MenuScene>,
    mut camera: Query<(&mut Transform, &mut Projection), With<MenuCamera>>,
    window: Query<&Window>,
) {
    if scene.failed {
        return;
    }
    if let LoadState::Failed(error) = assets.load_state(&scene.model) {
        error!("UI_MainMenu failed to load: {error}");
        scene.failed = true;
        return;
    }
    let Some(model) = models.get(&scene.model) else {
        return;
    };
    if scene.camera_ready && scene.next_batch == model.submeshes.len() {
        return;
    }
    if !scene.camera_ready {
        let Some(authored) = model.camera0 else {
            error!("UI_MainMenu has no authored camera 0");
            scene.failed = true;
            return;
        };
        let aspect = window
            .single()
            .map(|w| w.width() / w.height().max(1.0))
            .unwrap_or(4.0 / 3.0);
        let fwd = (authored.target - authored.eye).normalize_or_zero();
        let up = Quat::from_axis_angle(fwd, authored.roll) * Vec3::Y;
        if let Ok((mut transform, mut projection)) = camera.single_mut() {
            *transform = Transform::from_translation(authored.eye).looking_at(authored.target, up);
            *projection = Projection::from(PerspectiveProjection {
                fov: authored.fov / (1.0 + aspect * aspect).sqrt(),
                near: authored.near,
                far: authored.far.max(1000.0),
                ..default()
            });
            scene.camera_ready = true;
            info!(
                "UI_MainMenu: authored camera 0; {} render batches",
                model.submeshes.len()
            );
        } else {
            return;
        }
    }
    let end = (scene.next_batch + SCENE_BATCHES_PER_FRAME).min(model.submeshes.len());
    for index in scene.next_batch..end {
        crate::spawn_web_model_batch(
            &mut commands,
            &mut meshes,
            &mut materials,
            &model.submeshes[index],
            Transform::IDENTITY,
            format!("UI_MainMenu batch {index}"),
        );
    }
    scene.next_batch = end;
    if scene.next_batch == model.submeshes.len() {
        info!("UI_MainMenu static first frame READY: {} batches", end);
    }
}

fn load_sprite(assets: &AssetServer, path: &str) -> Handle<Image> {
    assets.load_with_settings(
        format!("mpq://{path}"),
        |settings: &mut BlpLoaderSettings| {
            settings.variant = BlpVariant::Sprite;
        },
    )
}

pub(super) fn drive(
    mut commands: Commands,
    assets: Res<AssetServer>,
    images: Res<Assets<Image>>,
    xmls: Res<Assets<FrameXmlSource>>,
    mut source: ResMut<LoginSource>,
    mut art: ResMut<GlueArt>,
    window: Query<&Window>,
    mut canvas: Query<&mut Node, With<LoginCanvas>>,
) {
    let scale = crate::original_glue::screen_scale(window.single().ok());
    if source.done {
        if let Ok(mut node) = canvas.single_mut() {
            node.width = Val::Px(1024.0 * scale);
            node.height = Val::Px(768.0 * scale);
        }
        return;
    }
    // AssetServer's browser reader is asynchronous. Wait for the shared client
    // button art before constructing the original widget tree; the stock
    // builder captures its image handles at spawn time.
    match assets.load_state(&source.button_up) {
        LoadState::Loaded => {
            if let Some(image) = images.get(&source.button_up) {
                art.button_up = Some((source.button_up.clone(), image.size_f32()));
            } else {
                return;
            }
        }
        LoadState::Failed(ref error) => {
            warn!("Glue button art unavailable; using stock widget fallback: {error}");
        }
        _ => return,
    }
    if let Some(image) = images.get(&source.button_down) {
        art.button_down = Some((source.button_down.clone(), image.size_f32()));
    }
    match assets.load_state(&source.xml) {
        LoadState::Loaded => {
            let Some(xml) = xmls.get(&source.xml) else {
                return;
            };
            let result = std::str::from_utf8(&xml.0)
                .map_err(|error| error.to_string())
                .and_then(parse_login);
            match result {
                Ok(chrome) => {
                    spawn_login(&mut commands, &assets, &art, &chrome, scale);
                    info!(
                        "Glue login READY: stock AccountLogin.xml, logo={}, account={:?}, password={:?}",
                        chrome.logo_file,
                        (chrome.account.x, chrome.account.y),
                        (chrome.password.x, chrome.password.y),
                    );
                }
                Err(error) => error!("Glue login XML failed: {error}"),
            }
            source.done = true;
        }
        LoadState::Failed(ref error) => {
            error!("Glue login asset failed: {error}");
            source.done = true;
        }
        _ => {}
    }
}

fn parse_login(xml: &str) -> Result<LoginChrome, String> {
    let document = framexml::parse(xml).map_err(|error| error.to_string())?;
    let root = document
        .items
        .iter()
        .find_map(|item| match item {
            TopLevel::Instance(element) if element.name() == Some("AccountLogin") => Some(element),
            _ => None,
        })
        .ok_or("AccountLogin ModelFFX is absent")?;
    let logo = named(root, "AccountLoginLogo")?;
    let account = named(root, "AccountLoginAccountEdit")?;
    let password = named(root, "AccountLoginPasswordEdit")?;
    let login = named(root, "AccountLoginLoginButton")?;
    let exit = named(root, "AccountLoginExitButton")?;
    let logo_file = logo
        .attr("file")
        .ok_or("login logo has no file")?
        .to_owned();
    let (logo_w, logo_h) = size(logo)?;
    let (logo_x, logo_y) = offset(logo)?;
    let (account_w, account_h) = size(account)?;
    let (account_x, account_y) = offset(account)?;
    let (password_w, password_h) = size(password)?;
    let (password_x, password_y) = offset(password)?;
    let (_, login_y) = offset(login)?;
    let (exit_x, exit_y) = offset(exit)?;
    Ok(LoginChrome {
        logo_file,
        logo: Rect {
            x: logo_x,
            y: -logo_y,
            w: logo_w,
            h: logo_h,
        },
        account: Rect {
            x: account_x,
            y: account_y,
            w: account_w,
            h: account_h,
        },
        password: Rect {
            x: password_x,
            y: password_y,
            w: password_w,
            h: password_h,
        },
        login_top: -login_y,
        exit_right: -exit_x,
        exit_bottom: exit_y,
    })
}

fn named<'a>(element: &'a Element, name: &str) -> Result<&'a Element, String> {
    if element.name() == Some(name) {
        return Ok(element);
    }
    element
        .children
        .iter()
        .find_map(|child| named(child, name).ok())
        .ok_or_else(|| format!("AccountLogin.xml lacks {name}"))
}

fn child<'a>(element: &'a Element, tag: &str) -> Result<&'a Element, String> {
    element
        .children
        .iter()
        .find(|child| child.tag.eq_ignore_ascii_case(tag))
        .ok_or_else(|| format!("{} lacks {tag}", element.name().unwrap_or(&element.tag)))
}

fn pair(element: &Element) -> Result<(f32, f32), String> {
    let x = element
        .attr("x")
        .ok_or("dimension lacks x")?
        .parse::<f32>()
        .map_err(|e| e.to_string())?;
    let y = element
        .attr("y")
        .ok_or("dimension lacks y")?
        .parse::<f32>()
        .map_err(|e| e.to_string())?;
    Ok((x, y))
}

fn size(element: &Element) -> Result<(f32, f32), String> {
    pair(child(child(element, "Size")?, "AbsDimension")?)
}

fn offset(element: &Element) -> Result<(f32, f32), String> {
    pair(child(
        child(child(child(element, "Anchors")?, "Anchor")?, "Offset")?,
        "AbsDimension",
    )?)
}

fn spawn_login(
    commands: &mut Commands,
    assets: &AssetServer,
    art: &GlueArt,
    chrome: &LoginChrome,
    s: f32,
) {
    let font: Handle<Font> = assets.load("mpq://Fonts/FRIZQT__.ttf");
    let logo_path = format!(
        "mpq://{}.blp",
        chrome.logo_file.replace('\\', "/").to_ascii_lowercase()
    );
    let logo: Handle<Image> =
        assets.load_with_settings(logo_path, |settings: &mut BlpLoaderSettings| {
            settings.variant = BlpVariant::Sprite;
        });
    commands
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            BackgroundColor(Color::NONE),
            GlobalZIndex(1000),
        ))
        .with_children(|screen| {
            screen
                .spawn((
                    Node {
                        width: Val::Px(1024.0 * s),
                        height: Val::Px(768.0 * s),
                        ..default()
                    },
                    LoginCanvas,
                ))
                .with_children(|ui| {
                    ui.spawn((
                        ImageNode::new(logo),
                        absolute(
                            s,
                            chrome.logo.x,
                            chrome.logo.y,
                            chrome.logo.w,
                            chrome.logo.h,
                        ),
                    ));
                    for (rect, caption, action) in [
                        (chrome.account, "Account Name", LoginAction::Account),
                        (chrome.password, "Account Password", LoginAction::Password),
                    ] {
                        ui.spawn((Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px((512.0 + rect.x - rect.w / 2.0) * s),
                            bottom: Val::Px(rect.y * s),
                            ..default()
                        },))
                            .with_children(|slot| {
                                glue_edit_box(
                                    slot,
                                    art,
                                    &font,
                                    (action, Button),
                                    LoginField,
                                    (rect.w, rect.h),
                                    (Color::srgb(0.3, 0.3, 0.3), Color::srgb(0.09, 0.09, 0.09)),
                                    (15.0, 0.0, 0.0, 5.0),
                                    s,
                                );
                            });
                        ui.spawn((
                            Text::new(caption),
                            TextFont {
                                font: font.clone(),
                                font_size: 15.0 * s,
                                ..default()
                            },
                            TextColor(Color::srgb(1.0, 0.78, 0.0)),
                            Node {
                                position_type: PositionType::Absolute,
                                left: Val::Px((512.0 + rect.x - 76.0) * s),
                                bottom: Val::Px((rect.y + rect.h + 9.0) * s),
                                ..default()
                            },
                        ));
                    }
                    ui.spawn((Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px((512.0 + 8.0 - 85.0) * s),
                        top: Val::Px(chrome.login_top * s),
                        ..default()
                    },))
                        .with_children(|slot| {
                            glue_button(
                                slot,
                                art,
                                &font,
                                LoginAction::Login,
                                "Login",
                                170.0,
                                45.0,
                                GlueBtnKind::Normal,
                                s,
                            );
                        });
                    ui.spawn((Node {
                        position_type: PositionType::Absolute,
                        right: Val::Px(chrome.exit_right * s),
                        bottom: Val::Px(chrome.exit_bottom * s),
                        ..default()
                    },))
                        .with_children(|slot| {
                            glue_button(
                                slot,
                                art,
                                &font,
                                LoginAction::Quit,
                                "Quit",
                                150.0,
                                38.0,
                                GlueBtnKind::Small,
                                s,
                            );
                        });
                });
        });
}

fn absolute(s: f32, left: f32, top: f32, width: f32, height: f32) -> Node {
    Node {
        position_type: PositionType::Absolute,
        left: Val::Px(left * s),
        top: Val::Px(top * s),
        width: Val::Px(width * s),
        height: Val::Px(height * s),
        ..default()
    }
}
