//! Rust-owned application data shared by the browser VM and its callbacks.
//!
//! The production `UiScript` keeps its frame model in `mlua` app data. This adapter gives the
//! browser backend the same typed, dynamically borrowed ownership pattern without changing
//! `rilua` itself or making Lua scripts able to access the host data.

use std::{
    any::{Any, TypeId},
    cell::{Ref, RefCell, RefMut},
    collections::HashMap,
    rc::Rc,
};

use rilua::{vm::state::LuaState, Lua, LuaResult, Val};

use crate::create_host_callback;

#[derive(Clone, Default)]
pub struct HostAppData(Rc<RefCell<HashMap<TypeId, Box<dyn Any>>>>);

impl HostAppData {
    /// Install or replace one Rust-owned type. An active borrow must be released first.
    pub fn set<T: Any>(&self, value: T) -> Option<T> {
        self.0
            .borrow_mut()
            .insert(TypeId::of::<T>(), Box::new(value))
            .and_then(|old| old.downcast::<T>().ok())
            .map(|old| *old)
    }

    /// A dynamic borrow of host state, absent if the type is missing or mutably borrowed.
    pub fn get<T: Any>(&self) -> Option<Ref<'_, T>> {
        Ref::filter_map(self.0.try_borrow().ok()?, |map| {
            map.get(&TypeId::of::<T>())?.downcast_ref::<T>()
        })
        .ok()
    }

    /// A mutable dynamic borrow, absent if another borrow is active.
    pub fn get_mut<T: Any>(&self) -> Option<RefMut<'_, T>> {
        RefMut::filter_map(self.0.try_borrow_mut().ok()?, |map| {
            map.get_mut(&TypeId::of::<T>())?.downcast_mut::<T>()
        })
        .ok()
    }

    /// Remove and return one Rust-owned type. An active borrow must be released first.
    pub fn remove<T: Any>(&self) -> Option<T> {
        self.0
            .try_borrow_mut()
            .ok()?
            .remove(&TypeId::of::<T>())?
            .downcast::<T>()
            .ok()
            .map(|value| *value)
    }

    /// Make a Lua function whose host callback sees this same app-data store.
    pub fn create_callback(
        &self,
        lua: &mut Lua,
        name: &str,
        callback: impl Fn(&mut LuaState, &HostAppData) -> LuaResult<u32> + 'static,
    ) -> Val {
        let data = self.clone();
        create_host_callback(lua, name, move |state| callback(state, &data))
    }
}

#[cfg(test)]
mod tests {
    use rilua::{Lua, LuaApiMut, Val};

    use super::HostAppData;

    #[test]
    fn typed_model_is_shared_with_lua_callback_and_borrow_is_released() {
        let data = HostAppData::default();
        data.set(7_u32);
        data.set(String::from("frame"));
        let mut lua = Lua::new().expect("Lua VM");
        let callback = data.create_callback(&mut lua, "increment", |state, data| {
            let mut count = data.get_mut::<u32>().expect("unborrowed counter");
            *count += 1;
            let value = *count;
            drop(count);
            state.push(Val::Num(f64::from(value)));
            Ok(1)
        });
        lua.set_global_val("increment", callback)
            .expect("publish callback");

        lua.exec("first = increment(); second = increment()")
            .expect("run callbacks");
        assert_eq!(lua.global::<f64>("first").expect("first"), 8.0);
        assert_eq!(lua.global::<f64>("second").expect("second"), 9.0);
        assert_eq!(*data.get::<u32>().expect("counter"), 9);
        assert_eq!(data.get::<String>().expect("other type").as_str(), "frame");
    }

    #[test]
    fn overlapping_mutable_borrow_is_rejected() {
        let data = HostAppData::default();
        data.set(1_u32);
        let guard = data.get::<u32>().expect("read borrow");
        assert!(data.get_mut::<u32>().is_none());
        drop(guard);
        assert!(data.get_mut::<u32>().is_some());
    }
}
