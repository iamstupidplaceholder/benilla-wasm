//! Region method-table cluster: **paint** — what a Texture shows and how it is tinted,
//! blended, cropped and layered. Split out of `region.rs` at the 0716 file-size budget.

use benilla_lua::{
    MultiValue as LuaMultiValue, Table as LuaTable, Value as LuaValue, Variadic as LuaVariadic, Vm,
};

use crate::script::object::{draw_layer_from_str, draw_layer_name};
use crate::script::{BlendMode, Model, TexCoords};

/// Resolve `self` (a region wrapper) to its live [`RegionHandle`].
use super::{region_flag, region_handle_of_context};

// SetAlpha/GetAlpha — the region's own alpha, distinct from the owner frame's. The ref kit reads
// it back to ramp a texture (CastingBarFrame's completion flash does
// `CastingBarFlash:SetAlpha(GetAlpha() + CASTING_BAR_FLASH_STEP)`), so the getter must return the
// region's value, never the frame's. Draw law + the open question: [`RegionData::alpha`].
// **KNOWN DIVERGENCE, byte-pinned and deliberately not closed here (decision 1782).** An
// ABSENT alpha argument is 1.0 below; in the reference it is *the alpha already on the region*.
// `0x79abd0` reads the current colour back (`0x79ac81` -> `0x77f8c0`), packs the caller's rgb
// against an `a` default of 1.0 (`0x79ad43`), and then asks a SECOND time whether argument 5
// was present — when it was not, it copies byte 3 of the read-back colour over the packed
// alpha (`0x79adfe`/`0x79ae0a`) before the store. So `SetVertexColor(r, g, b)` on 1.12.1 is
// `SetVertexColor(r, g, b, currentAlpha)`, and a three-argument call CANNOT restore opacity.
// wow-re `system/ui/scratch/button-state-texture-path-setter.md` §7.
//
// Why it still says 1.0: the fix is one line, but its blast radius is every three-argument
// call in FrameXML and in the addon corpus, and it only bites where a region already carries
// alpha < 1 — where it would surface as art going permanently translucent, which is a LOOK and
// therefore the director's call, not a quiet correction to fold into an unrelated slice. The
// known live case is the action bar's grid ring: stock `ActionButton_ShowGrid` passes the
// fourth argument explicitly (`SetVertexColor(1.0, 1.0, 1.0, 0.5)`), so closing this cannot
// silently change that file.
//
// **Shape C on r, g, b** (`Texture:SetVertexColor `0x79abd0``, `2=C 3=C 4=C 5=B`, wow-re
// `numeric-arg-coercion-law.md`): a bare `lua_tonumber` with no `lua_isnumber` gate, so a nil,
// a table or a string is **0.0** and the call never raises. Taking them as `f32` made mlua's
// converter the gate instead — the 2176 class — and the stock
// `QuestLogFrame.lua:337` idiom hands three nils (`titleButton.r/g/b` are only assigned in
// `QuestLog_Update`) on any path that selects a quest-log entry before the window has painted.
// GetVertexColor — the setter's own pair, a real 5875 binding (`0x79aa50`, wow-re
// `system/ui/ledger.tsv`; it sits directly above `SetVertexColor 0x79abd0` in the same region
// method family). Never set = the untinted white every region draws at by default.
// SetGradientAlpha(orientation, r1,g1,b1,a1, r2,g2,b2,a2) and its alpha-less twin
// SetGradient(orientation, r1,g1,b1, r2,g2,b2) — the two-stop linear gradient the client
// generates into the same `+0xcc` texture slot the colour form of SetTexture fills.
//
// These were missing, and they were the single wall in front of the corpus's largest family:
// `FuBar\FuBar_Panel.lua:144` calls SetGradientAlpha while building the bar, so all 20 FuBar
// plugins died there — after the debugstack/chunk-name fix got them that far.
//
// The orientation token is matched case-insensitively and anything that is not "VERTICAL" is
// horizontal, which is the client's own leniency and matters because addons spell it both ways.
// The gradient is stored WHOLE (see `RegionData::gradient`); the paint folds it to its midpoint
// because a quad carries one tint today. That approximation is visible and is recorded at the
// field, not hidden here.

// SetDesaturated(flag) -> shaderSupported — Texture only (`0x79c1e0`, wow-re ledger; the
// reference's own `ItemButtonTemplate.lua:69` is `local shaderSupported =
// icon:SetDesaturated(desaturated)`).
//
// **The RETURN is the whole design, and it is the half a plausible implementation drops.**
// 1.12 ran on cards that could not do the shader, so the verb reports whether it took effect
// and FrameXML falls back by hand:
//
//     if ( not desaturated ) then r,g,b = 1,1,1
//     elseif ( not r or not shaderSupported ) then r,g,b = 0.5,0.5,0.5 end
//     icon:SetVertexColor(r, g, b)
//
// The renderer now greys the texel (decision 1327 — `ui_quad.wgsl`'s luminance fold), so we
// answer **1 — supported**, and the branch above takes its shader arm: the icon goes greyscale
// AND wears the caller's own dim tint. Until 1327 this answered nil, which is a real 1.12
// card's answer and was the honest one while nothing greyed — but it costs the *look*: a 0.65
// grey multiply on colourful art is a dimmer, still-colourful icon, which is precisely what
// B162 reported against the talent tree.
//
// Why it matters far past one verb: **98 of the 109 addons that draw and then raise on being
// used, raise here** — `FuBar_Panel.lua:43`'s right-click reaches Dewdrop's `AddLine`
// (`Dewdrop-2.0.lua:2172`), which calls `button.arrow:SetDesaturated(true)` unguarded. A
// static scan costed this at 61 addons and it was declined; the use-probe costed it at 98 the
// moment anyone right-clicks.
//
// **The argument's truth table is not `if flag then`** (wow-re `texture-desaturate-law.md` §1.1,
// VERIFIED — `0x6f1c10(L, 2, default=1)` dispatched through the jump table at `0x6f1ce8`). Two
// of its arms are the opposite of the obvious reading, and both are reachable:
//  · **no argument at all is ON**, not off — `LUA_TNONE` takes the `ja` default arm, so
//    `tex:SetDesaturated()` greys. This is why the flag arrives as a `MultiValue`: mlua hands a
//    missing `Value` parameter through as `Nil`, which is the one arm that means OFF.
//  · **a number truncating to 0 is OFF** (`0x6f3620`+`0x40a2b0`), so `SetDesaturated(0)` clears.
//  · a table/function/userdata is ON (same default arm as absent).
// The string arm (`0x6f1c51`, comparing against `0x871460`/`0x853758`) is NOT modelled — no
// corpus caller passes one, and the comparands were not read; a string takes the ON arm here.
// GetTexture() — Texture only (`0x79ba70`), verified in wow-re's widget-method batch
// (`system/ui/scratch/widget-api-batch-benilla.md`). Three contract details are each the kind a
// plausible implementation gets silently wrong, so each is spelled out:
//
//  · **Exactly ONE return value**, never a multi-return.
//  · **The colour form returns the literal string `"Solid Texture"`** (`0x835708`), NOT nil.
//    `SetTexture(r,g,b)` synthesizes an 8x8 solid, and the getter reports that name — so an
//    addon's `if not tex then` passes straight through on a colour-filled region. Returning nil
//    here would look tidier and would be wrong in the one direction callers test for.
//  · **The path is stripped at the LAST `.`** (`0x79baf0`): the loader appends `.blp`/`.tga` to
//    what was set and the getter strips an extension back off. Taken verbatim rather than
//    "strip only a real extension" — a directory containing a dot is mangled by the real client
//    too, and this surface is a transcription, not an improvement.
//
// Four corpus addons: `AtlasQuest.lua:228` (`AQATLASMAP = AtlasMap:GetTexture()`) and
// `FuBarPlugin-2.0.lua:343` (`return self.iconFrame:GetTexture()`), each reached by two addons.
// SetAlphaGradient(start, length) — the per-character write-on reveal (CSimpleFontString;
// the quest-description machinery, ref QuestFrame.lua:548/558). Returns whether `start` is
// still inside the text (chars) — the ref's OnUpdate loop advances until this goes false.
// SetBlendMode("DISABLE"|"ALPHAKEY"|"BLEND"|"ADD"|"MOD") — `0x79a950`, the shared alphaMode
// enum `0x811aa8`. Only ADD changes draw behavior in v1 (DISABLE/ALPHAKEY/MOD are accepted as
// straight alpha, a stated gap — see [`BlendMode`]); an unrecognised name leaves the mode
// alone, which is what the enum-table lookup does with a string it cannot match.
//
// The mode is STORED as the mode. It used to collapse to `additive = (mode == "ADD")` on the
// way in, which is everything the renderer needs and strictly less than `GetBlendMode` has to
// answer — a texture set to `"MOD"` would have read back as `"BLEND"`.
// GetBlendMode() — ONE string, the enum's own spelling (`0x79a890`, table `0x87c128`, argc 1,
// arity 1, kinds `(string?)`). A Texture nothing has called the setter on answers `"BLEND"`,
// the CSimpleTexture ctor's `[+0xd0] = 2` (`0x76fc64`).
//
// **This one was answering nil, and nil is not an error here — it is a wrong picture.**
// `ShaguTweaks/mods/dark-ui-elements.lua:169` reads
// `elseif region.GetBlendMode and region:GetBlendMode() == "ADD" then` while recolouring a
// Blizzard frame's children: the `and` guard means a missing method never raised, it just took
// the other branch, so every additive texture in the frame got the dark recolour the reference
// leaves alone. The kinds column's `string?` is the reference's own nil leg (the name pointer
// can be NULL for a mode outside the table); the five modes are all this engine can hold, so
// nothing here reaches it.
// SetTexCoordModifiesRect(flag) / GetTexCoordModifiesRect() — `0x79c080` / `0x79c120`, table
// `0x87c128`. The setter writes the reference's `[texture+0x124]` and is its only writer; the
// getter answers `1`/`nil` (kinds `(nil) | (number)`, the predicate law — decision 2118 — not a
// Lua boolean).
//
// **The flag's GEOMETRY effect is not wired, and that is stated rather than implied.** In the
// reference the flag gates a rect-recompute leg (`ui.md:4619`, `0x770462`): with it set, a
// `SetTexCoord` re-derives the region's own rect from the UV quad instead of leaving the rect
// where the anchors put it and resampling inside it. Wiring that means the region resolve in
// `region::layout` reading this flag and taking the rect from `RegionData::tex_coords` — a
// resolve-order change. Nothing in the stock UI or in either addon corpus calls the SETTER, so
// there is no measured case to build it against; what has a caller is the GETTER —
// `pfUI/modules/thirdparty-tbc.lua:319` does a bare `if icon:GetTexCoordModifiesRect() then` on
// a Texture to choose between two `SetTexCoord` rectangles, and against this VM that raised.
// So: the flag is stored, answered truthfully, and read by nothing. See
// [`RegionData::tex_coord_modifies_rect`].
// GetDrawLayer() — the setter's pair, present on BOTH region leaves in the client (Texture
// `0x79a6c0`, FontString `0x79c660`, each its own copy — read off the method-table pair bytes).
//
// This module's own note used to end "GetDrawLayer is in the client's pair and absent here;
// absent is absent" — accurate, and a gap rather than a decision. pfUI's `GetNoNameObject`
// (api/api.lua:1342) reads it off every child while reskinning a Blizzard frame, and died
// there; that is the measurement (1719).
//
// Returns the layer NAME alone. The sub-level we store beside it is not a second return here:
// 1.12's pair is layer-only, and `SetDrawLayer`'s optional sub-level argument above is already
// marked as this engine's extension point.
// `SetRotation` WAS here, on the Texture leaf, and is GONE. 1.12 registers the name once, in
// the **PlayerModel** table `0x84f1fc` (`0x505f00`, argc 2 — the paper doll's rotate arrows),
// which we already answer through `modelframe`; it is in neither region map, and the carve
// above lists it among the five names that are in NEITHER (`texture-fontstring-method-split.md`).
// Ours was a later-era Texture verb shipped early for the world-map player arrow's stand-in
// rotation — and that arrow has since become a real Model frame driven by `ModelState::facing`
// (`script::worldmap_arrow`), so the verb's own reason went with it.
//
// Removal is safe by census, not by assumption: every `SetRotation` in this repo, in
// `assets/ui`, in the stock FrameXML/GlueXML and in both addon corpora has a MODEL receiver —
// `pfUI/api/ui-widgets.lua:580`'s `EnableClickRotate` hooks a modelframe's OnUpdate, and
// `CustomNameplates/options.lua:308` is `optionsFrame.preview.model:SetRotation(0.61)`. Not one
// texture receiver anywhere.
//
// `RegionData::rotation` and `QuadContent::Texture::rotation` now have no writer left. They are
// deliberately NOT pruned in the same change: that plumbing runs into the app's quad emit and
// its removal is a wider prune with its own review, not a tail on a method-surface fix.

// SetTexCoord(left, right, top, bottom) — the 4-edge form (XML `<TexCoords>`): a UV sub-rect in
// 0..1 texture space (top-left origin) the Texture region samples, slicing quadrant/atlas art
// (decision 0084). SetTexCoord(ULx,ULy, LLx,LLy, URx,URy, LRx,LRy) — the 8-arg affine form: an
// arbitrary UV quad (rotation/shear — the reference's `DrawRouteLine` route lines), stored per
// corner in the renderer's screen winding.
// GetTexCoord() → left, right, top, bottom (the 4-edge form; full texture if never set; an
// affine mapping reports its bounding edges).
/// Paint state shared by Texture and FontString, installed through Benilla's VM boundary.
///
/// These are the original callbacks from above, moved as one state-only group. Keeping the
/// method table itself unchanged preserves the existing Texture/FontString surface and wrapper
/// identity while making the callback bodies available to either Lua backend.
pub(super) fn install_context(vm: &mut Vm, methods: &LuaTable) -> benilla_lua::Result<()> {
    let set_texture = vm.create_function(
        "Region:SetTexture",
        |lua,
         (this, argument, green, blue, alpha): (
            LuaTable,
            LuaValue,
            LuaValue,
            LuaValue,
            LuaValue,
        )| {
            let rh = region_handle_of_context(lua, &this)?;
            let (loaded, content_derived_size) = {
                let mut model = lua.app_data_mut::<Model>().expect("model");
                let data = model.region_data.entry(rh).or_default();
                data.circular = false;
                data.portrait_unit = None;

                let same_path = matches!((&argument, &data.texture),
                        (LuaValue::String(path), Some(current))
                            if path.as_str().is_ok_and(|path| path == current.as_str()));
                let colour_form = matches!(&argument, LuaValue::Number(_) | LuaValue::Integer(_));
                if !same_path && !colour_form {
                    data.desaturated = false;
                }

                let content_derived_size = !data.anchors.is_empty()
                    && data
                        .size
                        .is_none_or(|(width, height)| width == 0.0 || height == 0.0);
                let loaded = match &argument {
                    LuaValue::String(path) if strict_lua_string(path)?.is_empty() => {
                        data.texture = None;
                        data.fill = None;
                        false
                    }
                    LuaValue::String(path) => {
                        let path = strict_lua_string(path)?.to_string();
                        drop(model);
                        let resolvable = {
                            let model = lua.app_data_ref::<Model>().expect("model");
                            model
                                .texture_probe
                                .as_ref()
                                .is_none_or(|probe| probe(&path))
                        };
                        let mut model = lua.app_data_mut::<Model>().expect("model");
                        let had_probe = model.texture_probe.is_some();
                        if resolvable {
                            let data = model.region_data.entry(rh).or_default();
                            data.texture = Some(path);
                            data.fill = None;
                        }
                        resolvable && had_probe
                    }
                    LuaValue::Number(_) | LuaValue::Integer(_) => {
                        data.fill = Some([
                            as_f32_context(&argument),
                            texture_channel_context(&green, 0.0),
                            texture_channel_context(&blue, 0.0),
                            texture_channel_context(&alpha, 1.0),
                        ]);
                        data.texture = None;
                        true
                    }
                    LuaValue::Nil => {
                        data.texture = None;
                        data.fill = None;
                        true
                    }
                    _ => false,
                };
                (loaded, content_derived_size)
            };

            if content_derived_size {
                lua.app_data_mut::<Model>()
                    .expect("model")
                    .touch_layout_region(rh);
            }
            Ok(if loaded {
                LuaValue::Number(1.0)
            } else {
                LuaValue::Nil
            })
        },
    )?;
    vm.table_set_function(&methods, "SetTexture", &set_texture)?;

    let set_alpha =
        vm.create_function("Region:SetAlpha", |lua, (this, alpha): (LuaTable, f32)| {
            let rh = region_handle_of_context(lua, &this)?;
            lua.app_data_mut::<Model>()
                .expect("model")
                .region_data
                .entry(rh)
                .or_default()
                .alpha = Some(alpha.clamp(0.0, 1.0));
            Ok(())
        })?;
    vm.table_set_function(&methods, "SetAlpha", &set_alpha)?;

    let get_alpha = vm.create_function("Region:GetAlpha", |lua, this: LuaTable| {
        let rh = region_handle_of_context(lua, &this)?;
        let model = lua.app_data_ref::<Model>().expect("model");
        Ok(model
            .region_data
            .get(&rh)
            .and_then(|data| data.alpha)
            .unwrap_or(1.0))
    })?;
    vm.table_set_function(&methods, "GetAlpha", &get_alpha)?;

    let set_vertex_color =
        vm.create_function(
            "Region:SetVertexColor",
            |lua,
             (this, red, green, blue, alpha): (
                LuaTable,
                LuaValue,
                LuaValue,
                LuaValue,
                Option<f32>,
            )| {
                let rh = region_handle_of_context(lua, &this)?;
                let mut model = lua.app_data_mut::<Model>().expect("model");
                let data = model.region_data.entry(rh).or_default();
                data.vertex_color = Some([
                    as_f32_context(&red),
                    as_f32_context(&green),
                    as_f32_context(&blue),
                    alpha.unwrap_or(1.0),
                ]);
                // `SetTextColor` writes the same slot, so this is explicit for font inheritance.
                data.font_explicit.color = true;
                Ok(())
            },
        )?;
    vm.table_set_function(&methods, "SetVertexColor", &set_vertex_color)?;

    let get_vertex_color = vm.create_function("Region:GetVertexColor", |lua, this: LuaTable| {
        let rh = region_handle_of_context(lua, &this)?;
        let model = lua.app_data_ref::<Model>().expect("model");
        let color = model
            .region_data
            .get(&rh)
            .and_then(|data| data.vertex_color)
            .unwrap_or([1.0, 1.0, 1.0, 1.0]);
        Ok((color[0], color[1], color[2], color[3]))
    })?;
    vm.table_set_function(&methods, "GetVertexColor", &get_vertex_color)?;

    let set_blend_mode = vm.create_function(
        "Region:SetBlendMode",
        |lua, (this, mode): (LuaTable, String)| {
            let rh = region_handle_of_context(lua, &this)?;
            if let Some(blend) = BlendMode::parse(&mode) {
                lua.app_data_mut::<Model>()
                    .expect("model")
                    .region_data
                    .entry(rh)
                    .or_default()
                    .blend = blend;
            }
            Ok(())
        },
    )?;
    vm.table_set_function(&methods, "SetBlendMode", &set_blend_mode)?;

    let get_blend_mode = vm.create_function("Region:GetBlendMode", |lua, this: LuaTable| {
        let rh = region_handle_of_context(lua, &this)?;
        let model = lua.app_data_ref::<Model>().expect("model");
        Ok(model
            .region_data
            .get(&rh)
            .map_or(BlendMode::default(), |data| data.blend)
            .name())
    })?;
    vm.table_set_function(&methods, "GetBlendMode", &get_blend_mode)?;

    let set_draw_layer = vm.create_function(
        "Region:SetDrawLayer",
        |lua, (this, layer, sub_level): (LuaTable, String, Option<i64>)| {
            let rh = region_handle_of_context(lua, &this)?;
            let draw_layer = draw_layer_from_str(&layer).ok_or_else(|| {
                benilla_lua::Error::runtime(format!("unknown draw layer '{layer}'"))
            })?;
            let mut model = lua.app_data_mut::<Model>().expect("model");
            if let Some(region) = model.arena.region_mut(rh) {
                region.draw_layer = draw_layer;
                if let Some(sub_level) = sub_level {
                    region.sub_level =
                        sub_level.clamp(i64::from(i8::MIN), i64::from(i8::MAX)) as i8;
                }
            }
            Ok(())
        },
    )?;
    vm.table_set_function(&methods, "SetDrawLayer", &set_draw_layer)?;

    let get_draw_layer = vm.create_function("Region:GetDrawLayer", |lua, this: LuaTable| {
        let rh = region_handle_of_context(lua, &this)?;
        let model = lua.app_data_ref::<Model>().expect("model");
        let draw_layer = model
            .arena
            .region(rh)
            .map_or(crate::order::DrawLayer::Artwork, |region| region.draw_layer);
        Ok(draw_layer_name(draw_layer))
    })?;
    vm.table_set_function(&methods, "GetDrawLayer", &get_draw_layer)?;

    for (name, with_alpha) in [("SetGradientAlpha", true), ("SetGradient", false)] {
        let function = vm.create_function(name, move |lua, arguments: LuaMultiValue| {
            let mut arguments = arguments.into_iter();
            let this = match arguments.next() {
                Some(LuaValue::Table(table)) => table,
                _ => return Err(benilla_lua::Error::runtime("expected a region")),
            };
            let orientation = match arguments.next() {
                Some(LuaValue::String(value)) => value
                    .as_str()
                    .map(str::to_owned)
                    .map_err(|error| benilla_lua::Error::runtime(error.to_string()))?,
                _ => String::new(),
            };
            let count = if with_alpha { 8 } else { 6 };
            let mut channels = [0.0_f32; 8];
            for channel in channels.iter_mut().take(count) {
                *channel = arguments.next().as_ref().map(as_f32_context).unwrap_or(0.0);
            }
            let (start, end) = if with_alpha {
                (
                    [channels[0], channels[1], channels[2], channels[3]],
                    [channels[4], channels[5], channels[6], channels[7]],
                )
            } else {
                (
                    [channels[0], channels[1], channels[2], 1.0],
                    [channels[3], channels[4], channels[5], 1.0],
                )
            };
            let rh = region_handle_of_context(lua, &this)?;
            lua.app_data_mut::<Model>()
                .expect("model")
                .region_data
                .entry(rh)
                .or_default()
                .gradient = Some(crate::script::Gradient {
                vertical: orientation.eq_ignore_ascii_case("VERTICAL"),
                start,
                end,
            });
            Ok(())
        })?;
        vm.table_set_function(&methods, name, &function)?;
    }

    let set_desaturated =
        vm.create_function("Region:SetDesaturated", |lua, arguments: LuaMultiValue| {
            let mut arguments = arguments.into_iter();
            let this = match arguments.next() {
                Some(LuaValue::Table(table)) => table,
                _ => return Ok(LuaValue::Nil),
            };
            let enabled = match arguments.next() {
                None => true,
                Some(LuaValue::Nil) => false,
                Some(LuaValue::Boolean(value)) => value,
                Some(LuaValue::Integer(value)) => value != 0,
                Some(LuaValue::Number(value)) => value.trunc() != 0.0,
                Some(_) => true,
            };
            let rh = region_handle_of_context(lua, &this)?;
            lua.app_data_mut::<Model>()
                .expect("model")
                .region_data
                .entry(rh)
                .or_default()
                .desaturated = enabled;
            Ok(LuaValue::Number(1.0))
        })?;
    vm.table_set_function(&methods, "SetDesaturated", &set_desaturated)?;

    let get_texture = vm.create_function("Region:GetTexture", |lua, this: LuaTable| {
        let rh = region_handle_of_context(lua, &this)?;
        let model = lua.app_data_ref::<Model>().expect("model");
        let Some(data) = model.region_data.get(&rh) else {
            return Ok(None);
        };
        if data.fill.is_some() {
            return Ok(Some("Solid Texture".to_string()));
        }
        Ok(data
            .texture
            .as_ref()
            .map(|texture| match texture.rfind('.') {
                Some(index) => texture[..index].to_string(),
                None => texture.clone(),
            }))
    })?;
    vm.table_set_function(&methods, "GetTexture", &get_texture)?;

    let set_alpha_gradient = vm.create_function(
        "Region:SetAlphaGradient",
        |lua, (this, start, length): (LuaTable, f32, f32)| {
            let rh = region_handle_of_context(lua, &this)?;
            let mut model = lua.app_data_mut::<Model>().expect("model");
            let data = model.region_data.entry(rh).or_default();
            data.alpha_gradient = Some((start, length));
            let chars = data.text.as_deref().map_or(0, |text| text.chars().count());
            Ok(start < chars as f32)
        },
    )?;
    vm.table_set_function(&methods, "SetAlphaGradient", &set_alpha_gradient)?;

    let set_tex_coord_modifies_rect = vm.create_function(
        "Region:SetTexCoordModifiesRect",
        |lua, (this, argument): (LuaTable, LuaValue)| {
            let rh = region_handle_of_context(lua, &this)?;
            let enabled = bool_or_default_context(Some(&argument), false);
            lua.app_data_mut::<Model>()
                .expect("model")
                .region_data
                .entry(rh)
                .or_default()
                .tex_coord_modifies_rect = enabled;
            Ok(())
        },
    )?;
    vm.table_set_function(
        &methods,
        "SetTexCoordModifiesRect",
        &set_tex_coord_modifies_rect,
    )?;

    let get_tex_coord_modifies_rect =
        vm.create_function("Region:GetTexCoordModifiesRect", |lua, this: LuaTable| {
            let rh = region_handle_of_context(lua, &this)?;
            let model = lua.app_data_ref::<Model>().expect("model");
            Ok(region_flag(
                model
                    .region_data
                    .get(&rh)
                    .is_some_and(|data| data.tex_coord_modifies_rect),
            ))
        })?;
    vm.table_set_function(
        &methods,
        "GetTexCoordModifiesRect",
        &get_tex_coord_modifies_rect,
    )?;

    let set_tex_coord = vm.create_function(
        "Region:SetTexCoord",
        |lua, (this, arguments): (LuaTable, LuaVariadic<LuaValue>)| {
            let coordinates: Vec<f32> = arguments.iter().map(as_f32_context).collect();
            let rh = region_handle_of_context(lua, &this)?;
            let tex_coords = match coordinates.len() {
                4 => Some(TexCoords::Rect([
                    coordinates[0],
                    coordinates[1],
                    coordinates[2],
                    coordinates[3],
                ])),
                8 => Some(TexCoords::Corners([
                    [coordinates[0], coordinates[1]],
                    [coordinates[4], coordinates[5]],
                    [coordinates[6], coordinates[7]],
                    [coordinates[2], coordinates[3]],
                ])),
                0 => None,
                count => {
                    return Err(benilla_lua::Error::runtime(format!(
                        "SetTexCoord: expected 4 (edges) or 8 (corner pairs) args, got {count}"
                    )))
                }
            };
            lua.app_data_mut::<Model>()
                .expect("model")
                .region_data
                .entry(rh)
                .or_default()
                .tex_coords = tex_coords;
            Ok(())
        },
    )?;
    vm.table_set_function(&methods, "SetTexCoord", &set_tex_coord)?;

    let get_tex_coord = vm.create_function("Region:GetTexCoord", |lua, this: LuaTable| {
        let rh = region_handle_of_context(lua, &this)?;
        let model = lua.app_data_ref::<Model>().expect("model");
        let corners = model
            .region_data
            .get(&rh)
            .and_then(|data| data.tex_coords)
            .map_or(
                [[0.0, 0.0], [0.0, 1.0], [1.0, 0.0], [1.0, 1.0]],
                |tex_coords| match tex_coords {
                    TexCoords::Corners(corners) => [corners[0], corners[3], corners[1], corners[2]],
                    TexCoords::Rect(_) => {
                        let [left, right, top, bottom] = tex_coords.edges();
                        [[left, top], [left, bottom], [right, top], [right, bottom]]
                    }
                },
            );
        Ok((
            corners[0][0],
            corners[0][1],
            corners[1][0],
            corners[1][1],
            corners[2][0],
            corners[2][1],
            corners[3][0],
            corners[3][1],
        ))
    })?;
    vm.table_set_function(methods, "GetTexCoord", &get_tex_coord)
}

fn as_f32_context(value: &LuaValue) -> f32 {
    match value {
        LuaValue::Number(number) => *number as f32,
        LuaValue::Integer(integer) => *integer as f32,
        _ => 0.0,
    }
}

fn texture_channel_context(value: &LuaValue, default: f32) -> f32 {
    match value {
        LuaValue::Number(_) | LuaValue::Integer(_) => as_f32_context(value),
        LuaValue::String(value) => value
            .as_str()
            .ok()
            .and_then(|value| value.parse::<f32>().ok())
            .unwrap_or(default),
        _ => default,
    }
}

fn strict_lua_string(value: &benilla_lua::LuaString) -> benilla_lua::Result<&str> {
    value
        .as_str()
        .map_err(|error| benilla_lua::Error::runtime(error.to_string()))
}

fn bool_or_default_context(value: Option<&LuaValue>, default: bool) -> bool {
    let Some(value) = value else {
        return default;
    };
    match value {
        LuaValue::Nil => false,
        LuaValue::Boolean(value) => *value,
        LuaValue::LightUserData(_) => default,
        LuaValue::Integer(value) => (*value as i32) != 0,
        LuaValue::Number(value) => (*value as i64 as i32) != 0,
        LuaValue::String(value) => match value.as_bytes().first() {
            Some(b'0' | b'F' | b'N' | b'f' | b'n') => false,
            Some(b'1'..=b'9' | b'T' | b'Y' | b't' | b'y') => true,
            _ if value.as_bytes().eq_ignore_ascii_case(b"off")
                || value.as_bytes().eq_ignore_ascii_case(b"disabled") =>
            {
                false
            }
            _ if value.as_bytes().eq_ignore_ascii_case(b"on")
                || value.as_bytes().eq_ignore_ascii_case(b"enabled") =>
            {
                true
            }
            _ => default,
        },
        LuaValue::Table(_) | LuaValue::Function(_) => default,
    }
}

impl crate::script::UiScript {
    /// Install the host's texture-path oracle — the resolver behind the path form of
    /// `SetTexture`'s **1 | nil** return ([`Model::texture_probe`]). The host hands in existence
    /// over its real stores (patch chain + loose addon folder); a VM that never gets one keeps
    /// answering nil for every path, the engine-less truth.
    pub fn set_texture_probe(&mut self, probe: crate::script::TextureProbe) {
        self.model_mut().texture_probe = Some(probe);
    }

    /// Install the host's font-path oracle — the resolver behind `SetFont`'s **1 | nil** return
    /// ([`Model::font_probe`], decision 2103). The host hands in load-ability over its real stores
    /// (patch chain + the one AddOns folder); a VM that never gets one keeps answering 1 for every
    /// non-empty path, because it has no font store a load could fail against.
    pub fn set_font_probe(&mut self, probe: crate::script::FontProbe) {
        self.model_mut().font_probe = Some(probe);
    }

    /// Install the host's texture **texel-size** oracle — what lets a region with an authored size
    /// of `0` on an axis take that span from its art, as the client's virtual size getters do
    /// ([`Model::texture_size_probe`], decision 1349 / wow-re `region-size-fallback.md` §2). A VM
    /// that never gets one leaves such a region exactly where it was.
    pub fn set_texture_size_probe(&mut self, probe: crate::script::TextureSizeProbe) {
        self.model_mut().texture_size_probe = Some(probe);
    }
}
