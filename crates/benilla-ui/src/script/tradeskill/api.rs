//! The Lua globals (`GetTradeSkillInfo`, `DoTradeSkill`, the filter family, …) — [`install`] wires
//! the Era `TradeSkillFrame.lua` API surface onto [`super::view`]'s display-tree/filter machinery.

use benilla_lua::{MultiValue, Value, Vm};

use crate::script::binding_abi::number_arg_value;
use crate::script::item_stats::item_link;
use crate::script::Model;

use super::view::{
    build_groups, first_recipe_index, inv_slot_token, num_rows, present_inv_slots, recipe_at, rows,
    select, selected_visible_index, set_collapsed, Row,
};

/// An optional string as a Lua value (`nil` when absent).
fn opt_str(s: Option<&String>) -> Value {
    s.cloned().map(Value::from).unwrap_or(Value::Nil)
}

/// A `bool` as the Era `1`/`nil` shape.
fn era_bool(b: bool) -> Value {
    if b {
        Value::Integer(1)
    } else {
        Value::Nil
    }
}

/// Register the tradeskill globals.
pub(in crate::script) fn install(lua: &mut Vm) -> benilla_lua::Result<()> {
    // GetTradeSkillLine() → lineName, rank, maxRank; ("UNKNOWN", 0, 0) with no window open (the
    // ref's own no-tradeskill shape).
    lua.register_function("GetTradeSkillLine", |lua, ()| {
        let model = lua.app_data_ref::<Model>().expect("model app_data");
        let (name, rank, max_rank) = match &model.trade_skill {
            Some(t) => (t.line_name.clone(), t.rank, t.max_rank),
            None => ("UNKNOWN".to_string(), 0, 0),
        };
        Ok(MultiValue::from(vec![
            Value::from(name),
            Value::Integer(i64::from(rank)),
            Value::Integer(i64::from(max_rank)),
        ]))
    })?;

    // → the number of VISIBLE rows the open window offers — headers + the recipes of uncollapsed
    // groups (0 when closed).
    lua.register_function("GetNumTradeSkills", |lua, ()| {
        let model = lua.app_data_ref::<Model>().expect("model app_data");
        Ok(num_rows(&model) as i64)
    })?;

    // The two link verbs (wow-re `tradeskill/scratch/tradeskill-craft-item-links.md`, 1973).
    //
    // GetTradeSkillItemLink(index) — `0x4ff410`: the number gate raises its Usage; then ZERO
    // values on every miss — an index off the list, a header row, a recipe with no product, an
    // uncached product template (no query is ever sent; the app pre-asks when the list lands) —
    // and otherwise ONE string, the product's `|Hitem:` link in its quality colour.
    lua.register_function("GetTradeSkillItemLink", |lua, index: Value| {
        let index = number_arg_value(index, "Usage: GetTradeSkillItemLink(index)")?;
        let model = lua.app_data_ref::<Model>().expect("model app_data");
        let link = usize::try_from(index)
            .ok()
            .and_then(|i| i.checked_sub(1))
            .and_then(|n| rows(&model).get(n).cloned())
            .and_then(|row| match row {
                Row::Header { .. } => None,
                Row::Entry(ei) => model.trade_skill.as_ref().map(|t| &t.recipes[ei]),
            })
            .filter(|r| r.product_item != 0)
            .and_then(|r| {
                model
                    .item_templates
                    .get(&r.product_item)
                    .map(|t| item_link(r.product_item, &t.name, t.quality))
            });
        Ok(match link {
            Some(l) => MultiValue::from(vec![Value::from(l)]),
            None => MultiValue::new(),
        })
    })?;

    // GetTradeSkillReagentItemLink(index, reagentIndex) — `0x4ff800`: both arguments through the
    // number gate, raising `Usage: GetTradeReagentSkillItemLink(…)` — Blizzard's own typo, kept.
    // `reagentIndex` is 1-based over the NON-EMPTY reagent slots and never range-checked in the
    // client; ALWAYS exactly one value: the reagent's link, or nil on any miss.
    lua.register_function(
        "GetTradeSkillReagentItemLink",
        |lua, (index, reagent): (Value, Value)| {
            let usage = "Usage: GetTradeReagentSkillItemLink(index, reagentIndex)";
            let index = number_arg_value(index, usage)?;
            let reagent = number_arg_value(reagent, usage)?;
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let link = usize::try_from(index)
                .ok()
                .and_then(|i| i.checked_sub(1))
                .and_then(|n| rows(&model).get(n).cloned())
                .and_then(|row| match row {
                    Row::Header { .. } => None,
                    Row::Entry(ei) => model.trade_skill.as_ref().map(|t| &t.recipes[ei]),
                })
                .zip(usize::try_from(reagent).ok().and_then(|r| r.checked_sub(1)))
                .and_then(|(r, ri)| r.reagents.get(ri))
                .and_then(|re| {
                    model
                        .item_templates
                        .get(&re.item)
                        .map(|t| item_link(re.item, &t.name, t.quality))
                });
            Ok(match link {
                Some(l) => Value::from(l),
                None => Value::Nil,
            })
        },
    )?;

    // GetTradeSkillInfo(index) → name, type, numAvailable, isExpanded. `index` 1-based into the
    // VISIBLE row list (the module doc's grouped-list law, wow-re `tradeskill` TU-B). A header row:
    // (groupName, "header", 0, isExpanded 1/nil). A recipe row: (name, difficulty color key,
    // numAvailable, nil). Out of range → a single nil.
    lua.register_function("GetTradeSkillInfo", |lua, index: usize| {
        let model = lua.app_data_ref::<Model>().expect("model app_data");
        let Some(n) = index.checked_sub(1) else {
            return Ok(MultiValue::from(vec![Value::Nil]));
        };
        let Some(row) = rows(&model).get(n).cloned() else {
            return Ok(MultiValue::from(vec![Value::Nil]));
        };
        match row {
            Row::Header { key, name } => {
                let expanded = !model.trade_skill_collapsed.contains(&key);
                Ok(MultiValue::from(vec![
                    Value::from(name),
                    Value::from("header"),
                    Value::Integer(0),
                    era_bool(expanded),
                ]))
            }
            Row::Entry(ei) => {
                let r = &model
                    .trade_skill
                    .as_ref()
                    .expect("rows() only yields Entry rows when trade_skill is Some")
                    .recipes[ei];
                Ok(MultiValue::from(vec![
                    Value::from(r.name.as_str()),
                    Value::from(r.difficulty.as_str()),
                    Value::Integer(i64::from(r.num_available)),
                    Value::Nil,
                ]))
            }
        }
    })?;

    // GetFirstTradeSkill() → the first NON-header visible index (0 when none).
    lua.register_function("GetFirstTradeSkill", |lua, ()| {
        let model = lua.app_data_ref::<Model>().expect("model app_data");
        Ok(i64::from(first_recipe_index(&model)))
    })?;

    // GetTradeSkillSubClasses() → the current group names, in group order (VERIFIED, wow-re
    // `tradeskill` TU-B: the filter-dropdown vocabulary IS the header list) — though v1 ships no
    // filter dropdown to consume it yet (the SubClass/InvSlot filter family below stays inert).
    lua.register_function("GetTradeSkillSubClasses", |lua, ()| {
        let model = lua.app_data_ref::<Model>().expect("model app_data");
        let mut out = Vec::new();
        if let Some(t) = model.trade_skill.as_ref() {
            for grp in build_groups(&t.recipes) {
                out.push(Value::from(grp.name));
            }
        }
        Ok(MultiValue::from(out))
    })?;

    // ExpandTradeSkillSubClass(i) / CollapseTradeSkillSubClass(i) — fold a group by its header's
    // VISIBLE index (i == 0 = ALL groups, the CollapseAll semantics — see [`set_collapsed`]).
    lua.register_function("ExpandTradeSkillSubClass", |lua, id: usize| {
        let mut model = lua.app_data_mut::<Model>().expect("model app_data");
        set_collapsed(&mut model, id, false);
        model.trade_skill_touched = true;
        Ok(())
    })?;
    lua.register_function("CollapseTradeSkillSubClass", |lua, id: usize| {
        let mut model = lua.app_data_mut::<Model>().expect("model app_data");
        set_collapsed(&mut model, id, true);
        model.trade_skill_touched = true;
        Ok(())
    })?;

    // GetTradeSkillIcon(index) → icon texture path (nil while in flight / OOB / a header row).
    lua.register_function("GetTradeSkillIcon", |lua, index: usize| {
        let model = lua.app_data_ref::<Model>().expect("model app_data");
        Ok(opt_str(
            recipe_at(&model, index).and_then(|r| r.icon.as_ref()),
        ))
    })?;

    // GetTradeSkillNumMade(index) → minMade, maxMade (0, 0 when OOB / no window / a header row).
    lua.register_function("GetTradeSkillNumMade", |lua, index: usize| {
        let model = lua.app_data_ref::<Model>().expect("model app_data");
        let (min_made, max_made) =
            recipe_at(&model, index).map_or((0, 0), |r| (r.min_made, r.max_made));
        Ok(MultiValue::from(vec![
            Value::Integer(i64::from(min_made)),
            Value::Integer(i64::from(max_made)),
        ]))
    })?;

    // GetTradeSkillCooldown(index) → remaining cooldown seconds, or nil when ready / OOB / a header
    // row (the ref Lua tests this return for truthiness).
    lua.register_function("GetTradeSkillCooldown", |lua, index: usize| {
        let model = lua.app_data_ref::<Model>().expect("model app_data");
        Ok(
            match recipe_at(&model, index).and_then(|r| r.cooldown_secs) {
                Some(secs) => Value::Integer(secs as i64),
                None => Value::Nil,
            },
        )
    })?;

    // GetTradeSkillNumReagents(index) → this recipe's reagent count (0 when OOB / no window / a
    // header row).
    lua.register_function("GetTradeSkillNumReagents", |lua, index: usize| {
        let model = lua.app_data_ref::<Model>().expect("model app_data");
        Ok(recipe_at(&model, index).map_or(0, |r| r.reagents.len()) as i64)
    })?;

    // GetTradeSkillReagentInfo(index, reagentIndex) → name, icon, need, have (a single nil when the
    // recipe/reagent index is OOB or `index` is a header row). name/icon are themselves nil while
    // the ask-once template answer is in flight — the ref grays/counts the row off exactly these
    // four.
    lua.register_function(
        "GetTradeSkillReagentInfo",
        |lua, (index, reagent_index): (usize, usize)| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let Some(reagent) = recipe_at(&model, index)
                .and_then(|r| reagent_index.checked_sub(1).and_then(|n| r.reagents.get(n)))
            else {
                return Ok(MultiValue::from(vec![Value::Nil]));
            };
            Ok(MultiValue::from(vec![
                opt_str(reagent.name.as_ref()),
                opt_str(reagent.icon.as_ref()),
                Value::Integer(i64::from(reagent.need)),
                Value::Integer(i64::from(reagent.have)),
            ]))
        },
    )?;

    // GetTradeSkillTools(index) → an alternating (name, has) multivalue, one pair per Requirements
    // tool (e.g. "Anvil", 1, "Mining Pick", nil) — the ref feeds this straight into
    // BuildColoredListString. Empty when the recipe has no tools / index is OOB / a header row.
    lua.register_function("GetTradeSkillTools", |lua, index: usize| {
        let model = lua.app_data_ref::<Model>().expect("model app_data");
        let mut out = Vec::new();
        if let Some(r) = recipe_at(&model, index) {
            for (name, have) in &r.tools {
                out.push(Value::from(name.as_str()));
                out.push(era_bool(*have));
            }
        }
        Ok(MultiValue::from(out))
    })?;

    // GetTradeskillRepeatCount() → the remaining Create All repeats (note the lowercase "s" in
    // "Tradeskill" — that IS the real 1.12 API name). 0 with no window open.
    lua.register_function("GetTradeskillRepeatCount", |lua, ()| {
        let model = lua.app_data_ref::<Model>().expect("model app_data");
        Ok(i64::from(
            model.trade_skill.as_ref().map_or(0, |t| t.repeat_count),
        ))
    })?;

    // SelectTradeSkill(index) / GetTradeSkillSelectionIndex() — the engine-held selection, VISIBLE
    // index in ([`select`]: a header index is IGNORED, not cleared), VISIBLE index out
    // ([`selected_visible_index`]), held internally as a stable flat-recipe position (module doc).
    lua.register_function("SelectTradeSkill", |lua, index: u32| {
        let mut model = lua.app_data_mut::<Model>().expect("model app_data");
        select(&mut model, index);
        Ok(())
    })?;
    lua.register_function("GetTradeSkillSelectionIndex", |lua, ()| {
        let model = lua.app_data_ref::<Model>().expect("model app_data");
        Ok(i64::from(selected_visible_index(&model)))
    })?;

    // DoTradeSkill(index, count) — queue the recipe's SPELL ID for `count` crafts (default 1, never
    // less than 1) — the app's repeat machine turns this into that many CMSG_CAST_SPELL sends. Out
    // of range / a header index → ignored.
    lua.register_function(
        "DoTradeSkill",
        |lua, (index, count): (usize, Option<i64>)| {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            if let Some((spell_id, avail)) =
                recipe_at(&model, index).map(|r| (r.spell_id, r.num_available))
            {
                // The client clamps the repeat to numAvailable at the latch (byte-VERIFIED —
                // wow-re `tradeskill` TU-D, `DoTradeSkill 0x500280`).
                let n = (count.unwrap_or(1).max(1) as u32).min(avail.max(1));
                model.trade_skill_dos.push((spell_id, n));
            }
            Ok(())
        },
    )?;

    // CloseTradeSkill() — client-side close (no packet, vanilla): flag it so the app clears its
    // local state.
    lua.register_function("CloseTradeSkill", |lua, ()| {
        let mut model = lua.app_data_mut::<Model>().expect("model app_data");
        model.trade_skill_close = true;
        Ok(())
    })?;

    // The sub-class/inv-slot FILTER family — real now (the dropdowns ship, ref
    // Blizzard_TradeSkillUI.lua l.314-414). Indexing follows the C originals: the SubClass index
    // is 1-based into the CURRENT GetTradeSkillSubClasses order (`0x4ffc70` bounds-checks against
    // the live header count), the InvSlot index 1-based into the GetTradeSkillInvSlots order;
    // index 0 is the "All" pseudo-entry on both. `Set*(0, 1, …)` = everything back on (the ref's
    // "All Subclasses"/"All Slots" menu row); `Set*(i, 1, 1)` = EXCLUSIVE — only entry i shown
    // (every ref menu click passes exactly this shape); `Set*(i, 1)` un-hides i; `Set*(i, 0)`
    // hides it. Each set re-lists + fires TRADE_SKILL_UPDATE via the touched flag (the `0x4fd710`
    // re-sort + event 0x139 shape). `Get*(0)` answers "is everything shown" — the ref's
    // all-checked probe.
    lua.register_function("GetTradeSkillSubClassFilter", |lua, index: usize| {
        let model = lua.app_data_ref::<Model>().expect("model app_data");
        let Some(t) = model.trade_skill.as_ref() else {
            return Ok(era_bool(false));
        };
        let groups = build_groups(&t.recipes);
        Ok(match index.checked_sub(1) {
            None => era_bool(
                !groups
                    .iter()
                    .any(|g| model.trade_skill_subclass_hidden.contains(&g.key)),
            ),
            Some(n) => era_bool(
                groups
                    .get(n)
                    .is_some_and(|g| !model.trade_skill_subclass_hidden.contains(&g.key)),
            ),
        })
    })?;
    lua.register_function(
        "SetTradeSkillSubClassFilter",
        |lua, (index, on, exclusive): (usize, Option<i64>, Option<i64>)| {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            let Some(t) = model.trade_skill.as_ref() else {
                return Ok(());
            };
            let keys: Vec<(u32, u32)> = build_groups(&t.recipes)
                .into_iter()
                .map(|g| g.key)
                .collect();
            let on = on.unwrap_or(1) != 0;
            let exclusive = exclusive.unwrap_or(0) != 0;
            match index.checked_sub(1) {
                None => model.trade_skill_subclass_hidden.clear(),
                Some(n) => {
                    let Some(&key) = keys.get(n) else {
                        return Ok(());
                    };
                    if on && exclusive {
                        model.trade_skill_subclass_hidden =
                            keys.iter().copied().filter(|&k| k != key).collect();
                    } else if on {
                        model.trade_skill_subclass_hidden.remove(&key);
                    } else {
                        model.trade_skill_subclass_hidden.insert(key);
                    }
                }
            }
            model.trade_skill_touched = true;
            Ok(())
        },
    )?;
    // GetTradeSkillInvSlots() → the distinct slot words of the open window's products, ascending
    // slot-bit order (the `0xbde058` bit walk — [`present_inv_slots`]).
    lua.register_function("GetTradeSkillInvSlots", |lua, ()| {
        let bits = {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            present_inv_slots(&model)
        };
        let mut out = Vec::new();
        // Each word is its `0x84dd70` token resolved off the player's own string table
        // (decision 2045). A token the table does not carry answers the empty string rather
        // than being dropped: the list is POSITIONAL — `GetTradeSkillInvSlotFilter(index)`
        // indexes the same order — so a hole would shift every filter after it.
        for bit in bits {
            let word = inv_slot_token(bit)
                .and_then(|k| crate::strings::global_context(lua, k))
                .unwrap_or_default();
            out.push(Value::from(word));
        }
        Ok(MultiValue::from(out))
    })?;
    // The InvSlot pair reads/writes the shown-mask (`0x84dd64`) with the index resolving to the
    // (index-1)-th SET bit of the accumulated present-slots mask — present slots only, ascending
    // (TU-G §4's enumeration; `GetTradeSkillInvSlots` returns exactly that order).
    lua.register_function("GetTradeSkillInvSlotFilter", |lua, index: usize| {
        let model = lua.app_data_ref::<Model>().expect("model app_data");
        if model.trade_skill.is_none() {
            return Ok(era_bool(false));
        }
        let bits = present_inv_slots(&model);
        Ok(match index.checked_sub(1) {
            // The "all shown?" probe: (present & mask) == present (TU-G §4, `0x4fffd0`).
            None => era_bool(
                bits.iter()
                    .all(|&b| model.trade_skill_invslot_mask & (1 << b) != 0),
            ),
            Some(n) => era_bool(
                bits.get(n)
                    .is_some_and(|&b| model.trade_skill_invslot_mask & (1 << b) != 0),
            ),
        })
    })?;
    lua.register_function(
        "SetTradeSkillInvSlotFilter",
        |lua, (index, on, exclusive): (usize, Option<i64>, Option<i64>)| {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            if model.trade_skill.is_none() {
                return Ok(());
            }
            let bits = present_inv_slots(&model);
            let on = on.unwrap_or(1) != 0;
            let exclusive = exclusive.unwrap_or(0) != 0;
            // The exact mask math per path (TU-G §4, the recovered `0x4fd730` args): all →
            // 0xffffffff · off → old & ~(1<<b) · exclusive → 1<<b · add → old | (1<<b).
            match index.checked_sub(1) {
                None => model.trade_skill_invslot_mask = u32::MAX,
                Some(n) => {
                    let Some(&bit) = bits.get(n) else {
                        return Ok(());
                    };
                    if on && exclusive {
                        model.trade_skill_invslot_mask = 1 << bit;
                    } else if on {
                        model.trade_skill_invslot_mask |= 1 << bit;
                    } else {
                        model.trade_skill_invslot_mask &= !(1 << bit);
                    }
                }
            }
            model.trade_skill_touched = true;
            Ok(())
        },
    )?;

    Ok(())
}
