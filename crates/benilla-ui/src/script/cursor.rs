//! The cursor payload (decision 0216, slice 1) — the client's payload-mode global `[0xb4d900]`
//! as a typed enum, and the drag-gesture mechanics (`RegisterForDrag`/`OnDragStart`/`OnDragStop`/
//! `OnReceiveDrag`) that transition it. One seam for every surface — bags, the paper doll
//! ([`doll`], decision 0208 phase 1b), action bars ([`bar`], decision 0216 §7/0218 §4), and the
//! spellbook ([`super::spellbook`]'s `PickupSpell`, slice 5 — the [`CursorSpell`] producer) — so
//! sounds, `CURSOR_UPDATE`, grid events, and lock display can't drift apart per window.
//!
//! [`container`](super::container)'s `pickup_container_item`, [`doll`]'s
//! `pickup_inventory_item`, and [`bar`]'s `pickup_action`/`place_action` (the surface-specific
//! transition bodies) all route through [`queue_cursor_update`]/[`queue_lock_changed`], the one
//! place those events fire from. [`bar`], [`doll`], and [`drag`] are split out purely for size —
//! this file keeps the payload types, the shared transition seam, and `install`.

use benilla_lua::{Value, Vm};

use super::{Model, ScriptValue};

mod bag_verbs;
mod bar;
mod doll;
mod drag;
pub(crate) mod money;
mod pet;

pub(crate) use bar::place_action;
pub(crate) use drag::{
    abandon_drag, arm_drag, maybe_start_drag, take_drag, DragGesture, DragRelease,
};

pub use crate::script_data::{
    CursorAction, CursorItem, CursorMacro, CursorMerchantItem, CursorMoney, CursorPayload,
    CursorPetAction, CursorSpell, CursorStablePet, EnchantConfirm, WorldPick, EQUIPMENT_BAG,
};

/// How many items the cursor is actually holding, for a payload picked up from a bag.
///
/// [`CursorItem::count`] is `None` for a **whole-stack** pickup — it records only what a *split*
/// carry took — so reading it as `unwrap_or(1)` answers 1 for a stack of twenty. That is a
/// display wart on a mail attachment and a real arithmetic error on an auction deposit, which is
/// computed per stack: it would quote a twentieth of what the server then charges. So the true
/// size comes back from the source slot, which still holds the stack (a pickup locks the slot, it
/// does not empty it).
///
/// Falls back to 1 only when the source slot is genuinely unreadable — a bag the app has not
/// pushed, or a slot that changed under us.
pub(super) fn held_count(model: &super::Model, item: &CursorItem) -> u32 {
    if let Some(n) = item.count {
        return n.max(1);
    }
    model
        .containers
        .get(&item.bag)
        .and_then(|c| c.slots.get(&item.slot))
        .filter(|s| s.item_id == item.item_id)
        .map_or(1, |s| s.count.max(1))
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// The one transition seam: CURSOR_UPDATE / ITEM_LOCK_CHANGED / DELETE_ITEM_CONFIRM
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// Queue `CURSOR_UPDATE` — fired on EVERY payload transition (pickup, place/clear, cancel,
/// `ClearCursor`, `DeleteCursorItem`, a world-drop clear). One push per transition.
///
/// Also derives `ACTIONBAR_SHOWGRID`/`ACTIONBAR_HIDEGRID` (decision 0216 §7) off
/// [`Model::cursor_grid_shown`]'s mirror against the CURRENT `model.cursor` (already the
/// post-transition state at every call site — every caller mutates `model.cursor` before calling
/// this): a None→Some edge (any payload arm, any surface — bags/doll/actions alike) shows the
/// bar's drop grid, Some→None hides it, Some→Some (the action hop) touches neither, so one
/// gesture never churns HIDE+SHOW. This is the one seam every pickup/place/clear already routes
/// through, so no call site needs to know about grid events at all.
pub(crate) fn queue_cursor_update(model: &mut Model) {
    model
        .pending_events
        .push(("CURSOR_UPDATE".to_string(), Vec::new()));
    // The two grids are derived off DIFFERENT predicates, because the reference fires them from
    // different places and the payload spaces do not overlap (decision 1010):
    //
    // - the ACTION bar's grid follows "is anything held that could land there" — every arm except
    //   the pet one, which `PlaceAction` refuses outright;
    // - the PET bar's grid follows the pet payload alone. The reference fires `PET_BAR_SHOWGRID`
    //   from inside the pet-action pickup builder itself (`0x494f28`), not from a shared cursor
    //   transition — so a spell on the cursor lights the action bar's empty slots and leaves the
    //   pet bar alone, which is also the only honest answer: you cannot drop it there.
    let pet_held = matches!(model.cursor, Some(CursorPayload::PetAction(_)));
    let bar_held = model.cursor.is_some() && !pet_held;
    if bar_held != model.cursor_grid_shown {
        model.cursor_grid_shown = bar_held;
        let event = if bar_held {
            "ACTIONBAR_SHOWGRID"
        } else {
            "ACTIONBAR_HIDEGRID"
        };
        model.pending_events.push((event.to_string(), Vec::new()));
    }
    if pet_held != model.pet_grid_shown {
        model.pet_grid_shown = pet_held;
        let event = if pet_held {
            "PET_BAR_SHOWGRID"
        } else {
            "PET_BAR_HIDEGRID"
        };
        model.pending_events.push((event.to_string(), Vec::new()));
    }
}

/// Queue `ITEM_LOCK_CHANGED(bag, slot)` — a pickup locking a source slot, or a place/cancel/
/// clear/destroy unlocking it (decision 0216 §4: the engine-derived held-here lock,
/// `container.rs`'s `GetContainerItemInfo` `held_here` check).
pub(crate) fn queue_lock_changed(model: &mut Model, bag: i64, slot: u32) {
    model.pending_events.push((
        "ITEM_LOCK_CHANGED".to_string(),
        vec![ScriptValue::Int(bag), ScriptValue::Int(i64::from(slot))],
    ));
}

/// Queue `DELETE_ITEM_CONFIRM(name, quality)` for an item payload dropped on the world (the popup
/// flow transcribed from `StaticPopup.lua`, decision 0216 §3). `name` is parsed out of the link's
/// `[...]` segment (empty string with no link); the payload is left untouched here — the popup's
/// `OnAccept`/`OnCancel` (`DeleteCursorItem`/`ClearCursor`) own clearing it.
fn queue_delete_item_confirm(model: &mut Model, item: &CursorItem) {
    let name = item_link_name(item.link.as_deref());
    let quality = item.quality.unwrap_or(0);
    model.pending_events.push((
        "DELETE_ITEM_CONFIRM".to_string(),
        vec![ScriptValue::Str(name), ScriptValue::Int(i64::from(quality))],
    ));
}

/// Parse an item link's display name out of its `|h[Name]|h` segment; empty with no link or an
/// unrecognized shape.
pub(super) fn item_link_name(link: Option<&str>) -> String {
    link.and_then(|l| l.split_once('['))
        .and_then(|(_, rest)| rest.split_once(']'))
        .map(|(name, _)| name.to_string())
        .unwrap_or_default()
}

/// `ClearCursor()` — drops whatever the cursor holds, any arm (the right-click/ESC cancel). An
/// item payload also un-locks its source slot; an already-empty cursor is not a transition (no
/// event).
pub(crate) fn clear_cursor(model: &mut Model) {
    // **An armed gift wrap dies with any cancel** (decision 1934), and this is the FIRST thing
    // the reference's `ClearCursor 0x495190` does — `0x5edf10` is its opening act and it is
    // **ungated by either parameter**, including `ClearCursor(0)`, the flavour that deliberately
    // keeps an ordinary held item's lock. So all 70 of its call sites cancel a wrap. The paper
    // unlocks and the cursor mode goes back to the base.
    if let Some(wrap) = model.pending_wrap.take() {
        queue_lock_changed(model, wrap.bag, wrap.slot);
        model.ui_cursor = None;
        model.ui_cursor_dirty = true;
    }
    match model.cursor.take() {
        Some(CursorPayload::Item(item)) => {
            queue_cursor_update(model);
            queue_lock_changed(model, item.bag, item.slot);
        }
        Some(
            CursorPayload::Spell(_)
            | CursorPayload::Action(_)
            | CursorPayload::Macro(_)
            | CursorPayload::PetAction(_)
            | CursorPayload::StablePet(_)
            // Mode 2 — coins go back where they never left (the purse was not debited).
            | CursorPayload::Money(_)
            // Mode 5 clears like every other non-item payload, and clearing it sends NO packet —
            // a vendor cursor abandoned on the world, on ESC, or by the window closing costs
            // nothing. `ClearCursor`'s own mode-5 arm is `0x49525f`.
            | CursorPayload::Merchant(_),
        ) => {
            queue_cursor_update(model);
        }
        None => {}
    }
}

/// **The SELL fork's cursor clear** — `0x494b60 SetCursorItem(0, …)`, which is *not* `ClearCursor`
/// and differs from [`clear_cursor`] in exactly one way that a player can see.
///
/// Both drop the payload and fire `CURSOR_UPDATE`. `ClearCursor` also un-locks the source slot;
/// this does **not**: `0x494b60`'s mode-1 arm is entered with `param1 = 0`, which skips `0x495420`,
/// so the sold item's slot stays **greyed** until the server's own inventory update removes it
/// (wow-re `object-layer/scratch/vendor-sell-on-right-click.md` §3, the §5 trio dispatched from
/// benilla 1905). Leaving the lock set is the point: the item is gone from the cursor but not yet
/// gone from the bag, and the grey is what says so.
///
/// Shared by the two verbs that sell off the cursor — `PickupMerchantItem`'s sell fork
/// (`0x4fb760`) and the interact ladder's vendor arm (`0x5df5d0`) — because they are the same
/// clear in the binary, and two copies of it is how one of them quietly grows a different
/// behaviour. Returns the item so the caller can address the packet by its slot.
///
/// A non-item payload is left exactly where it was: the fork answers for **mode 1 only**
/// (`GetCursorItem 0x494c60`), so a spell, a macro or a vendor row on the cursor is not a sale
/// and must not be dropped by asking.
pub(crate) fn take_cursor_item_for_sale(model: &mut Model) -> Option<CursorItem> {
    let item = match model.cursor.take() {
        Some(CursorPayload::Item(item)) => item,
        other => {
            model.cursor = other;
            return None;
        }
    };
    queue_cursor_update(model);
    Some(item)
}

/// What the app's world pick resolves under the cursor this frame — the reference's click-time
/// pick state `[this+0x350]` (0 = nothing / 1 = terrain / 2 = object; `0x481f60`, decisions
/// 0571 + 0574). Fed per frame by the app ([`super::UiScript::set_world_pick`]; stays
/// [`WorldPick::Nothing`] in tests/captures). Routes the left world-drop
/// ([`world_drop_click`]): over an `Object` no payload drops at all (the object leg `0x492ce0`
/// clears only the displayId-PREVIEW gate `[0xb4b41c]` — an arm benilla doesn't carry — and
/// dispatches SELECT/INTERACT with the payload still held); over `Terrain` an ITEM drops (the
/// `DELETE_ITEM_CONFIRM` popup via `0x5e0320`) and a spell/action payload clears silently
/// (decision 0843's deliberate divergence — the reference keeps it there); over `Nothing` the
/// item pops and any other payload silently clears (`0x492d30`).
/// A world drop: a completed left CLICK on the game world while a payload is held — press and
/// release both on the world (the world frame, or no frame at all where none is loaded — the
/// caller's `over_world`, decision 2089), no drag (the byte-verified trigger, decision 0218: the
/// client's `0x495300` runs on the WorldFrame click release only; a drag released over the world
/// routes as a drag and keeps carrying) — routed by [`Model::world_pick`] (decisions 0571 + 0574,
/// byte-verified wow-re cursor-dragdrop-payload.md §11):
///
/// - `Object`: NOTHING drops — the object leg (`0x492ce0`) keeps every real payload and
///   dispatches SELECT instead.
/// - `Terrain`: an item payload fires `DELETE_ITEM_CONFIRM(name, quality)` and STAYS held (the
///   reference popup's `OnAccept` calls `DeleteCursorItem`, `OnCancel` `ClearCursor`, and its
///   `OnUpdate` auto-hides when the cursor empties — the engine must not pre-clear); a
///   spell/action payload clears silently — a DELIBERATE divergence (decision 0843, the
///   director's call): the reference's terrain leg keeps a non-item payload on the left click
///   (`0x492c90`/`0x5e0320` clear non-items only on the right button), which leaves a spell
///   stuck to the cursor with no left-handed way off it.
/// - `Nothing`: the item pops the same popup; any other payload clears silently (`0x492d30`'s
///   non-item arm — here the reference agrees).
///
/// Returns whether the click was consumed as a drop (an empty cursor or an `Object` pick
/// consume nothing).
pub(crate) fn world_drop_click(model: &mut Model) -> bool {
    match (&model.cursor, model.world_pick) {
        (Some(CursorPayload::Item(item)), WorldPick::Terrain | WorldPick::Nothing) => {
            let item = item.clone();
            queue_delete_item_confirm(model, &item);
            true
        }
        (
            Some(
                CursorPayload::Spell(_)
                | CursorPayload::Action(_)
                | CursorPayload::Macro(_)
                | CursorPayload::PetAction(_),
            ),
            WorldPick::Terrain | WorldPick::Nothing,
        ) => {
            clear_cursor(model);
            true
        }
        _ => false,
    }
}

/// `SplitContainerItem(bag, slot, count)` — a stack-split pickup (`StackSplitFrame.lua`'s
/// `SplitStack`): only from an EMPTY cursor, onto a resolved and unlocked slot, `count >= 1`.
/// `count` at or past the whole stack degrades to a plain whole-stack pickup (`count: None` — the
/// client never "splits" a full stack; vmangos rejects `count == stack` as cheat). Returns
/// whether a payload was picked up (the repaint gate, matching `pickup_container_item`'s
/// contract).
pub(crate) fn split_container_item(model: &mut Model, bag: i64, slot: u32, count: i64) -> bool {
    if model.cursor.is_some() || count < 1 {
        return false;
    }
    let Some(s) = model.containers.get(&bag).and_then(|c| c.slots.get(&slot)) else {
        return false;
    };
    if s.item_id == 0 || s.locked {
        return false;
    }
    let n = count.min(i64::from(s.count)) as u32;
    let whole = n >= s.count;
    let item = CursorItem {
        bag,
        slot,
        item_id: s.item_id,
        texture: s.texture.clone(),
        link: s.link.clone(),
        quality: s.quality,
        count: if whole { None } else { Some(n) },
        bar_placeable: s.bar_placeable,
        equip_slots: s.equip_slots.clone(),
    };
    model.cursor = Some(CursorPayload::Item(item));
    queue_cursor_update(model);
    queue_lock_changed(model, bag, slot);
    true
}

/// `DeleteCursorItem()` — the delete popup's `OnAccept` (decision 0216 §3): an Item payload
/// queues its `(bag, slot, count)` destroy (`count == 0` = the whole stack) and clears; any other
/// payload (or an empty cursor) is a no-op — the client's contract for a stale/mismatched confirm.
pub(crate) fn delete_cursor_item(model: &mut Model) {
    let Some(CursorPayload::Item(item)) = &model.cursor else {
        return;
    };
    let item = item.clone();
    model.cursor = None;
    model
        .container_destroys
        .push((item.bag, item.slot, item.count.unwrap_or(0)));
    queue_cursor_update(model);
    queue_lock_changed(model, item.bag, item.slot);
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// UiScript accessors
// ─────────────────────────────────────────────────────────────────────────────────────────────

impl super::UiScript {
    /// The item currently held on the cursor, or `None` — the Item-arm projection of
    /// [`UiScript::cursor_payload`], kept for the app's existing extract/sound watchers
    /// (`ui_script/extract.rs`, `sound/ui.rs`), which only ever cared about items.
    pub fn cursor_item(&self) -> Option<CursorItem> {
        match self.model_ref().cursor.clone() {
            Some(CursorPayload::Item(item)) => Some(item),
            _ => None,
        }
    }

    /// The full cursor payload, any arm — what the app's hardware-cursor drive
    /// (`benilla::cursor`) and capture stand-in draw, and what the world-click consumers gate on.
    pub fn cursor_payload(&self) -> Option<CursorPayload> {
        self.model_ref().cursor.clone()
    }

    /// Feed: what the app's world pick resolves under the cursor this frame
    /// ([`Model::world_pick`], decisions 0571 + 0574) — routes [`world_drop_click`]'s legs
    /// (object keeps everything, terrain drops items only, nothing drops any arm).
    pub fn set_world_pick(&mut self, pick: WorldPick) {
        self.model_mut().world_pick = pick;
    }

    /// The sell fork's take-and-clear, for the app-side interact ladder — see
    /// [`take_cursor_item_for_sale`]. `None` for an empty cursor or any non-item payload, and in
    /// that case the payload is left untouched.
    pub fn take_cursor_item_for_sale(&mut self) -> Option<CursorItem> {
        take_cursor_item_for_sale(&mut self.model_mut())
    }

    /// `ClearCursor()`'s Rust seam — drops whatever the cursor holds, any arm, silently (fires
    /// `CURSOR_UPDATE` + the item source un-lock, exactly the Lua `ClearCursor`). The app's
    /// world-click router calls it for a clean RIGHT-click over empty world (`0x492c90`/
    /// `0x492d30`'s action-4 leg: `ClearCursor 0x495190(1,1)` unconditionally — decision 0571).
    pub fn clear_cursor_payload(&mut self) {
        clear_cursor(&mut self.model_mut());
    }

    /// Test-only: set the cursor payload directly, bypassing every Lua binding — drives the
    /// Spell/Action arms in tests before a spellbook/action-bar slice can produce them.
    #[cfg(test)]
    pub(crate) fn set_cursor_for_test(&mut self, payload: CursorPayload) {
        self.model_mut().cursor = Some(payload);
    }
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Lua globals: GetCursorInfo / CursorHasItem / CursorHasSpell / ClearCursor / SplitContainerItem /
// DeleteCursorItem — all top-level (the real client's `GetCursorInfo` &c. are not namespaced).
impl super::UiScript {
    /// Arm (or disarm) **the targeting cursor's item half** — the engine mirror of
    /// `TargetingWantsItem 0x6e6330`. While armed, a bag click ([`super::container`]) or a
    /// paper-doll click ([`doll::pickup_inventory_item`]) queues its `(bag, slot)` into the pick
    /// list instead of running the cursor gesture, exactly as the reference's two pickup
    /// functions reroute (`0x4f9b30` @ `4f9c54`, `0x4c7300` @ `4c76df`). The app owns the word:
    /// it arms on a resolved item-targeted cast and clears on the bind, a cancel, or ESC
    /// (decision 0923 — before it, this was the CraftFrame's private enchant pick).
    pub fn set_item_pick_armed(&mut self, armed: bool) {
        self.model_mut().item_pick_armed = armed;
    }

    /// Push what `SpellCanTargetUnit` answers — whether the standing targeting word could bind a
    /// unit at all (see [`crate::script::Model::spell_can_target_unit`]).
    pub fn set_spell_can_target_unit(&mut self, can: bool) {
        self.model_mut().spell_can_target_unit = can;
    }

    /// Drain the `(bag, slot)` clicks the armed item half consumed since the last call. A doll
    /// click reports as [`EQUIPMENT_BAG`] + its 1-based inventory slot — the ONE bag space, so
    /// the app resolves both seams with one lookup.
    pub fn take_item_picks(&mut self) -> Vec<(i64, u32)> {
        std::mem::take(&mut self.model_mut().item_picks)
    }

    /// Drain the enchant-confirm popups' answers since the last call (decision 0928). Both are
    /// answers to the *same* pick the app parked, which is why they share one queue.
    pub fn take_enchant_confirms(&mut self) -> Vec<EnchantConfirm> {
        std::mem::take(&mut self.model_mut().enchant_confirms)
    }
}

// The paper-doll globals (`PickupInventoryItem` &c.) install from [`doll`]; the action-bar
// globals (`PickupAction`/`PlaceAction`) install from [`bar`].
// ─────────────────────────────────────────────────────────────────────────────────────────────

pub(super) fn install(lua: &mut Vm) -> benilla_lua::Result<()> {
    // BindEnchant() / ReplaceEnchant() — the two enchant-confirm popups' OnAccept (StaticPopup.lua
    // `BIND_ENCHANT` l.1240 and `REPLACE_ENCHANT` l.1252). Both queue an intent the app answers
    // over its parked item guid; neither sends anything from here.
    for (name, answer) in [
        ("BindEnchant", EnchantConfirm::Bind),
        ("ReplaceEnchant", EnchantConfirm::Replace),
    ] {
        lua.register_function(name, move |lua, ()| {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            model.enchant_confirms.push(answer);
            Ok(())
        })?;
    }

    // GetCursorInfo() → per arm: Item ("item", itemID, itemLink); Spell ("spell", book_slot,
    // book_type, spell_id) — the Era shape; Action ("action", src_slot). Empty ⇒ all nils. A
    // fixed 4-return shape (padded with nil past each arm's own fields) so one function signature
    // covers every arm; callers destructuring fewer names simply ignore the tail.
    lua.register_function("GetCursorInfo", |lua, ()| {
        let model = lua.app_data_ref::<Model>().expect("model app_data");
        match &model.cursor {
            Some(CursorPayload::Item(c)) => Ok((
                Value::from("item"),
                Value::Integer(i64::from(c.item_id)),
                c.link.clone().map_or(Value::Nil, Value::from),
                Value::Nil,
            )),
            Some(CursorPayload::Spell(s)) => Ok((
                Value::from("spell"),
                Value::Integer(i64::from(s.book_slot)),
                Value::from(s.book_type.clone()),
                Value::Integer(i64::from(s.spell_id)),
            )),
            Some(CursorPayload::Action(a)) => Ok((
                Value::from("action"),
                Value::Integer(i64::from(a.src_slot)),
                Value::Nil,
                Value::Nil,
            )),
            // The Era `GetCursorInfo` shape for a macro: the kind word + the macro index.
            Some(CursorPayload::Macro(m)) => Ok((
                Value::from("macro"),
                Value::Integer(i64::from(m.index)),
                Value::Nil,
                Value::Nil,
            )),
            // The pet arm follows the Action arm's shape — kind word + the slot it came from.
            // Nothing in the shipped 1.12 UI reads it (the pet bar's own drag never asks what
            // it is carrying); it is here so the ONE payload space stays fully describable.
            Some(CursorPayload::PetAction(p)) => Ok((
                Value::from("petaction"),
                Value::Integer(i64::from(p.src_slot)),
                Value::Nil,
                Value::Nil,
            )),
            // Mode 10 — the stabled-pet grab (decision 1677). Reported as nothing: the
            // reference's `GetCursorInfo` arms are not carved for it, the stable window never
            // asks, and inventing a type string would put a guess where an addon could read it.
            Some(CursorPayload::StablePet(_)) => {
                Ok((Value::Nil, Value::Nil, Value::Nil, Value::Nil))
            }
            // Mode 2 — coins (1962). Reported as nothing for the same reason: the money arm
            // of `GetCursorInfo` is not carved, and `GetCursorMoney` is the reference's own
            // way of asking. Joins the carve when it lands.
            Some(CursorPayload::Money(_)) => Ok((Value::Nil, Value::Nil, Value::Nil, Value::Nil)),
            // Mode 5 — the vendor grab. Reported as nothing, for the same reason and with a
            // stronger warrant: **no registered binding in the 5875 image exposes mode 5 to
            // Lua at all.** `CursorHasItem`, `CursorHasSpell`, `CursorHasMoney` and
            // `CursorCanGoInSlot` are each nil for it, by census. A vendor cursor is invisible
            // from Lua and that invisibility is load-bearing — it is what keeps all thirteen
            // stock `CursorHasItem()` gates closed while one is held.
            Some(CursorPayload::Merchant(_)) => {
                Ok((Value::Nil, Value::Nil, Value::Nil, Value::Nil))
            }
            None => Ok((Value::Nil, Value::Nil, Value::Nil, Value::Nil)),
        }
    })?;

    lua.register_function("CursorHasItem", |lua, ()| {
        let model = lua.app_data_ref::<Model>().expect("model app_data");
        Ok(matches!(model.cursor, Some(CursorPayload::Item(_))))
    })?;
    lua.register_function("CursorHasSpell", |lua, ()| {
        let model = lua.app_data_ref::<Model>().expect("model app_data");
        Ok(matches!(model.cursor, Some(CursorPayload::Spell(_))))
    })?;
    lua.register_function("ClearCursor", |lua, ()| {
        let mut model = lua.app_data_mut::<Model>().expect("model app_data");
        clear_cursor(&mut model);
        Ok(())
    })?;

    lua.register_function(
        "SplitContainerItem",
        |lua, (bag, slot, count): (i64, u32, i64)| {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            Ok(split_container_item(&mut model, bag, slot, count))
        },
    )?;
    lua.register_function("DeleteCursorItem", |lua, ()| {
        let mut model = lua.app_data_mut::<Model>().expect("model app_data");
        delete_cursor_item(&mut model);
        Ok(())
    })?;

    doll::install(lua)?;
    bag_verbs::install(lua)?;
    bar::install(lua)?;
    pet::install(lua)?;
    money::install(lua)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{CursorAction, CursorItem, CursorPayload, CursorSpell};
    use crate::script::UiScript;

    /// A one-item, one-slot backpack (mirrors `container::tests::backpack` closely enough for
    /// these payload-shape tests, which don't need its in-flight slot).
    fn one_item_backpack() -> crate::script::container::ContainerState {
        let mut slots = std::collections::HashMap::new();
        slots.insert(
            1,
            crate::script::container::ContainerSlot {
                duration_ms: None,
                petition: None,
                already_bound: false,
                bar_placeable: true,
                durability: None,
                texture: Some("Interface\\Icons\\INV_Misc_Food_16".into()),
                count: 5,
                quality: Some(3),
                item_id: 117,
                link: Some("|cffffffff|Hitem:117|h[Tough Jerky]|h|r".into()),
                locked: false,
                equip_slots: Vec::new(),
                cooldown: None,
                readable: false,
                creator: None,
                flags: 0,
                enchants: Vec::new(),
            },
        );
        crate::script::container::ContainerState {
            name: Some("Backpack".into()),
            num_slots: 16,
            slots,
        }
    }

    /// **The sell clear is not `ClearCursor`** — decision 1914, and the difference is the one a
    /// player sees.
    ///
    /// `0x494b60 SetCursorItem(0, …)` drops the payload and fires `CURSOR_UPDATE`, but its mode-1
    /// arm skips `0x495420` with `param1 = 0`, so it never un-locks the source slot: the item is
    /// off the cursor and the bag slot stays **greyed** until the server's inventory update takes
    /// it away. `ClearCursor` un-locks. Firing `ITEM_LOCK_CHANGED` here would un-grey a slot whose
    /// item is still sitting in it awaiting the server, which is the visible wrong.
    ///
    /// The second half is the blast radius: the fork answers for **mode 1 only**, so asking a
    /// spell or a vendor row must not drop it.
    #[test]
    fn the_sell_clear_keeps_the_source_slot_locked() {
        let mut s = UiScript::new().unwrap();
        s.set_container(0, Some(one_item_backpack()));

        // Pick the item up, then drain whatever the pickup queued so the assert below sees only
        // what the SELL clear itself fires.
        s.eval::<()>("PickupContainerItem(0, 1)").unwrap();
        assert!(s.cursor_item().is_some(), "the pickup did not take");
        s.model_mut().pending_events.clear();

        let sold = s.take_cursor_item_for_sale().expect("mode 1 is a sale");
        assert_eq!((sold.bag, sold.slot), (0, 1));
        assert!(
            s.cursor_payload().is_none(),
            "the cursor kept the sold item"
        );

        let fired: Vec<String> = s
            .model_ref()
            .pending_events
            .iter()
            .map(|(e, _)| e.clone())
            .collect();
        assert!(
            fired.iter().any(|e| e == "CURSOR_UPDATE"),
            "the sell clear must fire CURSOR_UPDATE — got {fired:?}"
        );
        assert!(
            !fired.iter().any(|e| e == "ITEM_LOCK_CHANGED"),
            "the sell clear must NOT un-lock the source slot (that is `ClearCursor`'s job, not \
             `SetCursorItem(0, …)`'s) — got {fired:?}"
        );

        // Mode 1 only: every other payload is left exactly where it was.
        let mut s = UiScript::new().unwrap();
        s.set_cursor_for_test(CursorPayload::Spell(CursorSpell {
            book_slot: 1,
            book_type: "spell".into(),
            spell_id: 133,
            texture: None,
            passive: false,
        }));
        assert!(
            s.take_cursor_item_for_sale().is_none(),
            "a spell is not a sale"
        );
        assert!(
            matches!(s.cursor_payload(), Some(CursorPayload::Spell(_))),
            "asking for a sale dropped a non-item payload"
        );
    }

    #[test]
    fn get_cursor_info_and_has_checks_per_arm() {
        let mut s = UiScript::new().unwrap();

        // Empty: all nils, both Has* false.
        assert!(s
            .eval::<bool>("local k = GetCursorInfo() return k == nil")
            .unwrap());
        assert!(!s.eval::<bool>("return CursorHasItem()").unwrap());
        assert!(!s.eval::<bool>("return CursorHasSpell()").unwrap());

        // Item arm.
        s.set_cursor_for_test(CursorPayload::Item(CursorItem {
            bar_placeable: true,
            bag: 0,
            slot: 1,
            item_id: 117,
            texture: None,
            link: Some("|Hitem:117|h[Tough Jerky]|h".into()),
            count: None,
            quality: Some(3),
            equip_slots: Vec::new(),
        }));
        let (kind, id, link) = s
            .eval::<(String, i64, String)>("local k, id, link = GetCursorInfo() return k, id, link")
            .unwrap();
        assert_eq!(
            (kind.as_str(), id, link.as_str()),
            ("item", 117, "|Hitem:117|h[Tough Jerky]|h")
        );
        assert!(s.eval::<bool>("return CursorHasItem()").unwrap());
        assert!(!s.eval::<bool>("return CursorHasSpell()").unwrap());

        // Spell arm — the Era 4-tuple shape.
        s.set_cursor_for_test(CursorPayload::Spell(CursorSpell {
            passive: false,
            book_slot: 3,
            book_type: "spell".into(),
            spell_id: 133,
            texture: None,
        }));
        let (kind, slot, book, spell_id) = s
            .eval::<(String, i64, String, i64)>(
                "local k, slot, book, id = GetCursorInfo() return k, slot, book, id",
            )
            .unwrap();
        assert_eq!(
            (kind.as_str(), slot, book.as_str(), spell_id),
            ("spell", 3, "spell", 133)
        );
        assert!(!s.eval::<bool>("return CursorHasItem()").unwrap());
        assert!(s.eval::<bool>("return CursorHasSpell()").unwrap());

        // Action arm.
        s.set_cursor_for_test(CursorPayload::Action(CursorAction {
            src_slot: 12,
            kind: 0,
            action: 133,
            texture: None,
        }));
        let (kind, slot) = s
            .eval::<(String, i64)>("local k, slot = GetCursorInfo() return k, slot")
            .unwrap();
        assert_eq!((kind.as_str(), slot), ("action", 12));
        assert!(!s.eval::<bool>("return CursorHasItem()").unwrap());
        assert!(!s.eval::<bool>("return CursorHasSpell()").unwrap());
    }

    #[test]
    fn clear_cursor_widens_to_any_arm() {
        let mut s = UiScript::new().unwrap();
        s.set_cursor_for_test(CursorPayload::Spell(CursorSpell {
            passive: false,
            book_slot: 1,
            book_type: "spell".into(),
            spell_id: 1,
            texture: None,
        }));
        assert!(s.cursor_payload().is_some());
        s.run("ClearCursor()").unwrap();
        assert!(s.cursor_payload().is_none());
    }

    /// The world-drop pick routing (decisions 0571 + 0574 — wow-re §11, amended by 0843): a
    /// spell/action payload clears silently on BOTH empty-world legs — terrain included, the
    /// 0843 divergence (the reference keeps it on terrain; the director wants the left click to
    /// dismiss) — while an item keeps its byte-faithful popup flow.
    #[test]
    fn world_drop_terrain_and_nothing_both_clear_a_spell_payload() {
        use super::{world_drop_click, WorldPick};
        let mut s = UiScript::new().unwrap();
        for pick in [WorldPick::Terrain, WorldPick::Nothing] {
            s.set_cursor_for_test(CursorPayload::Spell(CursorSpell {
                passive: false,
                book_slot: 3,
                book_type: "spell".into(),
                spell_id: 133,
                texture: None,
            }));
            s.model_mut().world_pick = pick;
            assert!(
                world_drop_click(&mut s.model_mut()),
                "{pick:?}: the dismiss consumes the click"
            );
            assert!(
                s.cursor_payload().is_none(),
                "{pick:?}: the spell clears silently"
            );
        }

        // An item drops (the popup path) on BOTH empty-world legs, never over an object.
        let item = CursorPayload::Item(CursorItem {
            bar_placeable: true,
            bag: 0,
            slot: 1,
            item_id: 117,
            texture: None,
            link: None,
            count: None,
            quality: None,
            equip_slots: Vec::new(),
        });
        s.set_cursor_for_test(item.clone());
        s.model_mut().world_pick = WorldPick::Terrain;
        assert!(
            world_drop_click(&mut s.model_mut()),
            "terrain: the item pops the popup"
        );
        assert!(s.cursor_item().is_some(), "and stays held");
        s.model_mut().world_pick = WorldPick::Object;
        assert!(
            !world_drop_click(&mut s.model_mut()),
            "object: nothing drops at all"
        );
        assert!(s.cursor_item().is_some());
    }

    #[test]
    fn delete_cursor_item_queues_destroy_and_clears() {
        let mut s = UiScript::new().unwrap();
        s.set_container(0, Some(one_item_backpack()));
        s.run("PickupContainerItem(0, 1)").unwrap();
        assert!(s.cursor_item().is_some());

        s.run("DeleteCursorItem()").unwrap();
        assert!(s.cursor_item().is_none(), "the payload clears");
        assert_eq!(s.take_container_destroys(), vec![(0, 1, 0)]);
        assert!(s.take_container_destroys().is_empty(), "drained");
    }

    #[test]
    fn delete_cursor_item_is_a_no_op_off_the_item_arm() {
        let mut s = UiScript::new().unwrap();
        s.set_cursor_for_test(CursorPayload::Action(CursorAction {
            src_slot: 1,
            kind: 0,
            action: 1,
            texture: None,
        }));
        s.run("DeleteCursorItem()").unwrap();
        assert!(
            s.cursor_payload().is_some(),
            "non-item payload is untouched"
        );
        assert!(s.take_container_destroys().is_empty());
    }

    #[test]
    fn split_of_the_whole_stack_degrades_to_a_plain_pickup() {
        let mut s = UiScript::new().unwrap();
        s.set_container(0, Some(one_item_backpack())); // slot 1: a 5-stack
        assert!(s
            .eval::<bool>("return SplitContainerItem(0, 1, 5)")
            .unwrap());
        let held = s.cursor_item().expect("picked up");
        assert_eq!(held.count, None, "count>=stack ⇒ whole-stack pickup");
    }
}
