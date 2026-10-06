//! The duel **Era API surface** (decision 0633) — four globals, no state.
//!
//! Duels are the smallest possible shape of the [`super::party`] seam: everything the UI needs to
//! *read* arrives as event arguments (the challenger's name on `DUEL_REQUESTED`), so there is no
//! snapshot to push — only the outbound half. Each call queues a [`DuelRequest`] the app drains
//! ([`UiScript::take_duel_requests`]) and turns into its send, keeping the engine free of ECS/net
//! reach (decision 0068 §3).
//!
//! The four are exactly the reference's own duel bindings, registered adjacent in its Lua API
//! table (`0x849fc8`..`0x849ff0`) and each a one-liner over the same TU: `AcceptDuel` `0x4d4ce0`
//! → `0x4d4830`, `CancelDuel` `0x4d4cf0` → `0x4d48b0`, `StartDuelUnit` `0x4d4c40` (unit token →
//! guid, gated on typemask `0x10` = player), `StartDuel` `0x4d4c90` (name → guid). The two
//! `StartDuel*` calls do **not** send a duel opcode — they cast the duel spell at the guid; the
//! app owns that resolution.

use benilla_lua::Vm;

use super::Model;

pub use crate::script_data::DuelRequest;

impl super::UiScript {
    /// Drain the duel intents queued since the last call.
    pub fn take_duel_requests(&mut self) -> Vec<DuelRequest> {
        std::mem::take(&mut self.model_mut().duel_requests)
    }

    /// Queue an intent from the app side — the slash commands. In the reference these ARE Lua
    /// (`SlashCmdList["DUEL"]` calls `StartDuel`, `SlashCmdList["DUEL_CANCEL"]` calls
    /// `CancelDuel`); benilla parses slash lines in Rust, so the same intents enter the same
    /// queue here rather than through the globals.
    pub fn queue_duel_request(&mut self, request: DuelRequest) {
        self.model_mut().duel_requests.push(request);
    }
}

/// Register the duel globals (the same style/place [`super::party`] registers its actions).
pub(super) fn install(lua: &mut Vm) -> benilla_lua::Result<()> {
    // AcceptDuel() — the DUEL_REQUESTED popup's Accept.
    lua.register_function("AcceptDuel", |lua, ()| {
        let mut model = lua.app_data_mut::<Model>().expect("model app_data");
        model.duel_requests.push(DuelRequest::Accept);
        Ok(())
    })?;

    // CancelDuel() — the popup's Decline, and /forfeit //concede //yield once under way.
    lua.register_function("CancelDuel", |lua, ()| {
        let mut model = lua.app_data_mut::<Model>().expect("model app_data");
        model.duel_requests.push(DuelRequest::Cancel);
        Ok(())
    })?;

    // StartDuel(name) — /duel <name>.
    lua.register_function("StartDuel", |lua, name: String| {
        let mut model = lua.app_data_mut::<Model>().expect("model app_data");
        model.duel_requests.push(DuelRequest::StartByName(name));
        Ok(())
    })?;

    // StartDuelUnit(unit) — the unit popup's Duel row.
    lua.register_function("StartDuelUnit", |lua, unit: String| {
        let mut model = lua.app_data_mut::<Model>().expect("model app_data");
        model.duel_requests.push(DuelRequest::StartByUnit(unit));
        Ok(())
    })?;

    Ok(())
}
