//! Original tick/paint schedule, without reference files or a GPU/world fixture.
//! Checks the shared host SystemParam admission and original clip/settled contracts.

use super::*;

pub fn verify_ui_paint_host_boundary() {
    let mut script = UiScript::new().unwrap();
    script
        .run(
            r#"
        local frame = CreateFrame("ScrollFrame", "PaintHostGate")
        frame:SetPoint("TOPLEFT", 0, -100)
        frame:SetWidth(300); frame:SetHeight(200)
        local child = CreateFrame("Frame", "PaintHostChild")
        child:SetWidth(300); child:SetHeight(600)
        frame:SetScrollChild(child)
        PaintHostMarker = child:CreateTexture(nil, "ARTWORK")
        PaintHostMarker:SetTexture(1, 0, 0)
        PaintHostMarker:SetAllPoints()
    "#,
        )
        .unwrap();
    let mut app = App::new();
    app.insert_non_send_resource(script)
        .init_resource::<UiQuads>()
        .init_resource::<super::super::UiPassState>()
        .init_resource::<Assets<Image>>()
        .init_resource::<crate::portrait::PortraitImages>()
        .init_resource::<crate::portrait::BoothPanes>()
        .init_resource::<crate::ui_models::UiModelTiles>()
        .init_resource::<crate::minimap::MinimapWidget>()
        .init_resource::<super::super::UiFrameCost>()
        .init_resource::<super::super::UiCostWanted>()
        .init_resource::<Time>()
        .init_resource::<Time<Real>>()
        .init_resource::<super::super::UiClock>()
        .init_resource::<super::super::UiScaleCvar>();
    app.world_mut().spawn((
        Window {
            resolution: UVec2::new(1024, 768).into(),
            ..default()
        },
        PrimaryWindow,
    ));
    app.add_systems(Update, (tick_script, paint_script).chain());
    app.update();
    let quads = &app.world().resource::<UiQuads>().quads;
    let marker = quads
        .iter()
        .find(|q| q.color == [1.0, 0.0, 0.0, 1.0])
        .unwrap();
    assert!(marker.texture.is_none());
    assert_eq!(marker.clip, Some(Rect::new(0.0, 100.0, 300.0, 300.0)));
    app.world_mut().resource_mut::<UiQuads>().dirty = false;
    app.update();
    assert!(
        !app.world().resource::<UiQuads>().dirty,
        "settled native paint must still skip"
    );
    app.world_mut()
        .non_send_resource_mut::<UiScript>()
        .run("PaintHostMarker:SetTexture(0, 1, 0)")
        .unwrap();
    app.update();
    let quads = app.world().resource::<UiQuads>();
    assert!(quads.dirty);
    assert!(quads.quads.iter().any(|q| q.color == [0.0, 1.0, 0.0, 1.0]));
}
