//! Operations available both on an owned VM and inside an active host callback.
//!
//! Production FrameScript code is entered through both paths. Keeping one
//! explicit context contract prevents the UI layer from depending on backend
//! table methods or from reaching back into an already borrowed outer VM.

use std::any::Any;

use crate::debug::DebugFrame;
use crate::{
    AppDataRef, AppDataRefMut, CallbackContext, FromLua, FromLuaMulti, Function, IntoLua,
    IntoLuaMulti, MultiValue, Result, Table, Value, Vm,
};

/// Explicit access to the currently active Lua state.
///
/// The primitive methods traffic only in Benilla-owned values. Typed helpers
/// are defaults over those primitives, so conversions behave identically in
/// the native and browser backends.
pub trait LuaContext {
    fn app_data_ref<T: Any>(&self) -> Option<AppDataRef<'_, T>>;
    fn app_data_mut<T: Any>(&self) -> Option<AppDataRefMut<'_, T>>;
    fn remove_app_data<T: Any>(&mut self) -> Option<T>;

    /// Live VM memory in bytes, for the 5.0 `gcinfo` pair.
    fn used_memory_bytes(&self) -> usize;

    /// Run a full collection cycle, for the 5.0 `collectgarbage` verb.
    fn collect_garbage(&mut self) -> Result<()>;

    /// The current call stack, innermost frame first, for `debugstack`.
    ///
    /// Level 0 is the running function itself. An idle VM answers empty.
    fn debug_frames(&self) -> Vec<DebugFrame>;

    fn create_table(&mut self) -> Result<Table>;

    /// Compile and execute a named text chunk in the active VM.
    fn exec_named_text(&mut self, source: &[u8], name: &str) -> Result<()>;

    /// Compile a named text chunk without executing it.
    fn load_function(&mut self, source: &[u8], name: &str) -> Result<Function>;

    fn global_value(&mut self, name: &str) -> Result<Value>;
    fn set_global_value(&mut self, name: &str, value: Value) -> Result<()>;

    fn registry_value(&mut self, name: &str) -> Result<Value>;
    fn set_registry_value(&mut self, name: &str, value: Value) -> Result<()>;
    fn remove_registry_value(&mut self, name: &str) -> Result<()>;

    fn table_get_value(&mut self, table: &Table, key: Value) -> Result<Value>;
    fn table_set_value(&mut self, table: &Table, key: Value, value: Value) -> Result<()>;
    fn table_get_raw_value(&mut self, table: &Table, key: Value) -> Result<Value>;
    fn table_set_raw_value(&mut self, table: &Table, key: Value, value: Value) -> Result<()>;
    fn table_set_metatable(&mut self, table: &Table, metatable: Option<&Table>) -> Result<()>;
    fn table_metatable(&mut self, table: &Table) -> Result<Option<Table>>;
    fn table_raw_len(&mut self, table: &Table) -> Result<usize>;
    fn table_entries(&mut self, table: &Table) -> Result<Vec<(Value, Value)>>;
    fn table_identity(&mut self, table: &Table) -> Result<u64>;
    fn function_source_location(&mut self, function: &Function) -> Option<(String, usize)>;

    fn call_function_values(
        &mut self,
        function: &Function,
        arguments: MultiValue,
    ) -> Result<MultiValue>;

    fn get_global<T: FromLua>(&mut self, name: &str) -> Result<T> {
        T::from_lua(self.global_value(name)?)
    }

    fn put_global<T: IntoLua>(&mut self, name: &str, value: T) -> Result<()> {
        self.set_global_value(name, value.into_lua()?)
    }

    fn get_registry<T: FromLua>(&mut self, name: &str) -> Result<T> {
        T::from_lua(self.registry_value(name)?)
    }

    fn put_registry<T: IntoLua>(&mut self, name: &str, value: T) -> Result<()> {
        self.set_registry_value(name, value.into_lua()?)
    }

    fn table_get<K: IntoLua, V: FromLua>(&mut self, table: &Table, key: K) -> Result<V> {
        V::from_lua(self.table_get_value(table, key.into_lua()?)?)
    }

    fn table_set<K: IntoLua, V: IntoLua>(&mut self, table: &Table, key: K, value: V) -> Result<()> {
        self.table_set_value(table, key.into_lua()?, value.into_lua()?)
    }

    fn table_get_raw<K: IntoLua, V: FromLua>(&mut self, table: &Table, key: K) -> Result<V> {
        V::from_lua(self.table_get_raw_value(table, key.into_lua()?)?)
    }

    fn table_set_raw<K: IntoLua, V: IntoLua>(
        &mut self,
        table: &Table,
        key: K,
        value: V,
    ) -> Result<()> {
        self.table_set_raw_value(table, key.into_lua()?, value.into_lua()?)
    }

    fn call_function<A: IntoLuaMulti, R: FromLuaMulti>(
        &mut self,
        function: &Function,
        arguments: A,
    ) -> Result<R> {
        R::from_lua_multi(self.call_function_values(function, arguments.into_lua_multi()?)?)
    }

    fn call_method<A: IntoLuaMulti, R: FromLuaMulti>(
        &mut self,
        table: &Table,
        name: &str,
        arguments: A,
    ) -> Result<R> {
        let function: Function = self.table_get(table, name)?;
        let mut arguments = arguments.into_lua_multi()?;
        arguments.insert(0, Value::Table(table.clone()));
        R::from_lua_multi(self.call_function_values(&function, arguments)?)
    }
}

macro_rules! impl_lua_context {
    ($context:ty) => {
        impl LuaContext for $context {
            fn app_data_ref<T: Any>(&self) -> Option<AppDataRef<'_, T>> {
                <$context>::app_data_ref(self)
            }

            fn app_data_mut<T: Any>(&self) -> Option<AppDataRefMut<'_, T>> {
                <$context>::app_data_mut(self)
            }

            fn remove_app_data<T: Any>(&mut self) -> Option<T> {
                <$context>::remove_app_data(self)
            }

            fn used_memory_bytes(&self) -> usize {
                <$context>::used_memory_bytes(self)
            }

            fn collect_garbage(&mut self) -> Result<()> {
                <$context>::collect_garbage(self)
            }

            fn debug_frames(&self) -> Vec<DebugFrame> {
                <$context>::debug_frames(self)
            }

            fn create_table(&mut self) -> Result<Table> {
                <$context>::create_table(self)
            }

            fn exec_named_text(&mut self, source: &[u8], name: &str) -> Result<()> {
                <$context>::exec_named_text(self, source, name)
            }

            fn load_function(&mut self, source: &[u8], name: &str) -> Result<Function> {
                <$context>::load_function(self, source, name)
            }

            fn global_value(&mut self, name: &str) -> Result<Value> {
                <$context>::global(self, name)
            }

            fn set_global_value(&mut self, name: &str, value: Value) -> Result<()> {
                <$context>::set_global(self, name, value)
            }

            fn registry_value(&mut self, name: &str) -> Result<Value> {
                <$context>::named_registry_value(self, name)
            }

            fn set_registry_value(&mut self, name: &str, value: Value) -> Result<()> {
                <$context>::set_named_registry_value(self, name, value)
            }

            fn remove_registry_value(&mut self, name: &str) -> Result<()> {
                <$context>::remove_named_registry_value(self, name)
            }

            fn table_get_value(&mut self, table: &Table, key: Value) -> Result<Value> {
                <$context>::table_get(self, table, key)
            }

            fn table_set_value(&mut self, table: &Table, key: Value, value: Value) -> Result<()> {
                <$context>::table_set(self, table, key, value)
            }

            fn table_get_raw_value(&mut self, table: &Table, key: Value) -> Result<Value> {
                <$context>::table_get_raw(self, table, key)
            }

            fn table_set_raw_value(
                &mut self,
                table: &Table,
                key: Value,
                value: Value,
            ) -> Result<()> {
                <$context>::table_set_raw(self, table, key, value)
            }

            fn table_set_metatable(
                &mut self,
                table: &Table,
                metatable: Option<&Table>,
            ) -> Result<()> {
                <$context>::table_set_metatable(self, table, metatable)
            }

            fn table_metatable(&mut self, table: &Table) -> Result<Option<Table>> {
                <$context>::table_metatable(self, table)
            }

            fn table_raw_len(&mut self, table: &Table) -> Result<usize> {
                <$context>::table_raw_len(self, table)
            }

            fn table_entries(&mut self, table: &Table) -> Result<Vec<(Value, Value)>> {
                <$context>::table_entries(self, table)
            }

            fn table_identity(&mut self, table: &Table) -> Result<u64> {
                <$context>::table_identity(self, table)
            }

            fn function_source_location(&mut self, function: &Function) -> Option<(String, usize)> {
                <$context>::function_source_location(self, function)
            }

            fn call_function_values(
                &mut self,
                function: &Function,
                arguments: MultiValue,
            ) -> Result<MultiValue> {
                <$context>::call_function(self, function, arguments)
            }
        }
    };
}

impl_lua_context!(Vm);
impl_lua_context!(CallbackContext<'_>);
