use std::{
    any::Any,
    cell::Cell,
    ffi::c_void,
    fmt,
    ops::Deref,
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
};

use mlua_native::{ChunkMode, Lua, LuaOptions, MultiValue as NativeMultiValue, StdLib};

use crate::{
    AppDataRef, AppDataRefMut, Error, FromLua, FromLuaMulti, IntoLua, IntoLuaMulti, LuaString,
    MultiValue, Result, Value,
};

static NEXT_VM_ID: AtomicU64 = AtomicU64::new(1);

pub struct NativeVm {
    lua: Lua,
    id: u64,
    instruction_count: Rc<Cell<u64>>,
}

/// Identity left in the native Lua app-data store so an untouched mlua callback can temporarily
/// enter the same neutral context during the incremental UI migration.
#[derive(Clone, Copy)]
struct NativeVmIdentity(u64);

#[derive(Clone)]
pub struct NativeTable {
    table: mlua_native::Table,
    vm_id: u64,
}

#[derive(Clone)]
pub struct NativeFunction {
    function: mlua_native::Function,
    vm_id: u64,
}

pub struct NativeCallbackContext<'lua> {
    lua: &'lua Lua,
    vm_id: u64,
}

impl fmt::Debug for NativeTable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Table").field("vm", &self.vm_id).finish()
    }
}

impl fmt::Debug for NativeFunction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Function").field("vm", &self.vm_id).finish()
    }
}

/// Function identity, for the host's default-handler skip: two handles name
/// the same closure in the same VM. This is mlua's own reference equality,
/// scoped to one VM's identity.
impl PartialEq for NativeFunction {
    fn eq(&self, other: &Self) -> bool {
        self.vm_id == other.vm_id && self.function == other.function
    }
}

impl Deref for NativeVm {
    type Target = Lua;

    fn deref(&self) -> &Self::Target {
        self.native_lua()
    }
}

impl NativeVm {
    pub fn new() -> Result<Self> {
        let libraries = StdLib::STRING | StdLib::TABLE | StdLib::MATH;
        let lua = Lua::new_with(libraries, LuaOptions::default()).map_err(native_error)?;
        let id = NEXT_VM_ID.fetch_add(1, Ordering::Relaxed);
        lua.set_app_data(NativeVmIdentity(id));
        Ok(Self {
            lua,
            id,
            instruction_count: Rc::new(Cell::new(0)),
        })
    }

    /// Adopt an existing native Lua state during the incremental production-host migration.
    ///
    /// Unported modules continue to use the same state through [`Self::native_lua`]; ported
    /// modules use the neutral VM API. Both paths therefore see one registry and one app-data
    /// store rather than mirrored state.
    pub fn from_native_lua(lua: Lua) -> Self {
        let id = NEXT_VM_ID.fetch_add(1, Ordering::Relaxed);
        lua.set_app_data(NativeVmIdentity(id));
        Self {
            lua,
            id,
            instruction_count: Rc::new(Cell::new(0)),
        }
    }

    /// Transitional access for production modules not yet moved to the neutral API.
    pub fn native_lua(&self) -> &Lua {
        &self.lua
    }

    /// Convert a value supplied by an as-yet-unported native installer into the neutral model.
    pub fn import_native_value(&self, value: mlua_native::Value) -> Result<Value> {
        self.from_native_value(value)
    }

    /// Convert a neutral value for an as-yet-unported native installer.
    pub fn export_native_value(&self, value: Value) -> Result<mlua_native::Value> {
        self.to_native_value(value)
    }

    pub fn exec_named_text(&mut self, source: &[u8], name: &str) -> Result<()> {
        self.lua
            .load(source)
            .set_name(name)
            .set_mode(ChunkMode::Text)
            .exec()
            .map_err(native_error)
    }

    pub fn load_function(&mut self, source: &[u8], name: &str) -> Result<NativeFunction> {
        let function = self
            .lua
            .load(source)
            .set_name(name)
            .set_mode(ChunkMode::Text)
            .into_function()
            .map_err(native_error)?;
        Ok(self.wrap_function(function))
    }

    pub fn create_host_function(
        &mut self,
        _name: &str,
        callback: impl Fn(&mut NativeCallbackContext<'_>, MultiValue) -> Result<MultiValue> + 'static,
    ) -> Result<NativeFunction> {
        let vm_id = self.id;
        let function = self
            .lua
            .create_function(move |lua, arguments: NativeMultiValue| {
                let arguments = arguments
                    .into_vec()
                    .into_iter()
                    .map(|value| native_value_from_backend(lua, vm_id, value))
                    .collect::<Result<Vec<_>>>()
                    .map_err(|error| mlua_native::Error::runtime(error.to_string()))?;
                let mut context = NativeCallbackContext { lua, vm_id };
                let values = callback(&mut context, arguments)
                    .map_err(|error| mlua_native::Error::runtime(error.to_string()))?;
                let values = values
                    .into_iter()
                    .map(|value| native_value_to_backend(lua, vm_id, value))
                    .collect::<Result<Vec<_>>>()
                    .map_err(|error| mlua_native::Error::runtime(error.to_string()))?;
                Ok(NativeMultiValue::from_vec(values))
            })
            .map_err(native_error)?;
        Ok(self.wrap_function(function))
    }

    pub fn create_function<A, R>(
        &mut self,
        name: &str,
        callback: impl Fn(&mut NativeCallbackContext<'_>, A) -> Result<R> + 'static,
    ) -> Result<NativeFunction>
    where
        A: FromLuaMulti + 'static,
        R: IntoLuaMulti + 'static,
    {
        self.create_host_function(name, move |context, arguments| {
            callback(context, A::from_lua_multi(arguments)?)?.into_lua_multi()
        })
    }

    pub fn register_function<A, R>(
        &mut self,
        name: &str,
        callback: impl Fn(&mut NativeCallbackContext<'_>, A) -> Result<R> + 'static,
    ) -> Result<()>
    where
        A: FromLuaMulti + 'static,
        R: IntoLuaMulti + 'static,
    {
        let function = self.create_function(name, callback)?;
        self.set_global_function(name, &function)
    }

    pub fn create_table(&mut self) -> Result<NativeTable> {
        let table = self.lua.create_table().map_err(native_error)?;
        Ok(self.wrap_table(table))
    }

    pub fn set_app_data<T: Any>(&mut self, value: T) -> Option<T> {
        self.lua.set_app_data(value)
    }

    pub fn app_data_ref<T: Any>(&self) -> Option<AppDataRef<'_, T>> {
        self.lua.app_data_ref::<T>()
    }

    pub fn app_data_mut<T: Any>(&self) -> Option<AppDataRefMut<'_, T>> {
        self.lua.app_data_mut::<T>()
    }

    pub fn remove_app_data<T: Any>(&mut self) -> Option<T> {
        self.lua.remove_app_data::<T>()
    }

    /// Live VM memory in bytes; usable inside callbacks through [`crate::LuaContext`].
    pub fn used_memory_bytes(&self) -> usize {
        self.lua.used_memory()
    }

    /// The current call stack, innermost frame first, for `debugstack`.
    pub fn debug_frames(&self) -> Vec<crate::DebugFrame> {
        debug_frames_in_lua(&self.lua)
    }

    pub fn set_named_registry_value(&mut self, name: &str, value: impl IntoLua) -> Result<()> {
        let value = self.to_native_value(value.into_lua()?)?;
        self.lua
            .set_named_registry_value(name, value)
            .map_err(native_error)
    }

    /// Drop the string type metatable: 5.0 has none, so `("x"):upper()` raises
    /// there. Both runtimes open one with their string library; the dialect
    /// install removes it right after.
    pub fn remove_string_metatable(&self) {
        self.lua.set_type_metatable::<mlua_native::String>(None);
    }

    pub fn named_registry_value<T: FromLua>(&mut self, name: &str) -> Result<T> {
        let value = self.lua.named_registry_value(name).map_err(native_error)?;
        T::from_lua(self.from_native_value(value)?)
    }

    pub fn remove_named_registry_value(&mut self, name: &str) -> Result<()> {
        self.set_named_registry_value(name, Value::Nil)
    }

    pub fn set_instruction_budget(&mut self, every: u32, budget: u64) -> Result<()> {
        if every == 0 {
            return Err(Error::runtime(
                "instruction hook step must be greater than zero",
            ));
        }
        i32::try_from(every).map_err(|_| {
            Error::runtime("instruction hook step does not fit the native VM counter")
        })?;
        self.instruction_count.set(0);
        let instructions = Rc::clone(&self.instruction_count);
        self.lua
            .set_hook(
                mlua_native::HookTriggers::new().every_nth_instruction(every),
                move |_, _| {
                    let used = instructions.get().saturating_add(u64::from(every));
                    instructions.set(used);
                    if used > budget {
                        return Err(mlua_native::Error::runtime(instruction_budget_error(used)));
                    }
                    Ok(mlua_native::VmState::Continue)
                },
            )
            .map_err(native_error)
    }

    pub fn clear_instruction_budget(&mut self) {
        self.lua.remove_hook();
    }

    pub fn instructions_used(&self) -> u64 {
        self.instruction_count.get()
    }

    pub fn global(&mut self, name: &str) -> Result<Value> {
        let value = self.lua.globals().get(name).map_err(native_error)?;
        self.from_native_value(value)
    }

    pub fn set_global(&mut self, name: &str, value: impl Into<Value>) -> Result<()> {
        let value = self.to_native_value(value.into())?;
        self.lua.globals().set(name, value).map_err(native_error)
    }

    pub fn global_table(&mut self, name: &str) -> Result<NativeTable> {
        let value = self.global(name)?;
        match value {
            Value::Table(table) => Ok(table),
            value => Err(Error::type_mismatch("table", &value)),
        }
    }

    pub fn set_global_table(&mut self, name: &str, table: &NativeTable) -> Result<()> {
        self.set_global(name, Value::Table(table.clone()))
    }

    pub fn global_function(&mut self, name: &str) -> Result<NativeFunction> {
        let value = self.global(name)?;
        match value {
            Value::Function(function) => Ok(function),
            value => Err(Error::type_mismatch("function", &value)),
        }
    }

    pub fn set_global_function(&mut self, name: &str, function: &NativeFunction) -> Result<()> {
        self.set_global(name, Value::Function(function.clone()))
    }

    pub fn table_get(&mut self, table: &NativeTable, key: impl IntoLua) -> Result<Value> {
        let table = self.checked_table(table)?;
        let key = self.to_native_value(key.into_lua()?)?;
        let value = table.get(key).map_err(native_error)?;
        self.from_native_value(value)
    }

    pub fn table_set(
        &mut self,
        table: &NativeTable,
        key: impl IntoLua,
        value: impl IntoLua,
    ) -> Result<()> {
        let table = self.checked_table(table)?;
        let key = self.to_native_value(key.into_lua()?)?;
        let value = self.to_native_value(value.into_lua()?)?;
        table.set(key, value).map_err(native_error)
    }

    pub fn table_get_raw(&mut self, table: &NativeTable, key: impl IntoLua) -> Result<Value> {
        let table = self.checked_table(table)?;
        let key = self.to_native_value(key.into_lua()?)?;
        let value = table.raw_get(key).map_err(native_error)?;
        self.from_native_value(value)
    }

    pub fn table_set_raw(
        &mut self,
        table: &NativeTable,
        key: impl IntoLua,
        value: impl IntoLua,
    ) -> Result<()> {
        let table = self.checked_table(table)?;
        let key = self.to_native_value(key.into_lua()?)?;
        let value = self.to_native_value(value.into_lua()?)?;
        table.raw_set(key, value).map_err(native_error)
    }

    pub fn table_get_table(
        &mut self,
        table: &NativeTable,
        key: impl IntoLua,
    ) -> Result<NativeTable> {
        let value = self.table_get(table, key)?;
        match value {
            Value::Table(table) => Ok(table),
            value => Err(Error::type_mismatch("table", &value)),
        }
    }

    pub fn table_get_function(
        &mut self,
        table: &NativeTable,
        key: impl IntoLua,
    ) -> Result<NativeFunction> {
        let value = self.table_get(table, key)?;
        match value {
            Value::Function(function) => Ok(function),
            value => Err(Error::type_mismatch("function", &value)),
        }
    }

    pub fn table_set_table(
        &mut self,
        table: &NativeTable,
        key: impl IntoLua,
        value: &NativeTable,
    ) -> Result<()> {
        self.table_set(table, key, Value::Table(value.clone()))
    }

    pub fn table_set_function(
        &mut self,
        table: &NativeTable,
        key: impl IntoLua,
        function: &NativeFunction,
    ) -> Result<()> {
        self.table_set(table, key, Value::Function(function.clone()))
    }

    pub fn table_set_metatable(
        &mut self,
        table: &NativeTable,
        metatable: Option<&NativeTable>,
    ) -> Result<()> {
        let table = self.checked_table(table)?;
        let metatable = metatable
            .map(|table| self.checked_table(table))
            .transpose()?;
        table.set_metatable(metatable).map_err(native_error)
    }

    pub fn table_metatable(&mut self, table: &NativeTable) -> Result<Option<NativeTable>> {
        let table = self.checked_table(table)?;
        Ok(table.metatable().map(|table| self.wrap_table(table)))
    }

    pub fn table_raw_len(&mut self, table: &NativeTable) -> Result<usize> {
        Ok(self.checked_table(table)?.raw_len())
    }

    pub fn table_entries(&mut self, table: &NativeTable) -> Result<Vec<(Value, Value)>> {
        let table = self.checked_table(table)?;
        table
            .pairs::<mlua_native::Value, mlua_native::Value>()
            .map(|entry| {
                let (key, value) = entry.map_err(native_error)?;
                Ok((self.from_native_value(key)?, self.from_native_value(value)?))
            })
            .collect()
    }

    pub fn table_identity(&mut self, table: &NativeTable) -> Result<u64> {
        Ok(self.checked_table(table)?.to_pointer() as usize as u64)
    }

    pub fn function_source_location(
        &mut self,
        function: &NativeFunction,
    ) -> Option<(String, usize)> {
        let function = native_checked_function(self.id, function).ok()?;
        let info = function.info();
        Some((info.short_src?, info.line_defined?))
    }

    pub fn call_function(
        &mut self,
        function: &NativeFunction,
        args: MultiValue,
    ) -> Result<MultiValue> {
        let function = self.checked_function(function)?;
        let args = args
            .into_iter()
            .map(|value| self.to_native_value(value))
            .collect::<Result<Vec<_>>>()?;
        let values: NativeMultiValue = function
            .call(NativeMultiValue::from_vec(args))
            .map_err(native_error)?;
        values
            .into_vec()
            .into_iter()
            .map(|value| self.from_native_value(value))
            .collect()
    }

    pub fn table_call_method(
        &mut self,
        table: &NativeTable,
        name: &str,
        mut args: MultiValue,
    ) -> Result<MultiValue> {
        let method = self.table_get_function(table, name)?;
        args.insert(0, Value::Table(table.clone()));
        self.call_function(&method, args)
    }

    pub fn collect_garbage(&mut self) -> Result<()> {
        self.lua.gc_collect().map_err(native_error)
    }

    fn checked_table(&self, table: &NativeTable) -> Result<mlua_native::Table> {
        native_checked_table(self.id, table)
    }

    fn checked_function(&self, function: &NativeFunction) -> Result<mlua_native::Function> {
        native_checked_function(self.id, function)
    }

    fn wrap_table(&self, table: mlua_native::Table) -> NativeTable {
        NativeTable {
            table,
            vm_id: self.id,
        }
    }

    fn wrap_function(&self, function: mlua_native::Function) -> NativeFunction {
        NativeFunction {
            function,
            vm_id: self.id,
        }
    }

    fn to_native_value(&self, value: Value) -> Result<mlua_native::Value> {
        native_value_to_backend(&self.lua, self.id, value)
    }

    fn from_native_value(&self, value: mlua_native::Value) -> Result<Value> {
        native_value_from_backend(&self.lua, self.id, value)
    }
}

impl NativeCallbackContext<'_> {
    /// Convert a value supplied by an as-yet-unported native binding into the neutral value model.
    /// This bridge disappears once `benilla-ui` no longer accepts mlua values at its boundary.
    pub fn import_native_value(&self, value: mlua_native::Value) -> Result<Value> {
        native_value_from_backend(self.lua, self.vm_id, value)
    }

    /// Convert a neutral value back for an as-yet-unported native binding.
    pub fn export_native_value(&self, value: Value) -> Result<mlua_native::Value> {
        native_value_to_backend(self.lua, self.vm_id, value)
    }

    pub fn app_data_ref<T: Any>(&self) -> Option<AppDataRef<'_, T>> {
        self.lua.app_data_ref::<T>()
    }

    pub fn app_data_mut<T: Any>(&self) -> Option<AppDataRefMut<'_, T>> {
        self.lua.app_data_mut::<T>()
    }

    pub fn remove_app_data<T: Any>(&mut self) -> Option<T> {
        self.lua.remove_app_data::<T>()
    }

    /// Live VM memory in bytes; usable inside callbacks through [`crate::LuaContext`].
    pub fn used_memory_bytes(&self) -> usize {
        self.lua.used_memory()
    }

    /// The current call stack, innermost frame first, for `debugstack`.
    pub fn debug_frames(&self) -> Vec<crate::DebugFrame> {
        debug_frames_in_lua(&self.lua)
    }

    /// Run a full collection cycle.
    pub fn collect_garbage(&mut self) -> Result<()> {
        self.lua.gc_collect().map_err(native_error)
    }

    pub fn set_named_registry_value(&mut self, name: &str, value: impl IntoLua) -> Result<()> {
        let value = native_value_to_backend(self.lua, self.vm_id, value.into_lua()?)?;
        self.lua
            .set_named_registry_value(name, value)
            .map_err(native_error)
    }

    pub fn named_registry_value<T: FromLua>(&mut self, name: &str) -> Result<T> {
        let value = self.lua.named_registry_value(name).map_err(native_error)?;
        T::from_lua(native_value_from_backend(self.lua, self.vm_id, value)?)
    }

    pub fn remove_named_registry_value(&mut self, name: &str) -> Result<()> {
        self.set_named_registry_value(name, Value::Nil)
    }

    pub fn create_table(&mut self) -> Result<NativeTable> {
        let table = self.lua.create_table().map_err(native_error)?;
        Ok(NativeTable {
            table,
            vm_id: self.vm_id,
        })
    }

    pub fn exec_named_text(&mut self, source: &[u8], name: &str) -> Result<()> {
        self.lua
            .load(source)
            .set_name(name)
            .set_mode(ChunkMode::Text)
            .exec()
            .map_err(native_error)
    }

    pub fn load_function(&mut self, source: &[u8], name: &str) -> Result<NativeFunction> {
        let function = self
            .lua
            .load(source)
            .set_name(name)
            .set_mode(ChunkMode::Text)
            .into_function()
            .map_err(native_error)?;
        Ok(NativeFunction {
            function,
            vm_id: self.vm_id,
        })
    }

    pub fn global(&mut self, name: &str) -> Result<Value> {
        let value = self.lua.globals().get(name).map_err(native_error)?;
        native_value_from_backend(self.lua, self.vm_id, value)
    }

    pub fn set_global(&mut self, name: &str, value: impl Into<Value>) -> Result<()> {
        let value = native_value_to_backend(self.lua, self.vm_id, value.into())?;
        self.lua.globals().set(name, value).map_err(native_error)
    }

    pub fn table_get(&mut self, table: &NativeTable, key: impl IntoLua) -> Result<Value> {
        let table = native_checked_table(self.vm_id, table)?;
        let key = native_value_to_backend(self.lua, self.vm_id, key.into_lua()?)?;
        let value = table.get(key).map_err(native_error)?;
        native_value_from_backend(self.lua, self.vm_id, value)
    }

    pub fn table_set(
        &mut self,
        table: &NativeTable,
        key: impl IntoLua,
        value: impl IntoLua,
    ) -> Result<()> {
        let table = native_checked_table(self.vm_id, table)?;
        let key = native_value_to_backend(self.lua, self.vm_id, key.into_lua()?)?;
        let value = native_value_to_backend(self.lua, self.vm_id, value.into_lua()?)?;
        table.set(key, value).map_err(native_error)
    }

    pub fn table_get_raw(&mut self, table: &NativeTable, key: impl IntoLua) -> Result<Value> {
        let table = native_checked_table(self.vm_id, table)?;
        let key = native_value_to_backend(self.lua, self.vm_id, key.into_lua()?)?;
        let value = table.raw_get(key).map_err(native_error)?;
        native_value_from_backend(self.lua, self.vm_id, value)
    }

    pub fn table_set_raw(
        &mut self,
        table: &NativeTable,
        key: impl IntoLua,
        value: impl IntoLua,
    ) -> Result<()> {
        let table = native_checked_table(self.vm_id, table)?;
        let key = native_value_to_backend(self.lua, self.vm_id, key.into_lua()?)?;
        let value = native_value_to_backend(self.lua, self.vm_id, value.into_lua()?)?;
        table.raw_set(key, value).map_err(native_error)
    }

    pub fn table_set_metatable(
        &mut self,
        table: &NativeTable,
        metatable: Option<&NativeTable>,
    ) -> Result<()> {
        let table = native_checked_table(self.vm_id, table)?;
        let metatable = metatable
            .map(|table| native_checked_table(self.vm_id, table))
            .transpose()?;
        table.set_metatable(metatable).map_err(native_error)
    }

    pub fn table_metatable(&mut self, table: &NativeTable) -> Result<Option<NativeTable>> {
        let table = native_checked_table(self.vm_id, table)?;
        Ok(table.metatable().map(|table| NativeTable {
            table,
            vm_id: self.vm_id,
        }))
    }

    pub fn table_raw_len(&mut self, table: &NativeTable) -> Result<usize> {
        Ok(native_checked_table(self.vm_id, table)?.raw_len())
    }

    pub fn table_entries(&mut self, table: &NativeTable) -> Result<Vec<(Value, Value)>> {
        let table = native_checked_table(self.vm_id, table)?;
        table
            .pairs::<mlua_native::Value, mlua_native::Value>()
            .map(|entry| {
                let (key, value) = entry.map_err(native_error)?;
                Ok((
                    native_value_from_backend(self.lua, self.vm_id, key)?,
                    native_value_from_backend(self.lua, self.vm_id, value)?,
                ))
            })
            .collect()
    }

    pub fn table_identity(&mut self, table: &NativeTable) -> Result<u64> {
        Ok(native_checked_table(self.vm_id, table)?.to_pointer() as usize as u64)
    }

    pub fn function_source_location(
        &mut self,
        function: &NativeFunction,
    ) -> Option<(String, usize)> {
        let function = native_checked_function(self.vm_id, function).ok()?;
        let info = function.info();
        Some((info.short_src?, info.line_defined?))
    }

    pub fn call_function(
        &mut self,
        function: &NativeFunction,
        arguments: MultiValue,
    ) -> Result<MultiValue> {
        let function = native_checked_function(self.vm_id, function)?;
        let arguments = arguments
            .into_iter()
            .map(|value| native_value_to_backend(self.lua, self.vm_id, value))
            .collect::<Result<Vec<_>>>()?;
        let values: NativeMultiValue = function
            .call(NativeMultiValue::from_vec(arguments))
            .map_err(native_error)?;
        values
            .into_vec()
            .into_iter()
            .map(|value| native_value_from_backend(self.lua, self.vm_id, value))
            .collect()
    }

    pub fn table_call_method(
        &mut self,
        table: &NativeTable,
        name: &str,
        mut arguments: MultiValue,
    ) -> Result<MultiValue> {
        let value = self.table_get(table, name)?;
        let Value::Function(function) = value else {
            return Err(Error::type_mismatch("function", &value));
        };
        arguments.insert(0, Value::Table(table.clone()));
        self.call_function(&function, arguments)
    }
}

/// Enter the neutral API from an untouched native callback that only received `&mlua::Lua`.
///
/// The VM id comes from the private marker installed by [`NativeVm`], so handles made through this
/// borrowed context retain the same cross-VM checks as handles made through the owner. This is a
/// native-only migration seam; shared production code should accept [`crate::LuaContext`] instead.
/// An mlua failure crossing into the neutral error.
///
/// mlua decorates its `Display` with a category word (`syntax error: `,
/// `runtime error: `) that the reference's own `lua_pushstring` leg never
/// adds (the same law `benilla_ui::script::stdlib::lua_message` applies on
/// the still-native loadstring path). Strip it here so script channels
/// record the image's own message shape on both backends; anything that is
/// not a script failure keeps the backend rendering.
fn native_error(error: mlua_native::Error) -> Error {
    match error {
        mlua_native::Error::SyntaxError { message, .. } => Error::message(message),
        mlua_native::Error::RuntimeError(message) => Error::message(message),
        error => Error::backend(error),
    }
}

/// Read the mlua call stack into neutral frames, innermost first.
///
/// A direct transcription of `mlua::Debug` — no wording of ours, so the
/// formatter above sees the same fields on both backends.
fn debug_frames_in_lua(lua: &Lua) -> Vec<crate::DebugFrame> {
    use crate::{DebugFrame, DebugWhat};

    let mut out = Vec::new();
    let mut level = 0;
    while let Some(frame) = lua.inspect_stack(level, |d| {
        let source = d.source();
        DebugFrame {
            source: source.source.map(|s| s.into_owned()),
            short_src: source.short_src.map(|s| s.into_owned()),
            what: match source.what {
                "main" => DebugWhat::Main,
                "C" => DebugWhat::C,
                _ => DebugWhat::Lua,
            },
            line_defined: source.line_defined,
            current_line: d.current_line(),
            name: d.names().name.map(|s| s.into_owned()),
        }
    }) {
        out.push(frame);
        level += 1;
    }
    out
}

pub fn with_native_context<R>(
    lua: &Lua,
    callback: impl FnOnce(&mut NativeCallbackContext<'_>) -> Result<R>,
) -> Result<R> {
    let vm_id = lua
        .app_data_ref::<NativeVmIdentity>()
        .map(|identity| identity.0)
        .ok_or_else(|| Error::message("native Lua state is not owned by benilla_lua::Vm"))?;
    callback(&mut NativeCallbackContext { lua, vm_id })
}

fn native_checked_table(vm_id: u64, table: &NativeTable) -> Result<mlua_native::Table> {
    if table.vm_id != vm_id {
        return Err(Error::message("table belongs to another Lua VM"));
    }
    Ok(table.table.clone())
}

fn native_checked_function(vm_id: u64, function: &NativeFunction) -> Result<mlua_native::Function> {
    if function.vm_id != vm_id {
        return Err(Error::message("function belongs to another Lua VM"));
    }
    Ok(function.function.clone())
}

fn native_value_to_backend(lua: &Lua, vm_id: u64, value: Value) -> Result<mlua_native::Value> {
    Ok(match value {
        Value::Nil => mlua_native::Value::Nil,
        Value::Boolean(value) => mlua_native::Value::Boolean(value),
        Value::Integer(value) => mlua_native::Value::Integer(value),
        Value::Number(value) => mlua_native::Value::Number(value),
        Value::String(value) => {
            mlua_native::Value::String(lua.create_string(value.as_bytes()).map_err(native_error)?)
        }
        Value::Table(table) => mlua_native::Value::Table(native_checked_table(vm_id, &table)?),
        Value::Function(function) => {
            mlua_native::Value::Function(native_checked_function(vm_id, &function)?)
        }
        Value::LightUserData(value) => {
            mlua_native::Value::LightUserData(mlua_native::LightUserData(value as *mut c_void))
        }
    })
}

fn native_value_from_backend(_lua: &Lua, vm_id: u64, value: mlua_native::Value) -> Result<Value> {
    Ok(match value {
        mlua_native::Value::Nil => Value::Nil,
        mlua_native::Value::Boolean(value) => Value::Boolean(value),
        mlua_native::Value::Integer(value) => Value::Integer(value),
        mlua_native::Value::Number(value) => Value::Number(value),
        mlua_native::Value::String(value) => {
            Value::String(LuaString::from(value.as_bytes().to_vec()))
        }
        mlua_native::Value::Table(table) => Value::Table(NativeTable { table, vm_id }),
        mlua_native::Value::Function(function) => {
            Value::Function(NativeFunction { function, vm_id })
        }
        mlua_native::Value::LightUserData(value) => Value::LightUserData(value.0 as usize),
        value => {
            return Err(Error::message(format!(
                "unsupported native Lua value type {}",
                value.type_name()
            )));
        }
    })
}

fn instruction_budget_error(used: u64) -> String {
    format!(
        "benilla: instruction budget exhausted after {used} VM instructions — treating this as a non-terminating loop"
    )
}
