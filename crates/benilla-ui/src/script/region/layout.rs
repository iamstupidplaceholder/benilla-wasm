//! Region method-table cluster: **layout** — size, anchors and the resolved-rect readers.
//! Split out of `region.rs` at the 0716 file-size budget.

use benilla_lua::{MultiValue as LuaMultiValue, Table as LuaTable, Value as LuaValue, Vm};

use crate::layout::{Anchor, Point};
use crate::script::object::anchor_args::{
    parse_set_all_points_context, resolve_rel_target_context,
};
use crate::script::object::{anchor_bits_eq, frame_wrapper_context, point_name};
use crate::script::region_map::{set_shared, Side};
use crate::script::{Model, SCREEN};

/// Resolve `self` (a region wrapper) to its live [`RegionHandle`].
use super::{
    measured_wh_context, region_handle_of_context, region_ladder_context_context, region_owner_id,
    region_set_point_context, region_wrapper_context, size_bits_eq,
};

pub(super) fn install_context(vm: &mut Vm, m: &LuaTable) -> benilla_lua::Result<()> {
    // Region explicit size — fills the axes the region's anchors don't pin (unread under an
    // implicit SetAllPoints's two corners; decision 1310).
    set_shared(
        vm,
        m,
        Side::Region,
        "SetWidth",
        |lua, (this, w): (LuaTable, f32)| {
            let rh = region_handle_of_context(lua, &this)?;
            let mut model = lua.app_data_mut::<Model>().expect("model");
            let d = model.region_data.entry(rh).or_default();
            let new = Some((w, d.size.map_or(0.0, |s| s.1)));
            let changed = !size_bits_eq(d.size, new);
            d.size = new;
            if changed {
                // A size write moves no edge and no roster membership (decision 1388) — and on
                // a FontString the width is the WRAP width, a measure-key input, so it names
                // itself on the measure ledger too.
                model.touch_layout_region(rh);
                model.touch_measure(rh);
            }
            Ok(())
        },
    )?;

    set_shared(
        vm,
        m,
        Side::Region,
        "SetHeight",
        |lua, (this, h): (LuaTable, f32)| {
            let rh = region_handle_of_context(lua, &this)?;
            let mut model = lua.app_data_mut::<Model>().expect("model");
            let d = model.region_data.entry(rh).or_default();
            let new = Some((d.size.map_or(0.0, |s| s.0), h));
            let changed = !size_bits_eq(d.size, new);
            d.size = new;
            if changed {
                // A size write moves no edge and no roster membership (decision 1388) — and on
                // a FontString the width is the WRAP width, a measure-key input, so it names
                // itself on the measure ledger too.
                model.touch_layout_region(rh);
                model.touch_measure(rh);
            }
            Ok(())
        },
    )?;

    // **No `SetSize`** — the frame twin's note in `object/layout_methods.rs` applies here
    // unchanged: an Era verb 1.12's Region map does not carry (decision 2142).

    set_shared(vm, m, Side::Region, "GetWidth", |lua, this: LuaTable| {
        Ok(measured_wh_context(lua, &this)?.0)
    })?;

    set_shared(vm, m, Side::Region, "GetHeight", |lua, this: LuaTable| {
        Ok(measured_wh_context(lua, &this)?.1)
    })?;

    // GetLeft/GetRight/GetTop/GetBottom — the region's RESOLVED edges in its OWNER's units (y-up;
    // screen ÷ the owner's effective scale, the frame twin's law in `object/layout_methods.rs` —
    // a region shares its owner's scale, and a texture inside the scaled world map answered
    // screen units here while its owner answered local ones, decision 1985). Every drawable
    // region carries anchors (authored or the creation-path implicit anchor, decision 1310) and
    // reads its resolved rect; a templateless Lua region nobody anchored never resolves → nil,
    // same as pre-resolve.
    for (name, pick) in [
        ("GetLeft", 0u8),
        ("GetRight", 1u8),
        ("GetTop", 2u8),
        ("GetBottom", 3u8),
    ] {
        set_shared(vm, m, Side::Region, name, move |lua, this: LuaTable| {
            let rh = region_handle_of_context(lua, &this)?;
            let model = lua.app_data_ref::<Model>().expect("model");
            let inv = 1.0 / owner_scale(&model, rh);
            Ok(model.region_resolved.get(&rh).map(|r| {
                inv * match pick {
                    0 => r.left,
                    1 => r.right,
                    2 => r.top,
                    _ => r.bottom,
                }
            }))
        })?;
    }

    // Region anchors: SetPoint/ClearAllPoints/SetAllPoints mirror the frame versions
    // ([`super::object`]) but write [`super::RegionData::anchors`]. An unspecified `relativeTo`
    // defaults to the **owner frame**; a named one may be a frame or a sibling region (the real
    // XML anchors regions to sibling regions everywhere — merchant label plate → `$parentSlot`).
    set_shared(
        vm,
        m,
        Side::Region,
        "SetPoint",
        |lua, (this, rest): (LuaTable, LuaMultiValue)| region_set_point_context(lua, &this, &rest),
    )?;

    set_shared(
        vm,
        m,
        Side::Region,
        "ClearAllPoints",
        |lua, this: LuaTable| {
            let rh = region_handle_of_context(lua, &this)?;
            let mut model = lua.app_data_mut::<Model>().expect("model");
            let d = model.region_data.entry(rh).or_default();
            // Names its node, the frame twin's rule and for the frame twin's reason (decision
            // 2114): clearing every anchor is a retarget onto an EMPTY target list, so the cached
            // graph's edges are unlinked rather than the whole graph re-derived.
            let old: Option<Vec<u32>> =
                (!d.anchors.is_empty()).then(|| d.anchors.iter().map(|a| a.relative_to).collect());
            d.anchors.clear();
            if let Some(old) = old {
                model.touch_layout_retarget_region(rh, &old, &[]);
            }
            Ok(())
        },
    )?;

    set_shared(
        vm,
        m,
        Side::Region,
        "SetAllPoints",
        |lua, (this, rest): (LuaTable, LuaMultiValue)| {
            let rh = region_handle_of_context(lua, &this)?;
            // `who`/`$parent` first, then the `_G` read, then the guard — `region_ladder_context`.
            let (who, base) = region_ladder_context_context(lua, rh);
            let target = parse_set_all_points_context(lua, rest.first(), &base);
            let mut model = lua.app_data_mut::<Model>().expect("model");
            let me = model.region_id(rh);
            let owner = region_owner_id(&mut model, rh);
            let rel_id =
                resolve_rel_target_context(&model, &target, &who, "SetAllPoints", me, owner)?;
            let pair = [
                Anchor::new(Point::TopLeft, rel_id, Point::TopLeft, 0.0, 0.0),
                Anchor::new(Point::BottomRight, rel_id, Point::BottomRight, 0.0, 0.0),
            ];
            let data = model.region_data.entry(rh).or_default();
            let same = data.anchors.len() == 2
                && data
                    .anchors
                    .iter()
                    .zip(&pair)
                    .all(|(a, b)| anchor_bits_eq(a, b));
            if !same {
                let old: Vec<u32> = data.anchors.iter().map(|a| a.relative_to).collect();
                data.anchors.clear();
                data.anchors.extend_from_slice(&pair);
                // Reuse the same exact edge patch as SetPoint/ClearAllPoints;
                // missing region nodes still take the conservative full derive.
                model.touch_layout_retarget_region(rh, &old, &[rel_id, rel_id]);
            }
            Ok(())
        },
    )?;

    // ── The rest of the Region map `0xcf54b4` (wow-re `font-object-lua-surface.md`) ──────────────
    //
    // These four landed together because the MAP is the unit, not the name. `SetParent` shipped
    // alone when one addon line named it, and its own getter stayed missing for months — which is
    // how `TheoryCraft\TheoryCraftUI.lua:720` (`buttontext:GetParent()`, a FontString) died every
    // session, and how the per-kind census came to read `115 GetParent (missing on Texture,
    // FontString)`. The set is closed and byte-verified, so it is asserted as a set in
    // `tests/reference_surface.rs` rather than grown a name at a time.

    // GetParent() → the OWNER frame's wrapper. A region always has one (`region_owner_id` falls
    // back to the owner for every unresolved case), so unlike the frame twin this never answers nil.
    set_shared(vm, m, Side::Region, "GetParent", |lua, this: LuaTable| {
        let rh = region_handle_of_context(lua, &this)?;
        let owner = {
            let mut model = lua.app_data_mut::<Model>().expect("model");
            region_owner_id(&mut model, rh)
        };
        frame_wrapper_context(lua, owner)
    })?;

    // GetCenter() → the resolved rect's midpoint, or a nil PAIR before the first resolve — the same
    // contract, and the same source, as the GetLeft/GetRight/GetTop/GetBottom readers above.
    //
    // **Deliberately unscaled, where the frame twin divides by GetEffectiveScale.** The region edge
    // readers report raw resolved units; scaling only the centre would make `GetCenter()` disagree
    // with `(GetLeft() + GetRight()) / 2` on any scaled subtree — a contradiction inside one method
    // table is worse than a missing division, and regions have no scale of their own to divide by.
    set_shared(vm, m, Side::Region, "GetCenter", |lua, this: LuaTable| {
        let rh = region_handle_of_context(lua, &this)?;
        let model = lua.app_data_ref::<Model>().expect("model");
        let inv = 1.0 / owner_scale(&model, rh);
        Ok(match model.region_resolved.get(&rh) {
            Some(r) => (
                LuaValue::Number(f64::from(inv * (r.left + r.right) * 0.5)),
                LuaValue::Number(f64::from(inv * (r.bottom + r.top) * 0.5)),
            ),
            None => (LuaValue::Nil, LuaValue::Nil),
        })
    })?;

    // GetNumPoints() → how many anchors this region carries. Absent on our FRAMES too, which is the
    // same drift one table up; this side is what the corpus named.
    set_shared(
        vm,
        m,
        Side::Region,
        "GetNumPoints",
        |lua, this: LuaTable| {
            let rh = region_handle_of_context(lua, &this)?;
            let model = lua.app_data_ref::<Model>().expect("model");
            Ok(model
                .region_data
                .get(&rh)
                .map_or(0, |d| d.anchors.len() as i64))
        },
    )?;

    // GetPoint([n]) → point, relativeTo, relativePoint, xOfs, yOfs — the n-th (1-based, default
    // first) anchor, mirroring the frame twin including its out-of-range answer (five nils).
    //
    // **The relativeTo dispatch is the part a frame-shaped copy gets wrong.** Region anchors live in
    // ONE id space with frames and may target a sibling REGION — the real XML does it everywhere
    // (`region.rs`'s `resolve_target`: frames first, then the region-name registry). So the id is
    // matched against both tables and answered with the matching wrapper kind; handing back a frame
    // wrapper for a region id would be a working-looking handle onto the wrong object.
    set_shared(
        vm,
        m,
        Side::Region,
        "GetPoint",
        |lua, (this, n): (LuaTable, Option<i64>)| {
            let rh = region_handle_of_context(lua, &this)?;
            let anchor = {
                let model = lua.app_data_ref::<Model>().expect("model");
                let idx = (n.unwrap_or(1).max(1) - 1) as usize;
                model
                    .region_data
                    .get(&rh)
                    .and_then(|d| d.anchors.get(idx))
                    .cloned()
            };
            let Some(a) = anchor else {
                return Ok((
                    LuaValue::Nil,
                    LuaValue::Nil,
                    LuaValue::Nil,
                    LuaValue::Nil,
                    LuaValue::Nil,
                ));
            };
            let rel = {
                let is_frame = {
                    let model = lua.app_data_ref::<Model>().expect("model");
                    model.id_to_frame.contains_key(&a.relative_to)
                };
                if a.relative_to == SCREEN {
                    LuaValue::Nil
                } else if is_frame {
                    LuaValue::Table(frame_wrapper_context(lua, a.relative_to)?)
                } else {
                    LuaValue::Table(region_wrapper_context(lua, a.relative_to)?)
                }
            };
            Ok((
                LuaValue::from(point_name(a.point)),
                rel,
                LuaValue::from(point_name(a.relative_point)),
                LuaValue::Number(f64::from(a.x_off)),
                LuaValue::Number(f64::from(a.y_off)),
            ))
        },
    )?;
    Ok(())
}

/// The region's owner frame's effective scale (1 for an orphan) — the divisor that turns a
/// resolved screen rect into the owner's units, the space every region getter answers in.
fn owner_scale(model: &Model, rh: crate::widget::RegionHandle) -> f32 {
    model
        .arena
        .region(rh)
        .map(|r| crate::script::object::eff_scale(model, r.owner))
        .unwrap_or(1.0)
}
