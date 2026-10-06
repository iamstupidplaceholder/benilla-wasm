//! Capturing host callbacks for the browser Lua adapter.
//!
//! `rilua::RustFn` is a function pointer, while the production FrameScript host registers
//! closures that retain widget ids and other Rust-side state. Keep the callback in Lua-owned
//! userdata and let a single static trampoline dispatch it. The userdata is an upvalue, so
//! rilua's collector keeps it alive for exactly as long as the Lua function is reachable.

use std::rc::Rc;

use rilua::error::runtime_error;
use rilua::vm::{
    closure::{Closure, RustClosure},
    state::LuaState,
};
use rilua::{Lua, LuaApiMut, LuaResult, Val};

type HostCallback = Rc<dyn Fn(&mut LuaState) -> LuaResult<u32>>;

/// Create a Lua-callable Rust closure that may capture host state.
///
/// This is the low-level primitive for the future `UiScript` adapter; argument and result
/// conversions still belong to that adapter, not to the VM's dispatch machinery.
pub fn create_host_callback(
    lua: &mut Lua,
    name: &str,
    callback: impl Fn(&mut LuaState) -> LuaResult<u32> + 'static,
) -> Val {
    let userdata = lua.create_userdata(Rc::new(callback) as HostCallback);
    let mut closure = RustClosure::new(dispatch_host_callback, name);
    closure.upvalues.push(Val::Userdata(userdata.gc_ref()));
    let function = lua.state_mut().gc.alloc_closure(Closure::Rust(closure));
    Val::Function(function)
}

/// Publish a capturing callback as a global Lua function.
pub fn register_host_callback(
    lua: &mut Lua,
    name: &str,
    callback: impl Fn(&mut LuaState) -> LuaResult<u32> + 'static,
) -> LuaResult<()> {
    let function = create_host_callback(lua, name, callback);
    lua.set_global_val(name, function)
}

fn dispatch_host_callback(state: &mut LuaState) -> LuaResult<u32> {
    let caller = state.call_stack[state.ci].func;
    let Val::Function(function) = state.stack_get(caller) else {
        return Err(runtime_error("host callback has no active function"));
    };
    let userdata = match state.gc.closures.get(function) {
        Some(Closure::Rust(closure)) => match closure.upvalues.first() {
            Some(Val::Userdata(userdata)) => *userdata,
            _ => return Err(runtime_error("host callback has no userdata upvalue")),
        },
        _ => return Err(runtime_error("host callback closure is missing")),
    };
    // Clone the Rc before calling back: a host function may re-enter Lua or mutate its GC.
    let callback = state
        .gc
        .userdata
        .get(userdata)
        .and_then(|value| value.downcast_ref::<HostCallback>())
        .cloned()
        .ok_or_else(|| runtime_error("host callback userdata is missing"))?;
    callback(state)
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, rc::Rc};

    use rilua::{Lua, LuaApiMut, Val};

    use super::register_host_callback;

    #[test]
    fn captured_state_survives_lua_calls_and_collection() {
        let mut lua = Lua::new().expect("Lua VM");
        let calls = Rc::new(Cell::new(0_u32));
        let captured = Rc::clone(&calls);
        register_host_callback(&mut lua, "next_number", move |state| {
            let next = captured.get() + 1;
            captured.set(next);
            state.push(Val::Num(f64::from(next)));
            Ok(1)
        })
        .expect("register callback");

        lua.exec("first = next_number(); collectgarbage(); second = next_number()")
            .expect("call captured callback");
        assert_eq!(lua.global::<f64>("first").expect("first"), 1.0);
        assert_eq!(lua.global::<f64>("second").expect("second"), 2.0);
        assert_eq!(calls.get(), 2);
    }
}
