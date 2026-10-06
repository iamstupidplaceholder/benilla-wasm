//! Opt-in nonvisual gates call the actual target systems, not a browser picker.

use super::*;

pub fn verify_target_host_boundary() {
    catalogs::verify_delivery();

    use benilla_protocol::EntityKind;
    use benilla_world::{interact::CreaturePickPart, view::WorldCamera};
    use bevy::camera::{
        primitives::Aabb, CameraProjection, ComputedCameraValues, RenderTargetInfo,
    };
    use bevy::window::PrimaryWindow;

    let mut world = World::new();
    world.init_resource::<Hovered>();
    world.init_resource::<HoveredObject>();
    world.init_resource::<PickOcclusion>();
    world.init_resource::<cursor_mode::WorldCursor>();
    world.init_resource::<PressPick>();
    world.init_resource::<ButtonInput<MouseButton>>();
    world.init_resource::<crate::player::CameraControl>();
    world.init_resource::<crate::ui_script::PointerOverUi>();
    world.init_resource::<crate::vplates::PlateHover>();
    world.init_resource::<crate::net::SelfGuid>();
    world.init_resource::<Assets<Mesh>>();
    world.init_resource::<benilla_world::rig_palette::RigPalettes>();

    let mut window = Window::default();
    window.resolution.set(800.0, 600.0);
    window.set_cursor_position(Some(Vec2::new(400.0, 300.0)));
    world.spawn((window, PrimaryWindow));
    let mut projection = PerspectiveProjection::default();
    projection.aspect_ratio = 800.0 / 600.0;
    world.spawn((
        Camera {
            computed: ComputedCameraValues {
                clip_from_view: projection.get_clip_from_view(),
                target_info: Some(RenderTargetInfo {
                    physical_size: UVec2::new(800, 600),
                    scale_factor: 1.0,
                }),
                ..default()
            },
            ..default()
        },
        GlobalTransform::IDENTITY,
        WorldCamera,
    ));
    let mut spawn_unit = |guid, z| {
        let root = world
            .spawn((
                Guid(guid),
                crate::net::NetEntity {
                    kind: EntityKind::Unit,
                    display_id: None,
                    scale: 1.0,
                },
                GlobalTransform::from_translation(Vec3::new(0.0, 0.0, z)),
            ))
            .id();
        world.spawn((
            ChildOf(root),
            CreaturePickPart,
            Aabb::from_min_max(Vec3::splat(-0.5), Vec3::splat(0.5)),
            GlobalTransform::from_translation(Vec3::new(0.0, 0.0, z)),
        ));
        root
    };
    let near = spawn_unit(0x1111, -5.0);
    let far = spawn_unit(0x2222, -10.0);
    let hover = world.register_system(hover::update_hover);
    let latch = world.register_system(latch_press_pick);
    world.run_system(hover).unwrap();
    assert_eq!(world.resource::<Hovered>().target, Some(near));
    assert_eq!(world.resource::<Hovered>().guid, Some(0x1111));
    let distance = world.resource::<Hovered>().distance;
    assert!(distance.is_finite() && distance > 0.0);

    // Production strict occlusion comparison: a tie keeps the target.
    world.resource_mut::<PickOcclusion>().distance = distance;
    world.run_system(hover).unwrap();
    assert_eq!(world.resource::<Hovered>().target, Some(near));
    world.resource_mut::<PickOcclusion>().distance = distance - 0.1;
    world.run_system(hover).unwrap();
    assert!(world.resource::<Hovered>().target.is_none());
    world.resource_mut::<PickOcclusion>().distance = f32::INFINITY;

    world.run_system(hover).unwrap();
    world
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(MouseButton::Left);
    world.run_system(latch).unwrap();
    assert_eq!(world.resource::<PressPick>().hovered.guid, Some(0x1111));
    world.resource_mut::<crate::ui_script::PointerOverUi>().0 = true;
    world.run_system(hover).unwrap();
    assert!(
        world.resource::<Hovered>().target.is_none(),
        "UI owns this pointer"
    );
    world.resource_mut::<ButtonInput<MouseButton>>().clear();
    world.run_system(latch).unwrap();
    assert_eq!(
        world.resource::<PressPick>().hovered.guid,
        Some(0x1111),
        "held gesture retains its press pick"
    );

    world.resource_mut::<crate::ui_script::PointerOverUi>().0 = false;
    world.despawn(near);
    world.run_system(hover).unwrap();
    assert_eq!(
        world.resource::<Hovered>().target,
        Some(far),
        "real despawn releases the nearest candidate"
    );
}
