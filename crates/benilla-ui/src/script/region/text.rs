//! Region method-table cluster: **text** — what a `FontString` carries that other text-bearing
//! widgets do not. Split out of `region.rs` at the 0716 file-size budget.
//!
//! **This file is now the FontString-only half.** The ten names it used to hand-write a second copy
//! of — `Set/GetFontObject`, `Set/GetFont`, `Set/GetTextColor`, `Set/GetShadowColor`,
//! `Set/GetShadowOffset` — are not FontString-specific at all: in the binary each is a thin
//! type-guard shim tail-calling one shared implementation, so they come from
//! [`crate::script::font_block`] via one `install` at the bottom of this file, exactly as the
//! EditBox's table gets them. The justify law likewise lives once in [`crate::justify`].
//!
//! What legitimately stays here is the surface a FontString alone has: the string itself
//! (`SetText`/`GetText`), the measured extents, `Set/GetJustifyH`/`V`, `SetNonSpaceWrap`/
//! `CanNonSpaceWrap`, and `SetTextHeight`.
//!
//! The per-property override a region has over its inherited font object is unaffected — that is
//! the severance mask (`FontExplicit`), which the shared block writes the same way this file did.

use benilla_lua::{FromLua, LuaContext, Table as LuaTable, Value as LuaValue, Vm};

use crate::justify;
use crate::script::Model;

/// Resolve `self` (a region wrapper) to its live [`RegionHandle`].
use super::{region_handle_of_callback, region_handle_of_context};

// **No `SetFormattedText`.** A later-expansion name: absent from the client's 32-entry
// FontString map, from the stock 1.12 chain, and from both addon corpora (2142's census). The
// era spelling is `SetText(format(fmt, ...))`, and `format` here is already the
// positional-aware one ([`crate::strings`]), so `%N$s` behaves at the call site the same way
// it behaved inside this shim.

// GetText — **an EMPTY string comes back as `nil`, and that substitution is the getter's own**
// (`FontString:GetText 0x79d690`, wow-re
// `system/ui/scratch/fontstring-text-cell-and-gettext-contract.md`, §5 five-worker round):
//
// ```text
// 79d72f  mov  eax,[eax+0xf0]        ; the text cell
// 79d735  test eax,eax
// 79d739  je   0x79d740              ; NULL      -> substitute
// 79d73b  cmp  byte ptr [eax],0x0    ; the FIRST-BYTE test
// 79d73e  jne  0x79d742              ; non-empty -> keep
// 79d740  xor  eax,eax               ; EMPTY     -> NULL
// 79d746  call 0x6f3890              ; pushstring; NULL -> pushnil
// ```
//
// So `FontString:GetText` **cannot return `""`**. Not the setter's doing: `SetText 0x771d80`
// never writes NULL to `+0xf0` on any leg — NULL and `""` share one leg that truncates the
// buffer in place (`0x771e7e mov byte ptr [eax],0x0`, the pointer surviving) — so the cell
// really does hold a non-NULL empty string and this getter is what collapses it.
//
// The law is **per binding, not per family**, and the two neighbours differ:
// `Button:GetText 0x780e10` carries the same substitution (`0x780ec5`, three nil conditions —
// see `button.rs`), while `EditBox:GetText 0x7985c0` carries **none** (`0x79867b` reads
// `[edit+0x32c]` straight through), which is what stock `MailFrame.lua`'s
// `GetText() == ""`/`strlen(GetText())` is written against. Do not hoist this.
//
// What it costs to get wrong (decision 2110): the stock world map blanks
// `WorldMapFrameAreaDescription` with `SetText("")` on every POI hover, and Cartographer 2.02
// reads `if WorldMapFrameAreaDescription:GetText() then` as "this POI has a status line" —
// answering `""` there put the zone's level range on the description's own line under a
// whitened label and left it on screen forever.

// GetStringWidth (FontString): the host-measured text extent from the measure
//
// **There is no `GetStringHeight` beside it, and that asymmetry is the client's.** 1.12 has no
// such method on any table — byte-verified absent in every encoding, with `GetStringWidth`
// itself as the positive control — and Blizzard's own FrameXML calls it zero times. Ours was a
// byte-identical duplicate of `GetHeight`, which is what the reference uses for this
// (`0x7a2030` falls through to the same cached measurement), so it was pruned rather than
// reimplemented (1251).
// round-trip ([`super::UiScript::set_measured_text`]) — the client asks its font engine for the
// laid-out string's metrics exactly here (`fontstring.md`), and the tooltip's auto-size sums
// these to fit its lines. `0` until the string has been measured (a frame's latency; converges).
// The stored measure only counts while its key matches the CURRENT text/font/wrap
// ([`RegionData::measure_key`]): after a SetText the old string's width is not this string's
// metric — serving it is how the whisper header's `GetWidth()` latched the edit-box insets on
// the previous header's width. A poll-until-nonzero caller (the chat header machine) now
// converges on the RIGHT measure instead of settling on a stale one.
// `GetWidth`/`GetHeight` are the neighbouring law and NOT this one ([`super::virtual_span`]):
// they take the authored size first and the measure only on an axis authored `0` — and on that
// axis the width they return is *this* number, because the reference reads both getters out of
// the one cell `[fs+0xfc]` (`0x772890` is `0x79e510`'s callee). So the two agree exactly where
// the reference makes them agree, and differ exactly where a width was declared.
// GetStringWidth is the **natural, unwrapped** extent — never the declared box, and never the
// wrapped one (wow-re `fontstring-overflow.md`, "The measurement echo": the reference's getter
// re-measures the raw text with NO wrap constraint). Unlike `GetWidth` below it deliberately
// does NOT fall back to a declared `SetWidth`: that width is the very thing a caller
// asks this to be independent of. A kit that sizes a box from this number and then sets a width
// on the string — which is what the reference's own `PanelTemplates_TabResize` does — would
// otherwise read its own output back as its next input and never settle (decision 0997, the
// macro window's character tab changing width every frame). `0` until measured, as ever.

// SetJustifyH("LEFT"|"CENTER"|"RIGHT") — a FontString's horizontal justification (XML
// `justifyH`). The token table, the whole-string match, the raise on a miss and the cross-axis
// clear are all [`crate::justify`]'s, shared with the `<Font>` object's identical pair
// so the two cannot drift apart again.

// SetJustifyV("TOP"|"MIDDLE"|"BOTTOM") — a FontString's vertical justification (XML `justifyV`).

// GetJustifyH()/GetJustifyV() → **1 string**. Real entries on the reference's FontString table
// (`0x79e5f0` / `0x79e7f0`, the FontString column of wow-re's two-column accessor table) that
// this side had simply never grown: a FontString could set its justification and not read it
// back, while the `<Font>` object — the same law, transcribed separately — could do both.
//
// An untouched FontString answers CENTER/MIDDLE, which is `RegionData`'s default *and* the
// client's ctor default `0x212` (`CENTER | MIDDLE | 0x200`) read through each axis mask.

// SetNonSpaceWrap(enable) / CanNonSpaceWrap() — FontString only (`0x79e9f0` / `0x79ead0`).
//
// Two contract details from wow-re's batch, both easy to get wrong:
//  · the getter is **`CanNonSpaceWrap`**, not `GetNonSpaceWrap`, and it answers **`1` or nil**,
//    not a boolean — 1.12 predates that convention and an addon may compare against 1.
//  · a **no-argument call ENABLES** it (the default is on), rather than being a query.
//
// `oRA2/Leader/Item.lua:561` is `f.textname:SetNonSpaceWrap(false)`, reached by two addons.

// SetTextHeight(height) — switch the FontString to the scaled-string regime (§5-verified,
// wow-re `fontstring-overflow.md`: `0x771600` is the ONLY clearer of the one-to-one bit
// `0x200`; the literal size then flows through UNCAPPED, magnified from the raster). Stored
// as the distinct [`RegionData::text_height`] — the font object is untouched, so GetFont
// keeps reporting the face's own height like the real API.

// ── the shared font block ───────────────────────────────────────────────────────────────
//
// `SetFontObject · GetFontObject · SetFont · GetFont · Set/GetTextColor · Set/GetShadowColor ·
// Set/GetShadowOffset` are **not** FontString-specific. In the binary each is a thin type-guard
// shim that tail-calls one shared implementation (`0x79f210` SetFont, `0x79f3b0` GetFont, …),
// which is what [`super::super::font_block`] models — so a FontString and an EditBox are two
// entry points to one routine, not two routines that happen to agree.
//
// They lived here as a hand-written second copy because `region.rs` was over the file-size
// budget when the block was written; that split has since landed, and `font_block`'s own doc
// named this collapse as the follow-on. 226 lines of duplicate go with it.
/// Install the FontString-only method surface through the backend-neutral VM boundary.
pub(super) fn install_context(vm: &mut Vm, neutral_methods: &LuaTable) -> benilla_lua::Result<()> {
    let set_text = vm.create_function(
        "FontString:SetText",
        |lua, (this, text): (LuaTable, Option<LuaValue>)| {
            let text = text_arg_context(text)?;
            let rh = region_handle_of_context(lua, &this)?;
            let mut model = lua.app_data_mut::<Model>().expect("model");
            let data = model.region_data.entry(rh).or_default();
            let extent_changed = data.text.as_deref().is_some_and(|t| !t.is_empty())
                != text.as_deref().is_some_and(|t| !t.is_empty());
            data.text = text;
            data.alpha_gradient = None;
            if extent_changed {
                model.touch_layout_region(rh);
            }
            model.touch_measure(rh);
            Ok(())
        },
    )?;
    vm.table_set_function(neutral_methods, "SetText", &set_text)?;

    let get_text = vm.create_function("FontString:GetText", |lua, this: LuaTable| {
        let rh = region_handle_of_context(lua, &this)?;
        let model = lua.app_data_ref::<Model>().expect("model");
        Ok(model
            .region_data
            .get(&rh)
            .and_then(|data| data.text.clone())
            .filter(|text| !text.is_empty()))
    })?;
    vm.table_set_function(&neutral_methods, "GetText", &get_text)?;

    let get_string_width = vm
        .create_function("FontString:GetStringWidth", |lua, this: LuaTable| {
            natural_width_context(lua, &this)
        })?;
    vm.table_set_function(&neutral_methods, "GetStringWidth", &get_string_width)?;

    let set_justify_h = vm.create_function(
        "FontString:SetJustifyH",
        |lua, (this, value): (LuaTable, String)| {
            let rh = region_handle_of_context(lua, &this)?;
            let parsed = justify::parse_h(&value);
            if parsed == justify::Set::NoMatch {
                return Err(benilla_lua::Error::runtime(justify::usage_h_message(
                    "FontString",
                )));
            }
            let mut model = lua.app_data_mut::<Model>().expect("model");
            let data = model.region_data.entry(rh).or_default();
            match parsed {
                justify::Set::To(value) => data.justify.set_h(value),
                justify::Set::Clears => data.justify.clear_h(),
                justify::Set::NoMatch => unreachable!("returned above"),
            }
            data.font_explicit.justify_h = true;
            Ok(())
        },
    )?;
    vm.table_set_function(&neutral_methods, "SetJustifyH", &set_justify_h)?;

    let set_justify_v = vm.create_function(
        "FontString:SetJustifyV",
        |lua, (this, value): (LuaTable, String)| {
            let rh = region_handle_of_context(lua, &this)?;
            let parsed = justify::parse_v(&value);
            if parsed == justify::Set::NoMatch {
                return Err(benilla_lua::Error::runtime(justify::usage_v_message(
                    "FontString",
                )));
            }
            let mut model = lua.app_data_mut::<Model>().expect("model");
            let data = model.region_data.entry(rh).or_default();
            match parsed {
                justify::Set::To(value) => data.justify.set_v(value),
                justify::Set::Clears => data.justify.clear_v(),
                justify::Set::NoMatch => unreachable!("returned above"),
            }
            data.font_explicit.justify_v = true;
            Ok(())
        },
    )?;
    vm.table_set_function(&neutral_methods, "SetJustifyV", &set_justify_v)?;

    let get_justify_h = vm.create_function("FontString:GetJustifyH", |lua, this: LuaTable| {
        Ok(justify_context(lua, &this)?.name_h())
    })?;
    vm.table_set_function(&neutral_methods, "GetJustifyH", &get_justify_h)?;

    let get_justify_v = vm.create_function("FontString:GetJustifyV", |lua, this: LuaTable| {
        Ok(justify_context(lua, &this)?.name_v())
    })?;
    vm.table_set_function(&neutral_methods, "GetJustifyV", &get_justify_v)?;

    let set_non_space_wrap = vm.create_function(
        "FontString:SetNonSpaceWrap",
        |lua, (this, enabled): (LuaTable, LuaValue)| {
            let enabled = match enabled {
                LuaValue::Nil => true,
                LuaValue::Boolean(enabled) => enabled,
                _ => true,
            };
            let rh = region_handle_of_context(lua, &this)?;
            lua.app_data_mut::<Model>()
                .expect("model")
                .region_data
                .entry(rh)
                .or_default()
                .non_space_wrap = Some(enabled);
            Ok(())
        },
    )?;
    vm.table_set_function(&neutral_methods, "SetNonSpaceWrap", &set_non_space_wrap)?;

    let can_non_space_wrap =
        vm.create_function("FontString:CanNonSpaceWrap", |lua, this: LuaTable| {
            let rh = region_handle_of_context(lua, &this)?;
            let model = lua.app_data_ref::<Model>().expect("model");
            let enabled = model
                .region_data
                .get(&rh)
                .and_then(|data| data.non_space_wrap)
                .unwrap_or(true);
            Ok(enabled.then_some(1_i64))
        })?;
    vm.table_set_function(&neutral_methods, "CanNonSpaceWrap", &can_non_space_wrap)?;

    let set_text_height = vm.create_function(
        "FontString:SetTextHeight",
        |lua, (this, height): (LuaTable, f32)| {
            let rh = region_handle_of_context(lua, &this)?;
            let mut model = lua.app_data_mut::<Model>().expect("model");
            model.region_data.entry(rh).or_default().text_height = Some(height);
            model.touch_measure(rh);
            Ok(())
        },
    )?;
    vm.table_set_function(&neutral_methods, "SetTextHeight", &set_text_height)?;

    super::super::font_block::install_context(
        vm,
        neutral_methods,
        region_handle_of_callback,
        "FontString",
    )
}

fn text_arg_context(value: Option<LuaValue>) -> benilla_lua::Result<Option<String>> {
    match value {
        None | Some(LuaValue::Nil) => Ok(None),
        Some(LuaValue::String(value)) => Ok(Some(value.to_string_lossy().into_owned())),
        Some(value) => String::from_lua(value).map(Some),
    }
}

fn natural_width_context(lua: &mut impl LuaContext, this: &LuaTable) -> benilla_lua::Result<f32> {
    let rh = region_handle_of_context(lua, this)?;
    crate::script::measure::ensure_measured(lua, rh);
    let model = lua.app_data_ref::<Model>().expect("model");
    let Some(data) = model.region_data.get(&rh) else {
        return Ok(0.0);
    };
    let scale = model
        .arena
        .region(rh)
        .and_then(|region| model.arena.frame(region.owner))
        .map(|frame| frame.effective_scale)
        .unwrap_or(1.0);
    Ok(data
        .measured
        .filter(|measured| measured.key == data.measure_key(scale))
        .map(|measured| measured.natural_w)
        .unwrap_or(0.0))
}

fn justify_context(
    lua: &mut impl LuaContext,
    this: &LuaTable,
) -> benilla_lua::Result<justify::Justify> {
    let rh = region_handle_of_context(lua, this)?;
    let model = lua.app_data_ref::<Model>().expect("model");
    Ok(model
        .region_data
        .get(&rh)
        .map(|data| data.justify)
        .unwrap_or_default())
}
