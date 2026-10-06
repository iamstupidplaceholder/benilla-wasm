//! Named, Lua-GC-rooted values for the browser UI host.
//!
//! The production FrameScript host keeps method tables and handlers in Lua's registry rather
//! than retaining thousands of Rust handles. Use rilua's own registry table for that same
//! lifetime rule; the private key prefix prevents collisions with rilua's standard libraries.

use rilua::vm::state::LuaState;
use rilua::{Lua, LuaApiMut, LuaResult, Table, Val};

const PREFIX: &str = "\0benilla_ui:";

fn registry_key(state: &mut LuaState, name: &str) -> Val {
    let mut key = String::with_capacity(PREFIX.len() + name.len());
    key.push_str(PREFIX);
    key.push_str(name);
    Val::Str(state.gc.intern_string(key.as_bytes()))
}

/// Store a Lua value under a host-owned name. `Nil` removes the value.
pub fn set_named_registry_value(lua: &mut Lua, name: &str, value: Val) -> LuaResult<()> {
    set_named_registry_value_in_state(lua.state_mut(), name, value)
}

pub(crate) fn set_named_registry_value_in_state(
    state: &mut LuaState,
    name: &str,
    value: Val,
) -> LuaResult<()> {
    let table = Table::from_gc_ref(state.registry);
    let key = registry_key(state, name);
    table.raw_set(state, key, value)
}

/// Read a host-owned Lua value, returning `Nil` for an absent name.
pub fn named_registry_value(lua: &mut Lua, name: &str) -> LuaResult<Val> {
    named_registry_value_in_state(lua.state_mut(), name)
}

pub(crate) fn named_registry_value_in_state(state: &mut LuaState, name: &str) -> LuaResult<Val> {
    let table = Table::from_gc_ref(state.registry);
    let key = registry_key(state, name);
    table.raw_get(state, key)
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, rc::Rc};

    use rilua::{Lua, LuaApiMut, Val};

    use crate::create_host_callback;

    use super::{named_registry_value, set_named_registry_value};

    #[test]
    fn registry_keeps_handler_alive_through_gc() {
        let mut lua = Lua::new().expect("Lua VM");
        let count = Rc::new(Cell::new(0_u32));
        let captured = Rc::clone(&count);
        let handler = create_host_callback(&mut lua, "on_click", move |state| {
            captured.set(captured.get() + 1);
            state.push(Val::Num(f64::from(captured.get())));
            Ok(1)
        });
        set_named_registry_value(&mut lua, "button_click", handler).expect("store handler");
        lua.exec("collectgarbage()")
            .expect("collect unreachable values");

        let handler = named_registry_value(&mut lua, "button_click").expect("stored handler");
        lua.set_global_val("click", handler)
            .expect("publish handler");
        lua.exec("result = click()")
            .expect("invoke retained handler");
        assert_eq!(lua.global::<f64>("result").expect("result"), 1.0);
        assert_eq!(count.get(), 1);

        set_named_registry_value(&mut lua, "button_click", Val::Nil).expect("remove handler");
        assert_eq!(
            named_registry_value(&mut lua, "button_click").expect("missing handler"),
            Val::Nil
        );
    }
}
