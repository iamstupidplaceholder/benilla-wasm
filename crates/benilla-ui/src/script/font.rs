//! **Font objects as first-class Lua objects** — a `<Font name="GameFontNormal">` is not merely a
//! style record the loader flattens, it is an *object* the client publishes as the global
//! `GameFontNormal`, with its own method table.
//!
//! ## Why this exists
//!
//! 1.12's own FrameXML never touches the API — `grep -c FontObject` over
//! `wow-5875-re/WoW/_extracted_framexml/*.lua` is **0** — so the whole surface is addon-facing, and
//! the corpus (`/Users/sam/dev/wow-addons-vanilla`, 218 addons) is where the demand is:
//!
//! | call shape | sites | note |
//! |---|---|---|
//! | `x:SetFontObject(GameFontNormal)` (bare global) | 3,180 | the form we did **not** accept |
//! | `x:SetFontObject("GameFontNormal")` (string) | 6 | the only form we did accept |
//! | `x:GetFontObject():GetTextColor()` | 65 | needs an OBJECT back, not a name |
//! | `GameTooltipHeaderText:GetFont()` and kin | 268 | `Tablet-2.0`'s header-size probe |
//! | `GameFont*:GetTextColor/GetShadowOffset/GetShadowColor` | 13 | |
//! | `CreateFont(name)` | 3 | `_Nameplates`, `!OmniCC`, `FonzAppraiser` |
//! | `f:CopyFontObject(GameFontHighlightSmall)` | 1 | Font-on-Font |
//!
//! ## The method surface, and where it came from
//!
//! §5-verified in wow-re, `system/ui/scratch/font-object-lua-surface.md` (landed `0193861a`). The
//! Font method table is `.data 0x87c7c8` with **22 entries** — the count read from `mov edx,0x16`
//! at `0x7a10d5`, not from a run-length scan, which merges neighbouring tables and reports a bogus
//! 54. Its lookup `0x7a1100` has **no base-class fallback**, so 22 is the entire surface:
//! `GetObjectType · IsObjectType · GetName · SetFontObject · GetFontObject · CopyFontObject ·
//! SetFont · GetFont · SetAlpha · GetAlpha · SetTextColor · GetTextColor · SetShadowColor ·
//! GetShadowColor · SetShadowOffset · GetShadowOffset · SetSpacing · GetSpacing · SetJustifyH ·
//! GetJustifyH · SetJustifyV · GetJustifyV`. All implemented here **except the `Spacing` pair** —
//! see [`install`]. `CopyFontObject` is the only one of the 22 a FontString does not also have.
//!
//! The class is **`CSimpleFont`** (`__FILE__` `0x87a454`), and a named font is published by
//! `0x783870` → `SetName 0x784150` → `CreateLuaHandle 0x701bd0` → `_G[name] = handle` at
//! `0x701cb8`, **skipped when the global is already non-nil** — the same non-overwriting rule
//! [`publish_global`] applies on the frame side.
//!
//! ## Mutability — the deliberate part
//!
//! `GameFontNormal:SetFont(…)` on the real client repaints every FontString that inherits it, and
//! the mechanism is not a copy: `SetFontObject` stores a parent pointer (`+0x28`, FontString
//! `+0xd0`) **and** links the instance into the parent's intrusive dependents list (`+0x74`/`+0x78`,
//! link offset `+0x70`). Every setter ends in `vtable[+0x14]` = `NotifyDependents 0x784180`, which
//! walks that list and calls each dependent's `OnFontChanged` — `0x77e4b0` for a font (which
//! cascades) and `0x773530`→`0x770800` for a FontString (which re-drives its real setters).
//!
//! We model that with an eager push instead of an intrusive list, so no reader changes: every
//! setter here writes the [`FontObject`] record and then calls [`propagate`], which re-paints every
//! region whose [`RegionData::font_object`] names it.
//!
//! **The severance rule, corrected by the bytes.** Our first cut read `FONTINSTANCE+0x038` as an
//! "explicitly set" mask and reset it on every `SetFontObject`. Both halves were wrong. `+0x38`'s
//! low bits are a transient broadcast flag and its high bits mean "has a resolved value" — set by
//! the *inheriting* merge too (`0x7709d3`). The real "stop inheriting this property" signal is a
//! **cleared** bit in the previously-unrecorded **inheritMask at `+0x2c`** (FontString `+0xd4`,
//! per-axis justify at `+0x124`), cleared by each local setter and **never restored** — so a
//! FontString that set its own colour stays severed *across a later `SetFontObject`*.
//! [`RegionData::font_explicit`] is that mask, and nothing resets it.
//!
//! The source side gates too: a merge only copies properties the source actually holds, so a font
//! freshly minted by [`create_font`] (`mask == 0`) copies **nothing** — see [`repaint`].
//!
//! **What we deliberately do not model, now a verified divergence rather than an unknown:** a
//! `<Font inherits="OtherFont">` chain stays flattened at load (`Loader::do_font`), so mutating
//! `MasterFont` does not walk down to `GameFontNormal`. The reference links these live as well —
//! `LoadXML 0x783c30` calls the *same* `0x770c60` link function `SetFontObject` calls
//! (`0x783ce6`), and `font=` likewise (`0x783d22`). Flattening is what this arc was scoped to keep;
//! the cost is bounded and measured: **0 corpus addons mutate a declared font object at all** —
//! every mutation in 218 addons is of the addon's own [`create_font`] object — so the Font→Font
//! half has no demand behind it and the Font→FontString half has all of it.

use benilla_lua::{LuaContext, Table as LuaTable, Value as LuaValue, Vm};

use crate::justify;

use super::{FontObject, FontShadow, Model, Outline, RegionData};

/// The shared metatable of every font-object handle.
const REG_FONT_META: &str = "__benilla_font_meta";
/// The font-object method table the metatable's `__index` dispatches through.
const REG_FONT_METHODS: &str = "__benilla_font_methods";
/// name → handle, so `GameFontNormal == GameFontNormal` and a re-declared `<Font>` keeps identity.
const REG_FONT_WRAPPERS: &str = "__benilla_font_wrappers";

// ─────────────────────────────────────────────────────────────────────────────────────────────
// The handle
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// Get-or-create the Lua handle for a named font object.
///
/// Identity is the **name**, not a minted id: every font object in 1.12 has one (an unnamed
/// `<Font>` is warned and dropped at parse time, and [`create_font`] requires one), and keying by
/// name is what makes a re-declared `<Font name="GameFontNormal">` update the record *in place*
/// while every `SetFontObject(GameFontNormal)` already taken keeps pointing at the same object.
///
/// `T[0]` is the name string rather than the frame/region wrapper's `LightUserData` id, so
/// [`super::object::decode_id`] rejects a font handle passed where a frame is wanted, with its own
/// message, instead of decoding a garbage id.
pub(crate) fn wrapper_context(
    lua: &mut impl LuaContext,
    name: &str,
) -> benilla_lua::Result<LuaTable> {
    let wrappers: LuaTable = lua.get_registry(REG_FONT_WRAPPERS)?;
    if let LuaValue::Table(wrapper) = lua.table_get(&wrappers, name)? {
        return Ok(wrapper);
    }
    let wrapper = lua.create_table()?;
    lua.table_set_raw(&wrapper, 0_i64, name)?;
    let meta: LuaTable = lua.get_registry(REG_FONT_META)?;
    lua.table_set_metatable(&wrapper, Some(&meta))?;
    lua.table_set(&wrappers, name, wrapper.clone())?;
    Ok(wrapper)
}

/// Publish `_G[name]` as this font object's handle — what makes `fs:SetFontObject(GameFontNormal)`
/// resolve to anything at all. Non-overwriting, exactly like the frame side's [`publish_global`].
///
/// Called by `Loader::do_font` for every top-level `<Font name=…>`, and by [`create_font`].
pub(crate) fn publish_context(lua: &mut impl LuaContext, name: &str) -> benilla_lua::Result<()> {
    let wrapper = wrapper_context(lua, name)?;
    super::object::publish_global_context(lua, name, &wrapper)
}

/// Accept any of the **three** forms the real binding takes and return the registry name, with
/// `None` for the nil form (clear the link).
///
/// The reference's own usage string settles the shape — `.rdata 0x87c5cc` reads verbatim
/// `Usage: %s:SetFontObject(font or "font" or nil)` (wow-re
/// `system/ui/scratch/font-object-lua-surface.md`, §5-verified). So all three are real: the object
/// (3,180 of the corpus's 3,186 sites), the name string (our own shipped `assets/ui` plus 6 corpus
/// sites), and nil.
///
/// Anything else — a frame, a number, a missing argument — is an **error**, and in the reference
/// too: every rejection there is `luaL_error 0x6f4940`, which longjmps and aborts the call rather
/// than no-opping. A font that quietly fails to apply is the silent-drop class of 1203/1205/1211.
pub(super) fn resolve_context(
    lua: &mut impl LuaContext,
    verb: &str,
    value: &LuaValue,
) -> benilla_lua::Result<Option<String>> {
    match value {
        LuaValue::Nil => Ok(None),
        LuaValue::Table(table) => match lua.table_get_raw(table, 0_i64)? {
            LuaValue::String(name) => name
                .as_str()
                .map(|name| Some(name.to_string()))
                .map_err(|error| benilla_lua::Error::runtime(error.to_string())),
            _ => Err(benilla_lua::Error::runtime(format!(
                "{verb}: the argument is a table but not a font object \
                 (a frame or region cannot be a font)"
            ))),
        },
        LuaValue::String(name) => name
            .as_str()
            .map(|name| Some(name.to_string()))
            .map_err(|error| benilla_lua::Error::runtime(error.to_string())),
        other => Err(benilla_lua::Error::runtime(format!(
            "{verb}: expected a font object, a font name, or nil, got {}",
            other.type_name()
        ))),
    }
}

fn resolve_required_context(
    lua: &mut impl LuaContext,
    verb: &str,
    value: &LuaValue,
) -> benilla_lua::Result<String> {
    resolve_context(lua, verb, value)?.ok_or_else(|| {
        benilla_lua::Error::runtime(format!(
            "{verb}: expected a font object or a font name, got nil"
        ))
    })
}

fn name_of_context(lua: &mut impl LuaContext, this: &LuaTable) -> benilla_lua::Result<String> {
    match lua.table_get_raw(this, 0_i64)? {
        LuaValue::String(name) => name
            .as_str()
            .map(str::to_owned)
            .map_err(|error| benilla_lua::Error::runtime(error.to_string())),
        _ => Err(benilla_lua::Error::runtime(
            "not a font object (missing T[0] name identity)",
        )),
    }
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// The live link: font object → the regions that inherit it
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// Copy a font object's paint onto one region, **skipping every property that region severed by
/// setting for itself** ([`RegionData::font_explicit`]).
///
/// **A property the source does not hold copies nothing.** The reference gates every merge on the
/// source's own has-a-value mask (`+0x38`'s high bits, set by the merge `0x770910`): a font freshly
/// minted by `CreateFont` has `mask == 0`, so `SetFontObject`-ing it onto a FontString leaves that
/// string exactly as it was — no blanking, no fallback to a default. `_Nameplates` relies on
/// nothing here, but `!OmniCC`'s probe font would otherwise wipe a label it was pointed at.
///
/// Residue: [`Outline`] has no "unset" state distinct from `NONE`, so an outline is still written
/// unconditionally. Every shipped `<Font>` flattens from `MasterFont`, which declares no outline,
/// so the two readings coincide for all of them; a `CreateFont` object pointed at an outlined
/// FontString is the one case that differs, and it has no caller.
pub(crate) fn repaint(d: &mut RegionData, fo: &FontObject) {
    let ex = d.font_explicit;
    if !ex.face {
        if let Some(f) = &fo.font {
            d.font_path = Some(f.clone());
        }
    }
    if !ex.height {
        if let Some(h) = fo.height {
            d.font_height = Some(h);
        }
    }
    if !ex.outline {
        d.outline = fo.outline;
    }
    if !ex.shadow {
        if let Some(s) = fo.shadow {
            d.font_shadow = Some(s);
        }
    }
    if !ex.color {
        if let Some(c) = fo.color {
            d.vertex_color = Some(c);
        }
    }
    if !ex.justify_h {
        if let Some(j) = fo.justify_h {
            d.justify.set_h(j);
        }
    }
    if !ex.justify_v {
        if let Some(j) = fo.justify_v {
            d.justify.set_v(j);
        }
    }
}

/// Re-paint every region that inherits `name` — the `SetFont`/`SetTextColor`/… propagation the real
/// client gets for free from its live `parentFontObject`. Linear in regions, on a human-rate path
/// (a font-object setter is an addon config action, never a frame path).
///
/// Per-state **button** label fonts need no equivalent: they are stored as names and re-resolved at
/// every `extract`, so a mutated font object reaches them on the next frame by construction.
pub(crate) fn propagate(model: &mut Model, name: &str) {
    let Some(fo) = model.font_object(name).cloned() else {
        return;
    };
    for d in model.region_data.values_mut() {
        if d.font_object.as_deref() == Some(name) {
            repaint(d, &fo);
        }
    }
    // A fan-out cannot name its regions one by one without a second pass; a font-object edit is
    // a human-rate action, so the measure ledger goes conservative and the next sweep walks the
    // roster once, exactly as every sweep did before the ledger existed.
    model.touch_measure_all();
}

/// Run `f` over the named font object's record, then propagate the change to its dependants.
/// The one write path every setter below goes through.
fn edit_context<R>(
    lua: &mut impl LuaContext,
    this: &LuaTable,
    f: impl FnOnce(&mut FontObject) -> R,
) -> benilla_lua::Result<R> {
    let name = name_of_context(lua, this)?;
    let mut model = lua.app_data_mut::<Model>().expect("model");
    let out = f(model
        .font_objects_by_lower
        .entry(name.to_ascii_lowercase())
        .or_default());
    propagate(&mut model, &name);
    Ok(out)
}

/// Read the named font object's record (a clone — the records are small and this is human-rate).
fn read_context(lua: &mut impl LuaContext, this: &LuaTable) -> benilla_lua::Result<FontObject> {
    let name = name_of_context(lua, this)?;
    let model = lua.app_data_ref::<Model>().expect("model");
    Ok(model.font_object(&name).cloned().unwrap_or_default())
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// install — the method table, the metatable, and CreateFont
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// Build the font-object method table and metatable, and register the `CreateFont` global.
///
/// **Implemented** (20 of the reference table's 22): `GetObjectType`, `IsObjectType`, `GetName`,
/// `SetFontObject`, `GetFontObject`, `CopyFontObject`, `SetFont`, `GetFont`, `SetAlpha`,
/// `GetAlpha`, `SetTextColor`, `GetTextColor`, `SetShadowColor`, `GetShadowColor`,
/// `SetShadowOffset`, `GetShadowOffset`, `SetJustifyH`, `GetJustifyH`, `SetJustifyV`,
/// `GetJustifyV`.
///
/// **Deliberately absent: `SetSpacing`/`GetSpacing`.** We model no line spacing anywhere — neither
/// [`RegionData`] nor the text layout has the field, and the line pitch is the font height — so a
/// `SetSpacing` here could only store a number nobody draws. That is precisely the silently-ignored
/// setter this codebase keeps being bitten by (1203, 1205, 1211), and the alternative failure is
/// the good one: a missing method raises `attempt to call method 'SetSpacing' (a nil value)`, which
/// names itself. Demand behind it is **zero** — no call on any font-object global anywhere in the
/// 218-addon corpus, and 1.12's own FrameXML never calls the font API at all. When spacing becomes
/// a thing the renderer honours, the pair lands with it.
pub(super) fn install(lua: &mut Vm) -> benilla_lua::Result<()> {
    let wrappers = lua.create_table()?;
    lua.put_registry(REG_FONT_WRAPPERS, wrappers)?;
    let methods = lua.create_table()?;

    let get_object_type =
        lua.create_function("Font:GetObjectType", |_, _this: LuaTable| Ok("Font"))?;
    lua.table_set_function(&methods, "GetObjectType", &get_object_type)?;
    let is_object_type =
        lua.create_function("Font:IsObjectType", |_, (_this, ty): (LuaTable, String)| {
            Ok(if ty.eq_ignore_ascii_case("font") {
                LuaValue::Integer(1)
            } else {
                LuaValue::Nil
            })
        })?;
    lua.table_set_function(&methods, "IsObjectType", &is_object_type)?;
    let get_name = lua.create_function("Font:GetName", |lua, this: LuaTable| {
        name_of_context(lua, &this)
    })?;
    lua.table_set_function(&methods, "GetName", &get_name)?;

    let set_font_object = lua.create_function(
        "Font:SetFontObject",
        |lua, (this, other): (LuaTable, LuaValue)| {
            if resolve_context(lua, "SetFontObject", &other)?.is_some() {
                copy_from_context(lua, &this, &other, "SetFontObject")?;
            }
            Ok(())
        },
    )?;
    lua.table_set_function(&methods, "SetFontObject", &set_font_object)?;
    let copy_font_object = lua.create_function(
        "Font:CopyFontObject",
        |lua, (this, other): (LuaTable, LuaValue)| {
            resolve_required_context(lua, "CopyFontObject", &other)?;
            copy_from_context(lua, &this, &other, "CopyFontObject")
        },
    )?;
    lua.table_set_function(&methods, "CopyFontObject", &copy_font_object)?;
    let get_font_object =
        lua.create_function("Font:GetFontObject", |_, _this: LuaTable| Ok(LuaValue::Nil))?;
    lua.table_set_function(&methods, "GetFontObject", &get_font_object)?;

    let set_font = lua.create_function(
        "Font:SetFont",
        |lua, (this, file, height, flags): (LuaTable, LuaValue, LuaValue, Option<String>)| {
            let (path, height) = super::font_block::set_font_args_context(&file, &height, "Font")?;
            let ok = !path.is_empty();
            edit_context(lua, &this, |font| {
                if ok {
                    font.font = Some(path);
                }
                font.height = Some(height);
                if let Some(flags) = flags {
                    font.outline = Outline::flags(&flags);
                }
            })?;
            Ok(if ok {
                LuaValue::Number(1.0)
            } else {
                LuaValue::Nil
            })
        },
    )?;
    lua.table_set_function(&methods, "SetFont", &set_font)?;
    let get_font = lua.create_function("Font:GetFont", |lua, this: LuaTable| {
        let font = read_context(lua, &this)?;
        Ok((
            font.font.map_or(LuaValue::Nil, LuaValue::from),
            font.height.unwrap_or(0.0),
            font.outline.as_str(),
        ))
    })?;
    lua.table_set_function(&methods, "GetFont", &get_font)?;

    let set_text_color =
        lua.create_function(
            "Font:SetTextColor",
            |lua,
             (this, red, green, blue, alpha): (
                LuaTable,
                LuaValue,
                LuaValue,
                LuaValue,
                Option<f32>,
            )| {
                let (red, green, blue) = (
                    as_f32_context(&red),
                    as_f32_context(&green),
                    as_f32_context(&blue),
                );
                edit_context(lua, &this, |font| {
                    let old_alpha = font.color.map_or(1.0, |color| color[3]);
                    font.color = Some([red, green, blue, alpha.unwrap_or(old_alpha)]);
                })
            },
        )?;
    lua.table_set_function(&methods, "SetTextColor", &set_text_color)?;
    let get_text_color = lua.create_function("Font:GetTextColor", |lua, this: LuaTable| {
        let color = read_context(lua, &this)?
            .color
            .unwrap_or([1.0, 1.0, 1.0, 1.0]);
        Ok((color[0], color[1], color[2], color[3]))
    })?;
    lua.table_set_function(&methods, "GetTextColor", &get_text_color)?;
    let set_alpha =
        lua.create_function("Font:SetAlpha", |lua, (this, alpha): (LuaTable, f32)| {
            edit_context(lua, &this, |font| {
                let color = font.color.unwrap_or([1.0, 1.0, 1.0, 1.0]);
                font.color = Some([color[0], color[1], color[2], alpha]);
            })
        })?;
    lua.table_set_function(&methods, "SetAlpha", &set_alpha)?;
    let get_alpha = lua.create_function("Font:GetAlpha", |lua, this: LuaTable| {
        Ok(read_context(lua, &this)?
            .color
            .map_or(1.0, |color| color[3]))
    })?;
    lua.table_set_function(&methods, "GetAlpha", &get_alpha)?;

    let set_shadow_color =
        lua.create_function(
            "Font:SetShadowColor",
            |lua,
             (this, red, green, blue, alpha): (
                LuaTable,
                LuaValue,
                LuaValue,
                LuaValue,
                Option<f32>,
            )| {
                let (red, green, blue) = (
                    as_f32_context(&red),
                    as_f32_context(&green),
                    as_f32_context(&blue),
                );
                edit_context(lua, &this, |font| {
                    let offset = font.shadow.map_or([0.0, 0.0], |shadow| shadow.offset);
                    font.shadow = Some(FontShadow {
                        offset,
                        color: [red, green, blue, alpha.unwrap_or(1.0)],
                    });
                })
            },
        )?;
    lua.table_set_function(&methods, "SetShadowColor", &set_shadow_color)?;
    let get_shadow_color = lua.create_function("Font:GetShadowColor", |lua, this: LuaTable| {
        let color = read_context(lua, &this)?
            .shadow
            .map_or([0.0, 0.0, 0.0, 1.0], |shadow| shadow.color);
        Ok((color[0], color[1], color[2], color[3]))
    })?;
    lua.table_set_function(&methods, "GetShadowColor", &get_shadow_color)?;
    let set_shadow_offset = lua.create_function(
        "Font:SetShadowOffset",
        |lua, (this, x, y): (LuaTable, f32, f32)| {
            edit_context(lua, &this, |font| {
                let color = font
                    .shadow
                    .map_or([0.0, 0.0, 0.0, 1.0], |shadow| shadow.color);
                font.shadow = Some(FontShadow {
                    offset: [x, y],
                    color,
                });
            })
        },
    )?;
    lua.table_set_function(&methods, "SetShadowOffset", &set_shadow_offset)?;
    let get_shadow_offset =
        lua.create_function("Font:GetShadowOffset", |lua, this: LuaTable| {
            let offset = read_context(lua, &this)?
                .shadow
                .map_or([0.0, 0.0], |shadow| shadow.offset);
            Ok((offset[0], offset[1]))
        })?;
    lua.table_set_function(&methods, "GetShadowOffset", &get_shadow_offset)?;

    let set_justify_h = lua.create_function(
        "Font:SetJustifyH",
        |lua, (this, value): (LuaTable, String)| {
            let value = match justify::parse_h(&value) {
                justify::Set::To(value) => value,
                justify::Set::Clears => return Ok(()),
                justify::Set::NoMatch => {
                    return Err(benilla_lua::Error::runtime(justify::usage_h_message(
                        "Font",
                    )))
                }
            };
            edit_context(lua, &this, |font| font.justify_h = Some(value))
        },
    )?;
    lua.table_set_function(&methods, "SetJustifyH", &set_justify_h)?;
    let get_justify_h = lua.create_function("Font:GetJustifyH", |lua, this: LuaTable| {
        Ok(justify::name_h(
            read_context(lua, &this)?.justify_h.unwrap_or_default(),
        ))
    })?;
    lua.table_set_function(&methods, "GetJustifyH", &get_justify_h)?;
    let set_justify_v = lua.create_function(
        "Font:SetJustifyV",
        |lua, (this, value): (LuaTable, String)| {
            let value = match justify::parse_v(&value) {
                justify::Set::To(value) => value,
                justify::Set::Clears => return Ok(()),
                justify::Set::NoMatch => {
                    return Err(benilla_lua::Error::runtime(justify::usage_v_message(
                        "Font",
                    )))
                }
            };
            edit_context(lua, &this, |font| font.justify_v = Some(value))
        },
    )?;
    lua.table_set_function(&methods, "SetJustifyV", &set_justify_v)?;
    let get_justify_v = lua.create_function("Font:GetJustifyV", |lua, this: LuaTable| {
        Ok(justify::name_v(
            read_context(lua, &this)?.justify_v.unwrap_or_default(),
        ))
    })?;
    lua.table_set_function(&methods, "GetJustifyV", &get_justify_v)?;

    let meta = lua.create_table()?;
    lua.table_set(&meta, "__index", methods.clone())?;
    lua.put_registry(REG_FONT_METHODS, methods)?;
    lua.put_registry(REG_FONT_META, meta)?;
    let create_font = lua.create_function("CreateFont", create_font_context)?;
    lua.set_global_function("CreateFont", &create_font)
}

/// The body behind `SetFontObject`/`CopyFontObject` on a Font: take the *other* object's resolved
/// paint wholesale.
fn copy_from_context(
    lua: &mut impl LuaContext,
    this: &LuaTable,
    other: &LuaValue,
    verb: &str,
) -> benilla_lua::Result<()> {
    let src = resolve_required_context(lua, verb, other)?;
    let paint = {
        let model = lua.app_data_ref::<Model>().expect("model");
        model.font_object(&src).cloned().ok_or_else(|| {
            benilla_lua::Error::runtime(format!(
                "{verb}: no font object named '{src}' is registered"
            ))
        })?
    };
    edit_context(lua, this, |font| *font = paint)
}

/// `CreateFont(name)` — mint a font object at runtime and publish it as the global `name`, the
/// twin of a `<Font name=…>` declaration. Returns the handle.
///
/// Both halves are load-bearing in the corpus and neither can be dropped: `_Nameplates.lua:149` and
/// `FonzAppraiser/mods/gui/gui.lua:27` keep the **return**, while `!OmniCC/main.lua:40-41` throws it
/// away and reads the **global** on the very next line.
///
/// A name that already names a font object returns the existing one **unchanged, silently** — no
/// error, no re-publication (`0x7839ab`). We had chosen that as the non-destructive reading before
/// the bytes were asked; they agree.
///
/// The empty string is accepted (the reference's `lua_isstring` gate takes strings *and* numbers,
/// and unlike the XML path does not require a non-empty name); only a missing/nil argument is an
/// error. A fresh object holds nothing, so `SetFontObject`-ing it copies nothing — see [`repaint`].
fn create_font_context(
    lua: &mut benilla_lua::CallbackContext<'_>,
    name: Option<String>,
) -> benilla_lua::Result<LuaTable> {
    let name = name.ok_or_else(|| {
        benilla_lua::Error::runtime("CreateFont: a font name is required (it becomes a global)")
    })?;
    {
        let mut model = lua.app_data_mut::<Model>().expect("model");
        model
            .font_objects_by_lower
            .entry(name.to_ascii_lowercase())
            .or_default();
    }
    let wrapper = wrapper_context(lua, &name)?;
    super::object::publish_global_context(lua, &name, &wrapper)?;
    Ok(wrapper)
}

fn as_f32_context(value: &LuaValue) -> f32 {
    match value {
        LuaValue::Number(value) => *value as f32,
        LuaValue::Integer(value) => *value as f32,
        _ => 0.0,
    }
}
