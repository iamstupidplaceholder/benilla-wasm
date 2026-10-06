//! An explicit, GC-safe bridge for the browser UI port.
//!
//! Rilua operations take `&mut Lua`, unlike mlua's `&Lua` object API. Keep
//! that difference visible here instead of using interior mutability that
//! would fail when a Lua call synchronously invokes a Rust callback.

#[cfg(any(not(feature = "mlua-backend"), target_arch = "wasm32"))]
use std::any::Any;
#[cfg(any(not(feature = "mlua-backend"), target_arch = "wasm32"))]
use std::cell::{Ref, RefMut};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
};

#[cfg(any(not(feature = "mlua-backend"), target_arch = "wasm32"))]
use rilua::vm::callinfo::LUA_MULTRET;
use rilua::vm::state::LuaState;
#[cfg(any(not(feature = "mlua-backend"), target_arch = "wasm32"))]
use rilua::vm::state::MASK_COUNT;
#[cfg(any(not(feature = "mlua-backend"), target_arch = "wasm32"))]
use rilua::LuaApi;
use rilua::{
    runtime_error, FromLua, FromLuaMulti, Function, IntoLua, IntoLuaMulti, Lua, LuaApiMut,
    LuaResult, StdLib, Table, Val,
};

#[cfg(any(not(feature = "mlua-backend"), target_arch = "wasm32"))]
use crate::host_registry::{named_registry_value_in_state, set_named_registry_value_in_state};
#[cfg(any(not(feature = "mlua-backend"), target_arch = "wasm32"))]
use crate::AppData;
use crate::{create_host_callback, set_named_registry_value};

static NEXT_VM_ID: AtomicU64 = AtomicU64::new(1);

/// A browser Lua VM with explicit mutable access and rooted object handles.
pub struct BrowserVm {
    lua: Lua,
    id: u64,
    roots: Table,
    root_slots: Rc<RefCell<RootSlots>>,
}

/// A table rooted in this VM's Lua registry until the last Rust handle drops.
#[derive(Clone)]
pub struct BrowserTable {
    table: Table,
    root: Rc<Root>,
}

/// A compiled Lua function or captured host callback rooted in the VM registry.
#[derive(Clone)]
pub struct BrowserFunction {
    function: Function,
    root: Rc<Root>,
}

struct Root {
    vm_id: u64,
    slot: u32,
    slots: Rc<RefCell<RootSlots>>,
}

/// Numeric handles in a private, registry-rooted table. Reuse only after the
/// last Rc drops AND its old entry has been cleared by exclusive VM access.
/// This avoids formatting/interning a unique registry string for every value
/// crossing the Lua/host boundary; Lua GC still owns and traces all values.
#[derive(Default)]
struct RootSlots {
    next: u32,
    free: Vec<u32>,
    released: Vec<u32>,
}

impl RootSlots {
    fn flush(&mut self, state: &mut LuaState, roots: Table) -> LuaResult<()> {
        while let Some(&slot) = self.released.last() {
            roots.raw_set(state, Val::Num(f64::from(slot)), Val::Nil)?;
            self.released.pop();
            self.free.push(slot);
        }
        Ok(())
    }

    fn insert(&mut self, state: &mut LuaState, roots: Table, value: Val) -> LuaResult<u32> {
        let slot = match self.free.pop() {
            Some(slot) => slot,
            None => {
                self.next = self
                    .next
                    .checked_add(1)
                    .ok_or_else(|| runtime_error("browser Lua root slots exhausted"))?;
                self.next
            }
        };
        if let Err(error) = roots.raw_set(state, Val::Num(f64::from(slot)), value) {
            self.free.push(slot);
            return Err(error);
        }
        Ok(slot)
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        // Dropping a handle may happen inside a Lua callback. Defer registry
        // mutation until the next explicit, exclusive VM operation.
        self.slots.borrow_mut().released.push(self.slot);
    }
}

impl BrowserVm {
    /// Start with the same restricted library set used by the existing web
    /// Lua probes; production sandboxing remains a separate migration step.
    pub fn new() -> LuaResult<Self> {
        let mut lua = Lua::new_with(StdLib::BASE | StdLib::STRING | StdLib::TABLE | StdLib::MATH)?;
        let roots = lua.create_table();
        set_named_registry_value(&mut lua, "browser_roots", Val::Table(roots.gc_ref()))?;
        Ok(Self {
            lua,
            id: NEXT_VM_ID.fetch_add(1, Ordering::Relaxed),
            roots,
            root_slots: Rc::new(RefCell::new(RootSlots::default())),
        })
    }

    fn flush_released_roots(&mut self) -> LuaResult<()> {
        self.root_slots
            .borrow_mut()
            .flush(self.lua.state_mut(), self.roots)
    }

    fn root_table(&mut self, table: Table) -> LuaResult<BrowserTable> {
        let root = self.root_value("table", Val::Table(table.gc_ref()))?;
        Ok(BrowserTable { table, root })
    }

    fn root_function(&mut self, function: Function) -> LuaResult<BrowserFunction> {
        let root = self.root_value("function", Val::Function(function.gc_ref()))?;
        Ok(BrowserFunction { function, root })
    }

    fn root_value(&mut self, _kind: &str, value: Val) -> LuaResult<Rc<Root>> {
        self.flush_released_roots()?;
        let slot = self
            .root_slots
            .borrow_mut()
            .insert(self.lua.state_mut(), self.roots, value)?;
        Ok(Rc::new(Root {
            vm_id: self.id,
            slot,
            slots: Rc::clone(&self.root_slots),
        }))
    }

    fn checked_table(&self, table: &BrowserTable) -> LuaResult<Table> {
        if table.root.vm_id != self.id {
            return Err(runtime_error("table belongs to another browser Lua VM"));
        }
        Ok(table.table)
    }

    fn checked_function(&self, function: &BrowserFunction) -> LuaResult<Function> {
        if function.root.vm_id != self.id {
            return Err(runtime_error("function belongs to another browser Lua VM"));
        }
        Ok(function.function)
    }

    pub fn create_table(&mut self) -> LuaResult<BrowserTable> {
        let table = self.lua.create_table();
        self.root_table(table)
    }

    pub fn load_function(&mut self, source: &[u8], name: &str) -> LuaResult<BrowserFunction> {
        self.flush_released_roots()?;
        reject_bytecode(source, name)?;
        let function = self.lua.load_bytes(source, name)?;
        self.root_function(function)
    }

    /// Host callback registration remains low-level until argument conversion
    /// and callback re-entry have a dedicated adapter.
    pub fn create_host_function_raw(
        &mut self,
        name: &str,
        callback: impl Fn(&mut LuaState) -> LuaResult<u32> + 'static,
    ) -> LuaResult<BrowserFunction> {
        self.flush_released_roots()?;
        let function = create_host_callback(&mut self.lua, name, callback);
        let function = Function::from_lua(function, &self.lua)?;
        self.root_function(function)
    }

    /// A captured callback with typed Lua arguments and return values. The
    /// `LuaState` argument is the currently active VM; callbacks can inspect
    /// or update it without borrowing `BrowserVm` recursively.
    pub fn create_host_function<A, R>(
        &mut self,
        name: &str,
        callback: impl Fn(&mut LuaState, A) -> LuaResult<R> + 'static,
    ) -> LuaResult<BrowserFunction>
    where
        A: FromLuaMulti + 'static,
        R: IntoLuaMulti + 'static,
    {
        self.create_host_function_raw(name, move |state| {
            let arguments: Vec<Val> = (state.base..state.top)
                .map(|index| state.stack_get(index))
                .collect();
            let arguments = A::from_lua_multi(&arguments, state)?;
            let result = callback(state, arguments)?;
            let values = result.into_lua_multi(state)?;
            let count = u32::try_from(values.len())
                .map_err(|_| runtime_error("too many host callback return values"))?;
            for value in values {
                state.push(value);
            }
            Ok(count)
        })
    }

    pub fn call_function<A: IntoLuaMulti, R: FromLuaMulti>(
        &mut self,
        function: &BrowserFunction,
        args: A,
    ) -> LuaResult<R> {
        let function = self.checked_function(function)?;
        self.flush_released_roots()?;
        let args = args.into_lua_multi(&mut self.lua)?;
        // The traced variant restores the VM stack even when a script errors.
        let results = self.lua.call_function_traced(&function, &args)?;
        R::from_lua_multi(&results, &self.lua)
    }

    pub fn table_set<K: IntoLua, V: IntoLua>(
        &mut self,
        table: &BrowserTable,
        key: K,
        value: V,
    ) -> LuaResult<()> {
        let table = self.checked_table(table)?;
        self.flush_released_roots()?;
        let key = key.into_lua(&mut self.lua)?;
        let value = value.into_lua(&mut self.lua)?;
        self.lua
            .state_mut()
            .settable(Val::Table(table.gc_ref()), key, value)
    }

    pub fn table_set_table<K: IntoLua>(
        &mut self,
        table: &BrowserTable,
        key: K,
        value: &BrowserTable,
    ) -> LuaResult<()> {
        let target = self.checked_table(table)?;
        let value = self.checked_table(value)?;
        self.flush_released_roots()?;
        let key = key.into_lua(&mut self.lua)?;
        self.lua
            .state_mut()
            .settable(Val::Table(target.gc_ref()), key, Val::Table(value.gc_ref()))
    }

    pub fn table_set_function<K: IntoLua>(
        &mut self,
        table: &BrowserTable,
        key: K,
        function: &BrowserFunction,
    ) -> LuaResult<()> {
        let table = self.checked_table(table)?;
        let function = self.checked_function(function)?;
        self.flush_released_roots()?;
        let key = key.into_lua(&mut self.lua)?;
        self.lua.state_mut().settable(
            Val::Table(table.gc_ref()),
            key,
            Val::Function(function.gc_ref()),
        )
    }

    pub fn table_get<K: IntoLua, V: FromLua>(
        &mut self,
        table: &BrowserTable,
        key: K,
    ) -> LuaResult<V> {
        let table = self.checked_table(table)?;
        self.flush_released_roots()?;
        let key = key.into_lua(&mut self.lua)?;
        let value = self
            .lua
            .state_mut()
            .gettable(Val::Table(table.gc_ref()), key)?;
        V::from_lua(value, &self.lua)
    }

    pub fn table_get_table<K: IntoLua>(
        &mut self,
        table: &BrowserTable,
        key: K,
    ) -> LuaResult<BrowserTable> {
        let table = self.checked_table(table)?;
        self.flush_released_roots()?;
        let key = key.into_lua(&mut self.lua)?;
        let value = self
            .lua
            .state_mut()
            .gettable(Val::Table(table.gc_ref()), key)?;
        let table = Table::from_lua(value, &self.lua)?;
        self.root_table(table)
    }

    pub fn table_get_function<K: IntoLua>(
        &mut self,
        table: &BrowserTable,
        key: K,
    ) -> LuaResult<BrowserFunction> {
        let table = self.checked_table(table)?;
        self.flush_released_roots()?;
        let key = key.into_lua(&mut self.lua)?;
        let value = self
            .lua
            .state_mut()
            .gettable(Val::Table(table.gc_ref()), key)?;
        let function = Function::from_lua(value, &self.lua)?;
        self.root_function(function)
    }

    /// Call a table method with the table as Lua's implicit `self` argument.
    /// `gettable` deliberately honors `__index`, as FrameScript methods live
    /// on shared metatables rather than as raw keys on every frame.
    pub fn table_call_method<A: IntoLuaMulti, R: FromLuaMulti>(
        &mut self,
        table: &BrowserTable,
        name: &str,
        args: A,
    ) -> LuaResult<R> {
        let table = self.checked_table(table)?;
        self.flush_released_roots()?;
        let name = name.into_lua(&mut self.lua)?;
        let method = self
            .lua
            .state_mut()
            .gettable(Val::Table(table.gc_ref()), name)?;
        let method = Function::from_lua(method, &self.lua)?;
        let mut args = args.into_lua_multi(&mut self.lua)?;
        args.insert(0, Val::Table(table.gc_ref()));
        let results = self.lua.call_function_traced(&method, &args)?;
        R::from_lua_multi(&results, &self.lua)
    }

    pub fn table_set_metatable(
        &mut self,
        table: &BrowserTable,
        metatable: Option<&BrowserTable>,
    ) -> LuaResult<()> {
        let table = self.checked_table(table)?;
        let metatable = metatable
            .map(|table| self.checked_table(table))
            .transpose()?;
        self.flush_released_roots()?;
        table.set_metatable(self.lua.state_mut(), metatable)
    }

    pub fn set_global_table(&mut self, name: &str, table: &BrowserTable) -> LuaResult<()> {
        let table = self.checked_table(table)?;
        self.flush_released_roots()?;
        self.lua.set_global_val(name, Val::Table(table.gc_ref()))
    }

    pub fn set_global_function(&mut self, name: &str, function: &BrowserFunction) -> LuaResult<()> {
        let function = self.checked_function(function)?;
        self.flush_released_roots()?;
        self.lua
            .set_global_val(name, Val::Function(function.gc_ref()))
    }

    pub fn global_table(&mut self, name: &str) -> LuaResult<BrowserTable> {
        self.flush_released_roots()?;
        let value = self.lua.get_global_val(name);
        let table = Table::from_lua(value, &self.lua)?;
        self.root_table(table)
    }

    pub fn global_function(&mut self, name: &str) -> LuaResult<BrowserFunction> {
        self.flush_released_roots()?;
        let value = self.lua.get_global_val(name);
        let function = Function::from_lua(value, &self.lua)?;
        self.root_function(function)
    }

    pub fn global<V: FromLua>(&mut self, name: &str) -> LuaResult<V> {
        self.flush_released_roots()?;
        self.lua.global(name)
    }

    /// Publish a primitive global. Tables and functions keep their rooted, typed entry points.
    pub fn set_global<V: IntoLua>(&mut self, name: &str, value: V) -> LuaResult<()> {
        self.flush_released_roots()?;
        let value = value.into_lua(&mut self.lua)?;
        self.lua.set_global_val(name, value)
    }

    pub fn exec_bytes(&mut self, source: &[u8], name: &str) -> LuaResult<()> {
        self.flush_released_roots()?;
        reject_bytecode(source, name)?;
        let function = self.lua.load_bytes(source, name)?;
        self.lua.call_function_traced(&function, &[]).map(|_| ())
    }

    pub fn collect_garbage(&mut self) -> LuaResult<()> {
        self.flush_released_roots()?;
        self.lua.gc_collect()
    }
}

// The production-facing facade uses rilua on every target. `native_vm` supplies the same facade
// only when a native comparison build explicitly enables `mlua-backend`.
#[cfg(any(not(feature = "mlua-backend"), target_arch = "wasm32"))]
pub struct Vm {
    inner: BrowserVm,
    app_data: AppData,
    instruction_count: Rc<Cell<u64>>,
}

#[cfg(any(not(feature = "mlua-backend"), target_arch = "wasm32"))]
#[derive(Clone)]
pub struct VmTable(BrowserTable);

#[cfg(any(not(feature = "mlua-backend"), target_arch = "wasm32"))]
#[derive(Clone)]
pub struct VmFunction(BrowserFunction);

#[cfg(any(not(feature = "mlua-backend"), target_arch = "wasm32"))]
pub struct BrowserCallbackContext<'state> {
    state: &'state mut LuaState,
    vm_id: u64,
    roots: Table,
    root_slots: Rc<RefCell<RootSlots>>,
    app_data: AppData,
}

#[cfg(any(not(feature = "mlua-backend"), target_arch = "wasm32"))]
impl std::fmt::Debug for VmTable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Table")
            .field("vm", &self.0.root.vm_id)
            .finish()
    }
}

#[cfg(any(not(feature = "mlua-backend"), target_arch = "wasm32"))]
impl std::fmt::Debug for VmFunction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Function")
            .field("vm", &self.0.root.vm_id)
            .finish()
    }
}

/// Function identity, for the host's default-handler skip: two handles name
/// the same closure in the same VM. The rilua handle is itself an identity
/// (`GcRef` equality), so the root is not consulted.
#[cfg(any(not(feature = "mlua-backend"), target_arch = "wasm32"))]
impl PartialEq for BrowserFunction {
    fn eq(&self, other: &Self) -> bool {
        self.function == other.function
    }
}

#[cfg(any(not(feature = "mlua-backend"), target_arch = "wasm32"))]
impl PartialEq for VmFunction {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

#[cfg(any(not(feature = "mlua-backend"), target_arch = "wasm32"))]
impl Vm {
    pub fn new() -> crate::Result<Self> {
        BrowserVm::new()
            .map(|inner| Self {
                inner,
                app_data: AppData::default(),
                instruction_count: Rc::new(Cell::new(0)),
            })
            .map_err(crate::Error::backend)
    }

    /// Live VM memory in bytes, for the 5.0 `gcinfo` pair.
    pub fn used_memory_bytes(&self) -> usize {
        self.inner.lua.gc_count()
    }

    /// The current call stack, innermost frame first, for `debugstack`.
    pub fn debug_frames(&self) -> Vec<crate::DebugFrame> {
        debug_frames_in_state(self.inner.lua.state())
    }

    /// Drop the string type metatable: 5.0 has none, so `("x"):upper()` raises
    /// there. Both runtimes open one with their string library; the dialect
    /// install removes it right after.
    pub fn remove_string_metatable(&mut self) {
        // rilua's own tag mapping (`metatable::type_tag`): Nil=0 … String=3.
        self.inner.lua.state_mut().gc.type_metatables[3] = None;
    }

    pub fn exec_named_text(&mut self, source: &[u8], name: &str) -> crate::Result<()> {
        self.inner
            .exec_bytes(source, name)
            .map_err(crate::Error::backend)
    }

    pub fn load_function(&mut self, source: &[u8], name: &str) -> crate::Result<VmFunction> {
        self.inner
            .load_function(source, name)
            .map(VmFunction)
            .map_err(crate::Error::backend)
    }

    pub fn create_host_function(
        &mut self,
        name: &str,
        callback: impl Fn(
                &mut BrowserCallbackContext<'_>,
                crate::MultiValue,
            ) -> crate::Result<crate::MultiValue>
            + 'static,
    ) -> crate::Result<VmFunction> {
        let vm_id = self.inner.id;
        let roots = self.inner.roots;
        let root_slots = Rc::clone(&self.inner.root_slots);
        let app_data = self.app_data.clone();
        self.inner
            .create_host_function_raw(name, move |state| {
                let arguments = (state.base..state.top)
                    .map(|index| state.stack_get(index))
                    .collect::<Vec<_>>();
                let mut context = BrowserCallbackContext {
                    state,
                    vm_id,
                    roots,
                    root_slots: Rc::clone(&root_slots),
                    app_data: app_data.clone(),
                };
                let arguments = arguments
                    .into_iter()
                    .map(|value| context.from_raw_value(value))
                    .collect::<crate::Result<Vec<_>>>()
                    .map_err(|error| runtime_error(error.to_string()))?;
                let values = callback(&mut context, arguments)
                    .map_err(|error| runtime_error(error.to_string()))?;
                let count = u32::try_from(values.len())
                    .map_err(|_| runtime_error("too many host callback return values"))?;
                for value in values {
                    let value = context
                        .to_raw_value(value)
                        .map_err(|error| runtime_error(error.to_string()))?;
                    context.state.push(value);
                }
                Ok(count)
            })
            .map(VmFunction)
            .map_err(crate::Error::backend)
    }

    pub fn create_function<A, R>(
        &mut self,
        name: &str,
        callback: impl Fn(&mut BrowserCallbackContext<'_>, A) -> crate::Result<R> + 'static,
    ) -> crate::Result<VmFunction>
    where
        A: crate::FromLuaMulti + 'static,
        R: crate::IntoLuaMulti + 'static,
    {
        self.create_host_function(name, move |context, arguments| {
            callback(context, A::from_lua_multi(arguments)?)?.into_lua_multi()
        })
    }

    pub fn register_function<A, R>(
        &mut self,
        name: &str,
        callback: impl Fn(&mut BrowserCallbackContext<'_>, A) -> crate::Result<R> + 'static,
    ) -> crate::Result<()>
    where
        A: crate::FromLuaMulti + 'static,
        R: crate::IntoLuaMulti + 'static,
    {
        let function = self.create_function(name, callback)?;
        self.set_global_function(name, &function)
    }

    pub fn create_table(&mut self) -> crate::Result<VmTable> {
        self.inner
            .create_table()
            .map(VmTable)
            .map_err(crate::Error::backend)
    }

    pub fn app_data(&self) -> &AppData {
        &self.app_data
    }

    pub fn set_app_data<T: Any>(&mut self, value: T) -> Option<T> {
        self.app_data.set(value)
    }

    pub fn app_data_ref<T: Any>(&self) -> Option<Ref<'_, T>> {
        self.app_data.get::<T>()
    }

    pub fn app_data_mut<T: Any>(&self) -> Option<RefMut<'_, T>> {
        self.app_data.get_mut::<T>()
    }

    pub fn remove_app_data<T: Any>(&mut self) -> Option<T> {
        self.app_data.remove::<T>()
    }

    pub fn set_named_registry_value(
        &mut self,
        name: &str,
        value: impl crate::IntoLua,
    ) -> crate::Result<()> {
        self.inner
            .flush_released_roots()
            .map_err(crate::Error::backend)?;
        let value = self.to_raw_value(value.into_lua()?)?;
        set_named_registry_value_in_state(self.inner.lua.state_mut(), name, value)
            .map_err(crate::Error::backend)
    }

    pub fn named_registry_value<T: crate::FromLua>(&mut self, name: &str) -> crate::Result<T> {
        self.inner
            .flush_released_roots()
            .map_err(crate::Error::backend)?;
        let value = named_registry_value_in_state(self.inner.lua.state_mut(), name)
            .map_err(crate::Error::backend)?;
        T::from_lua(self.from_raw_value(value)?)
    }

    pub fn remove_named_registry_value(&mut self, name: &str) -> crate::Result<()> {
        self.set_named_registry_value(name, crate::Value::Nil)
    }

    pub fn set_instruction_budget(&mut self, every: u32, budget: u64) -> crate::Result<()> {
        let every = i32::try_from(every).map_err(|_| {
            crate::Error::runtime("instruction hook step does not fit the browser VM counter")
        })?;
        if every == 0 {
            return Err(crate::Error::runtime(
                "instruction hook step must be greater than zero",
            ));
        }

        self.inner
            .flush_released_roots()
            .map_err(crate::Error::backend)?;
        self.instruction_count.set(0);
        let instructions = Rc::clone(&self.instruction_count);
        let hook = create_host_callback(
            &mut self.inner.lua,
            "benilla_instruction_budget",
            move |_| {
                let used = instructions.get().saturating_add(every as u64);
                instructions.set(used);
                if used > budget {
                    return Err(runtime_error(instruction_budget_error(used)));
                }
                Ok(0)
            },
        );
        let state = self.inner.lua.state_mut();
        state.hook.hook_func = hook;
        state.hook.hook_mask = MASK_COUNT;
        state.hook.allow_hook = true;
        state.hook.base_hook_count = every;
        state.hook.hook_count = every;
        state.hook.yield_on_hook = false;
        Ok(())
    }

    pub fn clear_instruction_budget(&mut self) {
        self.inner.lua.state_mut().hook = Default::default();
    }

    pub fn instructions_used(&self) -> u64 {
        self.instruction_count.get()
    }

    pub fn global(&mut self, name: &str) -> crate::Result<crate::Value> {
        self.inner
            .flush_released_roots()
            .map_err(crate::Error::backend)?;
        let value = self.inner.lua.get_global_val(name);
        self.from_raw_value(value)
    }

    pub fn set_global(&mut self, name: &str, value: impl Into<crate::Value>) -> crate::Result<()> {
        self.inner
            .flush_released_roots()
            .map_err(crate::Error::backend)?;
        let value = self.to_raw_value(value.into())?;
        self.inner
            .lua
            .set_global_val(name, value)
            .map_err(crate::Error::backend)
    }

    pub fn global_table(&mut self, name: &str) -> crate::Result<VmTable> {
        let value = self.global(name)?;
        match value {
            crate::Value::Table(table) => Ok(table),
            value => Err(crate::Error::type_mismatch("table", &value)),
        }
    }

    pub fn set_global_table(&mut self, name: &str, table: &VmTable) -> crate::Result<()> {
        self.set_global(name, crate::Value::Table(table.clone()))
    }

    pub fn global_function(&mut self, name: &str) -> crate::Result<VmFunction> {
        let value = self.global(name)?;
        match value {
            crate::Value::Function(function) => Ok(function),
            value => Err(crate::Error::type_mismatch("function", &value)),
        }
    }

    pub fn set_global_function(&mut self, name: &str, function: &VmFunction) -> crate::Result<()> {
        self.set_global(name, crate::Value::Function(function.clone()))
    }

    pub fn table_get(
        &mut self,
        table: &VmTable,
        key: impl crate::IntoLua,
    ) -> crate::Result<crate::Value> {
        self.inner
            .flush_released_roots()
            .map_err(crate::Error::backend)?;
        let table = self
            .inner
            .checked_table(&table.0)
            .map_err(crate::Error::backend)?;
        let key = self.to_raw_value(crate::IntoLua::into_lua(key)?)?;
        let value = self
            .inner
            .lua
            .state_mut()
            .gettable(Val::Table(table.gc_ref()), key)
            .map_err(crate::Error::backend)?;
        self.from_raw_value(value)
    }

    pub fn table_set(
        &mut self,
        table: &VmTable,
        key: impl crate::IntoLua,
        value: impl crate::IntoLua,
    ) -> crate::Result<()> {
        self.inner
            .flush_released_roots()
            .map_err(crate::Error::backend)?;
        let table = self
            .inner
            .checked_table(&table.0)
            .map_err(crate::Error::backend)?;
        let key = self.to_raw_value(crate::IntoLua::into_lua(key)?)?;
        let value = self.to_raw_value(crate::IntoLua::into_lua(value)?)?;
        self.inner
            .lua
            .state_mut()
            .settable(Val::Table(table.gc_ref()), key, value)
            .map_err(crate::Error::backend)
    }

    pub fn table_get_raw(
        &mut self,
        table: &VmTable,
        key: impl crate::IntoLua,
    ) -> crate::Result<crate::Value> {
        self.inner
            .flush_released_roots()
            .map_err(crate::Error::backend)?;
        let table = self
            .inner
            .checked_table(&table.0)
            .map_err(crate::Error::backend)?;
        let key = self.to_raw_value(crate::IntoLua::into_lua(key)?)?;
        let value = table
            .raw_get(self.inner.lua.state_mut(), key)
            .map_err(crate::Error::backend)?;
        self.from_raw_value(value)
    }

    pub fn table_set_raw(
        &mut self,
        table: &VmTable,
        key: impl crate::IntoLua,
        value: impl crate::IntoLua,
    ) -> crate::Result<()> {
        self.inner
            .flush_released_roots()
            .map_err(crate::Error::backend)?;
        let table = self
            .inner
            .checked_table(&table.0)
            .map_err(crate::Error::backend)?;
        let key = self.to_raw_value(crate::IntoLua::into_lua(key)?)?;
        let value = self.to_raw_value(crate::IntoLua::into_lua(value)?)?;
        table
            .raw_set(self.inner.lua.state_mut(), key, value)
            .map_err(crate::Error::backend)
    }

    pub fn table_get_table(
        &mut self,
        table: &VmTable,
        key: impl crate::IntoLua,
    ) -> crate::Result<VmTable> {
        let value = self.table_get(table, key)?;
        match value {
            crate::Value::Table(table) => Ok(table),
            value => Err(crate::Error::type_mismatch("table", &value)),
        }
    }

    pub fn table_get_function(
        &mut self,
        table: &VmTable,
        key: impl crate::IntoLua,
    ) -> crate::Result<VmFunction> {
        let value = self.table_get(table, key)?;
        match value {
            crate::Value::Function(function) => Ok(function),
            value => Err(crate::Error::type_mismatch("function", &value)),
        }
    }

    pub fn table_set_table(
        &mut self,
        table: &VmTable,
        key: impl crate::IntoLua,
        value: &VmTable,
    ) -> crate::Result<()> {
        self.table_set(table, key, crate::Value::Table(value.clone()))
    }

    pub fn table_set_function(
        &mut self,
        table: &VmTable,
        key: impl crate::IntoLua,
        function: &VmFunction,
    ) -> crate::Result<()> {
        self.table_set(table, key, crate::Value::Function(function.clone()))
    }

    pub fn table_set_metatable(
        &mut self,
        table: &VmTable,
        metatable: Option<&VmTable>,
    ) -> crate::Result<()> {
        self.inner
            .table_set_metatable(&table.0, metatable.map(|table| &table.0))
            .map_err(crate::Error::backend)
    }

    pub fn table_metatable(&mut self, table: &VmTable) -> crate::Result<Option<VmTable>> {
        self.inner
            .flush_released_roots()
            .map_err(crate::Error::backend)?;
        let table = self
            .inner
            .checked_table(&table.0)
            .map_err(crate::Error::backend)?;
        let Some(metatable) = table.metatable(self.inner.lua.state_mut()) else {
            return Ok(None);
        };
        Ok(Some(VmTable(
            self.inner
                .root_table(metatable)
                .map_err(crate::Error::backend)?,
        )))
    }

    pub fn table_raw_len(&mut self, table: &VmTable) -> crate::Result<usize> {
        self.inner
            .flush_released_roots()
            .map_err(crate::Error::backend)?;
        let table = self
            .inner
            .checked_table(&table.0)
            .map_err(crate::Error::backend)?;
        usize::try_from(table.raw_len(self.inner.lua.state_mut()))
            .map_err(|_| crate::Error::runtime("Lua table length does not fit usize"))
    }

    pub fn table_entries(
        &mut self,
        table: &VmTable,
    ) -> crate::Result<Vec<(crate::Value, crate::Value)>> {
        self.inner
            .flush_released_roots()
            .map_err(crate::Error::backend)?;
        let table = self
            .inner
            .checked_table(&table.0)
            .map_err(crate::Error::backend)?;
        let mut cursor = Val::Nil;
        let mut entries = Vec::new();
        while let Some((key, value)) = table
            .next(self.inner.lua.state_mut(), cursor)
            .map_err(crate::Error::backend)?
        {
            cursor = key;
            entries.push((self.from_raw_value(key)?, self.from_raw_value(value)?));
        }
        Ok(entries)
    }

    pub fn table_identity(&mut self, table: &VmTable) -> crate::Result<u64> {
        self.inner
            .checked_table(&table.0)
            .map(|table| u64::from(table.identity()))
            .map_err(crate::Error::backend)
    }

    pub fn function_source_location(&mut self, function: &VmFunction) -> Option<(String, usize)> {
        self.inner.flush_released_roots().ok()?;
        let function = self.inner.checked_function(&function.0).ok()?;
        let (source, line) = function.source_location(self.inner.lua.state())?;
        Some((source, line as usize))
    }

    pub fn call_function(
        &mut self,
        function: &VmFunction,
        args: crate::MultiValue,
    ) -> crate::Result<crate::MultiValue> {
        self.inner
            .flush_released_roots()
            .map_err(crate::Error::backend)?;
        let function = self
            .inner
            .checked_function(&function.0)
            .map_err(crate::Error::backend)?;
        let args = args
            .into_iter()
            .map(|value| self.to_raw_value(value))
            .collect::<crate::Result<Vec<_>>>()?;
        let values = self
            .inner
            .lua
            .call_function_traced(&function, &args)
            .map_err(crate::Error::backend)?;
        values
            .into_iter()
            .map(|value| self.from_raw_value(value))
            .collect()
    }

    pub fn table_call_method(
        &mut self,
        table: &VmTable,
        name: &str,
        mut args: crate::MultiValue,
    ) -> crate::Result<crate::MultiValue> {
        let method = self.table_get_function(table, name)?;
        args.insert(0, crate::Value::Table(table.clone()));
        self.call_function(&method, args)
    }

    pub fn collect_garbage(&mut self) -> crate::Result<()> {
        self.inner.collect_garbage().map_err(crate::Error::backend)
    }

    fn to_raw_value(&mut self, value: crate::Value) -> crate::Result<Val> {
        Ok(match value {
            crate::Value::Nil => Val::Nil,
            crate::Value::Boolean(value) => Val::Bool(value),
            crate::Value::Integer(value) => Val::Num(value as f64),
            crate::Value::Number(value) => Val::Num(value),
            crate::Value::String(value) => value
                .as_bytes()
                .into_lua(&mut self.inner.lua)
                .map_err(crate::Error::backend)?,
            crate::Value::Table(table) => {
                let table = self
                    .inner
                    .checked_table(&table.0)
                    .map_err(crate::Error::backend)?;
                Val::Table(table.gc_ref())
            }
            crate::Value::Function(function) => {
                let function = self
                    .inner
                    .checked_function(&function.0)
                    .map_err(crate::Error::backend)?;
                Val::Function(function.gc_ref())
            }
            crate::Value::LightUserData(value) => Val::LightUserdata(value),
        })
    }

    fn from_raw_value(&mut self, value: Val) -> crate::Result<crate::Value> {
        Ok(match value {
            Val::Nil => crate::Value::Nil,
            Val::Bool(value) => crate::Value::Boolean(value),
            Val::Num(value) => crate::Value::Number(value),
            Val::Str(_) => {
                let bytes =
                    Vec::<u8>::from_lua(value, &self.inner.lua).map_err(crate::Error::backend)?;
                crate::Value::String(crate::LuaString::from(bytes))
            }
            Val::Table(_) => {
                let table =
                    Table::from_lua(value, &self.inner.lua).map_err(crate::Error::backend)?;
                crate::Value::Table(VmTable(
                    self.inner
                        .root_table(table)
                        .map_err(crate::Error::backend)?,
                ))
            }
            Val::Function(_) => {
                let function =
                    Function::from_lua(value, &self.inner.lua).map_err(crate::Error::backend)?;
                crate::Value::Function(VmFunction(
                    self.inner
                        .root_function(function)
                        .map_err(crate::Error::backend)?,
                ))
            }
            Val::LightUserdata(value) => crate::Value::LightUserData(value),
            value => {
                return Err(crate::Error::message(format!(
                    "unsupported browser Lua value type {}",
                    value.type_name()
                )));
            }
        })
    }
}

#[cfg(any(not(feature = "mlua-backend"), target_arch = "wasm32"))]
impl BrowserCallbackContext<'_> {
    pub fn app_data(&self) -> &AppData {
        &self.app_data
    }

    pub fn app_data_ref<T: Any>(&self) -> Option<Ref<'_, T>> {
        self.app_data.get::<T>()
    }

    pub fn app_data_mut<T: Any>(&self) -> Option<RefMut<'_, T>> {
        self.app_data.get_mut::<T>()
    }

    pub fn remove_app_data<T: Any>(&mut self) -> Option<T> {
        self.app_data.remove::<T>()
    }

    /// Live VM memory in bytes; usable inside callbacks through [`crate::LuaContext`].
    pub fn used_memory_bytes(&self) -> usize {
        self.state.gc.gc_state.total_bytes
    }

    /// The current call stack, innermost frame first, for `debugstack`.
    pub fn debug_frames(&self) -> Vec<crate::DebugFrame> {
        debug_frames_in_state(self.state)
    }

    /// Run a full collection cycle.
    pub fn collect_garbage(&mut self) -> crate::Result<()> {
        self.state.full_gc().map_err(crate::Error::backend)
    }

    pub fn set_named_registry_value(
        &mut self,
        name: &str,
        value: impl crate::IntoLua,
    ) -> crate::Result<()> {
        self.flush_released_roots()?;
        let value = self.to_raw_value(value.into_lua()?)?;
        set_named_registry_value_in_state(self.state, name, value).map_err(crate::Error::backend)
    }

    pub fn named_registry_value<T: crate::FromLua>(&mut self, name: &str) -> crate::Result<T> {
        self.flush_released_roots()?;
        let value =
            named_registry_value_in_state(self.state, name).map_err(crate::Error::backend)?;
        T::from_lua(self.from_raw_value(value)?)
    }

    pub fn remove_named_registry_value(&mut self, name: &str) -> crate::Result<()> {
        self.set_named_registry_value(name, crate::Value::Nil)
    }

    pub fn create_table(&mut self) -> crate::Result<VmTable> {
        self.flush_released_roots()?;
        let table = self.state.create_table();
        let root = self.root_value("table", Val::Table(table.gc_ref()))?;
        Ok(VmTable(BrowserTable { table, root }))
    }

    pub fn exec_named_text(&mut self, source: &[u8], name: &str) -> crate::Result<()> {
        self.flush_released_roots()?;
        reject_bytecode(source, name).map_err(crate::Error::backend)?;
        let function = self
            .state
            .load_bytes(source, name)
            .map_err(crate::Error::backend)?;
        call_function_in_state(self.state, function, &[])
            .map(|_| ())
            .map_err(crate::Error::backend)
    }

    pub fn load_function(&mut self, source: &[u8], name: &str) -> crate::Result<VmFunction> {
        self.flush_released_roots()?;
        reject_bytecode(source, name).map_err(crate::Error::backend)?;
        let function = self
            .state
            .load_bytes(source, name)
            .map_err(crate::Error::backend)?;
        let root = self.root_value("function", Val::Function(function.gc_ref()))?;
        Ok(VmFunction(BrowserFunction { function, root }))
    }

    pub fn global(&mut self, name: &str) -> crate::Result<crate::Value> {
        self.flush_released_roots()?;
        let value = self.state.get_global_val(name);
        self.from_raw_value(value)
    }

    pub fn set_global(&mut self, name: &str, value: impl Into<crate::Value>) -> crate::Result<()> {
        self.flush_released_roots()?;
        let value = self.to_raw_value(value.into())?;
        self.state
            .set_global_val(name, value)
            .map_err(crate::Error::backend)
    }

    pub fn table_get(
        &mut self,
        table: &VmTable,
        key: impl crate::IntoLua,
    ) -> crate::Result<crate::Value> {
        self.flush_released_roots()?;
        let table = self.checked_table(table)?;
        let key = self.to_raw_value(crate::IntoLua::into_lua(key)?)?;
        let value = self
            .state
            .gettable(Val::Table(table.gc_ref()), key)
            .map_err(crate::Error::backend)?;
        self.from_raw_value(value)
    }

    pub fn table_set(
        &mut self,
        table: &VmTable,
        key: impl crate::IntoLua,
        value: impl crate::IntoLua,
    ) -> crate::Result<()> {
        self.flush_released_roots()?;
        let table = self.checked_table(table)?;
        let key = self.to_raw_value(crate::IntoLua::into_lua(key)?)?;
        let value = self.to_raw_value(crate::IntoLua::into_lua(value)?)?;
        self.state
            .settable(Val::Table(table.gc_ref()), key, value)
            .map_err(crate::Error::backend)
    }

    pub fn table_get_raw(
        &mut self,
        table: &VmTable,
        key: impl crate::IntoLua,
    ) -> crate::Result<crate::Value> {
        self.flush_released_roots()?;
        let table = self.checked_table(table)?;
        let key = self.to_raw_value(crate::IntoLua::into_lua(key)?)?;
        let value = table
            .raw_get(self.state, key)
            .map_err(crate::Error::backend)?;
        self.from_raw_value(value)
    }

    pub fn table_set_raw(
        &mut self,
        table: &VmTable,
        key: impl crate::IntoLua,
        value: impl crate::IntoLua,
    ) -> crate::Result<()> {
        self.flush_released_roots()?;
        let table = self.checked_table(table)?;
        let key = self.to_raw_value(crate::IntoLua::into_lua(key)?)?;
        let value = self.to_raw_value(crate::IntoLua::into_lua(value)?)?;
        table
            .raw_set(self.state, key, value)
            .map_err(crate::Error::backend)
    }

    pub fn table_set_metatable(
        &mut self,
        table: &VmTable,
        metatable: Option<&VmTable>,
    ) -> crate::Result<()> {
        self.flush_released_roots()?;
        let table = self.checked_table(table)?;
        let metatable = metatable
            .map(|metatable| self.checked_table(metatable))
            .transpose()?;
        table
            .set_metatable(self.state, metatable)
            .map_err(crate::Error::backend)
    }

    pub fn table_metatable(&mut self, table: &VmTable) -> crate::Result<Option<VmTable>> {
        self.flush_released_roots()?;
        let table = self.checked_table(table)?;
        let Some(metatable) = table.metatable(self.state) else {
            return Ok(None);
        };
        let root = self.root_value("table", Val::Table(metatable.gc_ref()))?;
        Ok(Some(VmTable(BrowserTable {
            table: metatable,
            root,
        })))
    }

    pub fn table_raw_len(&mut self, table: &VmTable) -> crate::Result<usize> {
        self.flush_released_roots()?;
        let table = self.checked_table(table)?;
        usize::try_from(table.raw_len(self.state))
            .map_err(|_| crate::Error::runtime("Lua table length does not fit usize"))
    }

    pub fn table_entries(
        &mut self,
        table: &VmTable,
    ) -> crate::Result<Vec<(crate::Value, crate::Value)>> {
        self.flush_released_roots()?;
        let table = self.checked_table(table)?;
        let mut cursor = Val::Nil;
        let mut entries = Vec::new();
        while let Some((key, value)) = table
            .next(self.state, cursor)
            .map_err(crate::Error::backend)?
        {
            cursor = key;
            entries.push((self.from_raw_value(key)?, self.from_raw_value(value)?));
        }
        Ok(entries)
    }

    pub fn table_identity(&mut self, table: &VmTable) -> crate::Result<u64> {
        Ok(u64::from(self.checked_table(table)?.identity()))
    }

    pub fn function_source_location(&mut self, function: &VmFunction) -> Option<(String, usize)> {
        self.flush_released_roots().ok()?;
        let function = self.checked_function(function).ok()?;
        let (source, line) = function.source_location(self.state)?;
        Some((source, line as usize))
    }

    pub fn call_function(
        &mut self,
        function: &VmFunction,
        arguments: crate::MultiValue,
    ) -> crate::Result<crate::MultiValue> {
        self.flush_released_roots()?;
        let function = self.checked_function(function)?;
        let arguments = arguments
            .into_iter()
            .map(|value| self.to_raw_value(value))
            .collect::<crate::Result<Vec<_>>>()?;
        let values = call_function_in_state(self.state, function, &arguments)
            .map_err(crate::Error::backend)?;
        values
            .into_iter()
            .map(|value| self.from_raw_value(value))
            .collect()
    }

    pub fn table_call_method(
        &mut self,
        table: &VmTable,
        name: &str,
        mut arguments: crate::MultiValue,
    ) -> crate::Result<crate::MultiValue> {
        let value = self.table_get(table, name)?;
        let crate::Value::Function(function) = value else {
            return Err(crate::Error::type_mismatch("function", &value));
        };
        arguments.insert(0, crate::Value::Table(table.clone()));
        self.call_function(&function, arguments)
    }

    fn checked_table(&self, table: &VmTable) -> crate::Result<Table> {
        if table.0.root.vm_id != self.vm_id {
            return Err(crate::Error::message(
                "table belongs to another browser Lua VM",
            ));
        }
        Ok(table.0.table)
    }

    fn checked_function(&self, function: &VmFunction) -> crate::Result<Function> {
        if function.0.root.vm_id != self.vm_id {
            return Err(crate::Error::message(
                "function belongs to another browser Lua VM",
            ));
        }
        Ok(function.0.function)
    }

    fn flush_released_roots(&mut self) -> crate::Result<()> {
        self.root_slots
            .borrow_mut()
            .flush(self.state, self.roots)
            .map_err(crate::Error::backend)
    }

    fn root_value(&mut self, _kind: &str, value: Val) -> crate::Result<Rc<Root>> {
        self.flush_released_roots()?;
        let slot = self
            .root_slots
            .borrow_mut()
            .insert(self.state, self.roots, value)
            .map_err(crate::Error::backend)?;
        Ok(Rc::new(Root {
            vm_id: self.vm_id,
            slot,
            slots: Rc::clone(&self.root_slots),
        }))
    }

    fn to_raw_value(&mut self, value: crate::Value) -> crate::Result<Val> {
        Ok(match value {
            crate::Value::Nil => Val::Nil,
            crate::Value::Boolean(value) => Val::Bool(value),
            crate::Value::Integer(value) => Val::Num(value as f64),
            crate::Value::Number(value) => Val::Num(value),
            crate::Value::String(value) => value
                .as_bytes()
                .into_lua(self.state)
                .map_err(crate::Error::backend)?,
            crate::Value::Table(table) => {
                let table = self.checked_table(&table)?;
                Val::Table(table.gc_ref())
            }
            crate::Value::Function(function) => {
                let function = self.checked_function(&function)?;
                Val::Function(function.gc_ref())
            }
            crate::Value::LightUserData(value) => Val::LightUserdata(value),
        })
    }

    fn from_raw_value(&mut self, value: Val) -> crate::Result<crate::Value> {
        Ok(match value {
            Val::Nil => crate::Value::Nil,
            Val::Bool(value) => crate::Value::Boolean(value),
            Val::Num(value) => crate::Value::Number(value),
            Val::Str(_) => {
                let bytes =
                    Vec::<u8>::from_lua(value, &*self.state).map_err(crate::Error::backend)?;
                crate::Value::String(crate::LuaString::from(bytes))
            }
            Val::Table(_) => {
                let table = Table::from_lua(value, &*self.state).map_err(crate::Error::backend)?;
                let root = self.root_value("table", Val::Table(table.gc_ref()))?;
                crate::Value::Table(VmTable(BrowserTable { table, root }))
            }
            Val::Function(_) => {
                let function =
                    Function::from_lua(value, &*self.state).map_err(crate::Error::backend)?;
                let root = self.root_value("function", Val::Function(function.gc_ref()))?;
                crate::Value::Function(VmFunction(BrowserFunction { function, root }))
            }
            Val::LightUserdata(value) => crate::Value::LightUserData(value),
            value => {
                return Err(crate::Error::message(format!(
                    "unsupported browser Lua value type {}",
                    value.type_name()
                )));
            }
        })
    }
}

#[cfg(any(not(feature = "mlua-backend"), target_arch = "wasm32"))]
fn call_function_in_state(
    state: &mut LuaState,
    function: Function,
    arguments: &[Val],
) -> LuaResult<Vec<Val>> {
    let function_index = state.top;
    let saved_base = state.base;
    let saved_ci = state.ci;
    state.ensure_stack(1 + arguments.len());
    state.push(Val::Function(function.gc_ref()));
    for argument in arguments {
        state.push(*argument);
    }
    state.base = function_index + 1;

    let result = state.call_function(function_index, LUA_MULTRET);
    let values = result.map(|()| {
        (function_index..state.top)
            .map(|index| state.stack_get(index))
            .collect::<Vec<_>>()
    });
    state.top = function_index;
    state.base = saved_base;
    state.ci = saved_ci;
    values
}

/// Reject a binary chunk at a text-mode load door, mirroring mlua's
/// `ChunkMode::Text` on the native side: every benilla load door (chunk files,
/// `loadstring`, inline scripts) is text-only by sandbox contract, and the
/// reference never executes shipped bytecode. The scan mirrors rilua's own
/// `compile_or_undump` prelude (one `#` line skipped, then the signature), so
/// the two cannot disagree about what counts as binary.
// Un-gated: pure byte scan over rilua types (always a dependency), called from
// doors compiled under every feature/target combination.
fn reject_bytecode(source: &[u8], name: &str) -> rilua::LuaResult<()> {
    let data = if source.first() == Some(&b'#') {
        match source.iter().position(|&b| b == b'\n') {
            Some(pos) => &source[pos + 1..],
            None => &[],
        }
    } else {
        source
    };
    if data.starts_with(rilua::vm::dump::LUA_SIGNATURE) {
        return Err(rilua::error::LuaError::Syntax(rilua::error::SyntaxError {
            message: "attempt to load a binary chunk".to_owned(),
            source: name.to_owned(),
            line: 1,
            raw_message: None,
        }));
    }
    Ok(())
}

#[cfg(any(not(feature = "mlua-backend"), target_arch = "wasm32"))]
fn instruction_budget_error(used: u64) -> String {
    format!(
        "benilla: instruction budget exhausted after {used} VM instructions — treating this as a non-terminating loop"
    )
}

/// Read rilua's call stack into neutral frames, innermost first.
///
/// Level 0 is the running function itself; index 0 is the entry sentinel
/// and never reported — an idle VM answers empty, like mlua's walk past the
/// main chunk. The outermost Lua frame reads as the main chunk.
#[cfg(any(not(feature = "mlua-backend"), target_arch = "wasm32"))]
fn debug_frames_in_state(state: &LuaState) -> Vec<crate::DebugFrame> {
    use crate::{DebugFrame, DebugWhat};
    use rilua::vm::{callinfo::CallInfo, closure::Closure, value::Val};

    if state.call_stack.is_empty() {
        return Vec::new();
    }
    let top = state.ci.min(state.call_stack.len().saturating_sub(1));
    // The outermost live Lua frame is the main chunk.
    let mut main_idx = None;
    for (idx, ci) in state.call_stack.iter().enumerate().take(top + 1).skip(1) {
        if ci.is_lua {
            main_idx = Some(idx);
            break;
        }
    }
    let mut out = Vec::new();
    for idx in (1..=top).rev() {
        let ci: &CallInfo = &state.call_stack[idx];
        let frame = match state.stack_get(ci.func) {
            Val::Function(handle) => match state.gc.closures.get(handle) {
                Some(Closure::Lua(lcl)) => {
                    let proto = &lcl.proto;
                    DebugFrame {
                        source: Some(proto.source.clone()),
                        short_src: None,
                        what: if Some(idx) == main_idx {
                            DebugWhat::Main
                        } else {
                            DebugWhat::Lua
                        },
                        line_defined: Some(proto.line_defined as usize),
                        current_line: proto
                            .line_info
                            .get(ci.saved_pc)
                            .copied()
                            .map(|line| line as usize),
                        name: rilua::vm::debug_info::getfuncname(
                            state,
                            idx,
                            &state.gc.string_arena,
                        )
                        .map(|(_, name)| name),
                    }
                }
                _ => DebugFrame {
                    source: None,
                    short_src: Some("[C]".to_owned()),
                    what: DebugWhat::C,
                    line_defined: None,
                    current_line: None,
                    name: None,
                },
            },
            _ => DebugFrame {
                source: None,
                short_src: Some("[C]".to_owned()),
                what: DebugWhat::C,
                line_defined: None,
                current_line: None,
                name: None,
            },
        };
        out.push(frame);
    }
    out
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    #[test]
    fn table_handles_survive_gc_and_release_after_last_clone() {
        let mut vm = BrowserVm::new().expect("VM");
        let table = vm.create_table().expect("table");
        vm.table_set(&table, "count", 7_i64).expect("set");
        let clone = table.clone();
        vm.collect_garbage().expect("collect");
        assert_eq!(vm.table_get::<_, i64>(&clone, "count").ok(), Some(7));

        let slot = table.root.slot;
        drop(table);
        vm.collect_garbage().expect("collect while clone lives");
        assert_ne!(
            vm.roots
                .raw_get(vm.lua.state_mut(), Val::Num(f64::from(slot)))
                .ok(),
            Some(Val::Nil)
        );

        drop(clone);
        vm.collect_garbage().expect("collect after release");
        assert_eq!(
            vm.roots
                .raw_get(vm.lua.state_mut(), Val::Num(f64::from(slot)))
                .ok(),
            Some(Val::Nil)
        );
    }

    #[test]
    fn nested_tables_and_foreign_vm_are_handled_explicitly() {
        let mut vm = BrowserVm::new().expect("VM");
        let outer = vm.create_table().expect("outer");
        let inner = vm.create_table().expect("inner");
        vm.table_set(&inner, "visible", true).expect("inner value");
        vm.table_set_table(&outer, "child", &inner)
            .expect("nested table");
        let loaded = vm.table_get_table(&outer, "child").expect("read child");
        assert_eq!(vm.table_get::<_, bool>(&loaded, "visible").ok(), Some(true));

        let mut other = BrowserVm::new().expect("other VM");
        assert!(other.table_get::<_, bool>(&outer, "visible").is_err());
    }

    #[test]
    fn script_sees_the_same_rooted_table_as_the_host() {
        let mut vm = BrowserVm::new().expect("VM");
        let frame = vm.create_table().expect("frame");
        vm.table_set(&frame, "text", "before")
            .expect("initial text");
        vm.set_global_table("Frame", &frame).expect("publish frame");
        vm.exec_bytes(b"Frame.text = 'after'", "@web/frame.lua")
            .expect("run frame script");
        assert_eq!(
            vm.table_get::<_, String>(&frame, "text").ok(),
            Some("after".to_owned())
        );
        let from_global = vm.global_table("Frame").expect("read global frame");
        assert_eq!(
            vm.table_get::<_, String>(&from_global, "text").ok(),
            Some("after".to_owned())
        );
    }

    #[test]
    fn frame_method_dispatch_uses_index_metatable() {
        let mut vm = BrowserVm::new().expect("VM");
        let frame = vm.create_table().expect("frame");
        let methods = vm.create_table().expect("methods");
        let metatable = vm.create_table().expect("metatable");
        vm.table_set(&frame, "text", "from frame")
            .expect("frame field");
        vm.set_global_table("Methods", &methods)
            .expect("publish methods");
        vm.exec_bytes(
            b"function Methods:GetText() return self.text end",
            "@web/methods.lua",
        )
        .expect("define method");
        vm.table_set_table(&metatable, "__index", &methods)
            .expect("link methods");
        vm.table_set_metatable(&frame, Some(&metatable))
            .expect("install metatable");

        let text: String = vm
            .table_call_method(&frame, "GetText", ())
            .expect("call inherited method");
        assert_eq!(text, "from frame");

        vm.exec_bytes(
            b"function Methods:Sum9(a,b,c,d,e,f,g,h,i) return a+b+c+d+e+f+g+h+i end",
            "@web/methods.lua",
        )
        .expect("define nine-argument method");
        let total: f32 = vm
            .table_call_method(
                &frame,
                "Sum9",
                (
                    1.0_f32, 2.0_f32, 3.0_f32, 4.0_f32, 5.0_f32, 6.0_f32, 7.0_f32, 8.0_f32, 9.0_f32,
                ),
            )
            .expect("call nine-argument method");
        assert_eq!(total, 45.0);
    }

    #[test]
    fn compiled_handler_survives_gc_and_last_handle_releases_its_root() {
        let mut vm = BrowserVm::new().expect("VM");
        let handler = vm
            .load_function(
                b"Clicks = (Clicks or 0) + 1; return Clicks",
                "@Interface\\FrameXML\\Button.xml:OnClick",
            )
            .expect("compile handler");
        assert_eq!(vm.call_function::<_, i64>(&handler, ()).ok(), Some(1));
        let cloned = handler.clone();
        let slot = handler.root.slot;
        drop(handler);
        vm.collect_garbage().expect("collect with cloned handler");
        assert_eq!(vm.call_function::<_, i64>(&cloned, ()).ok(), Some(2));
        drop(cloned);
        vm.collect_garbage().expect("collect released handler");
        assert_eq!(
            vm.roots
                .raw_get(vm.lua.state_mut(), Val::Num(f64::from(slot)))
                .ok(),
            Some(Val::Nil)
        );
    }

    #[test]
    fn captured_host_handler_can_be_stored_on_a_frame() {
        let mut vm = BrowserVm::new().expect("VM");
        let calls = Rc::new(Cell::new(0_u32));
        let captured = Rc::clone(&calls);
        let callback = vm
            .create_host_function_raw("OnClick", move |state| {
                let next = captured.get() + 1;
                captured.set(next);
                state.push(Val::Num(f64::from(next)));
                Ok(1)
            })
            .expect("captured callback");
        let frame = vm.create_table().expect("frame");
        vm.table_set_function(&frame, "OnClick", &callback)
            .expect("store callback");
        drop(callback);
        vm.collect_garbage().expect("collect callback");
        let handler = vm
            .table_get_function(&frame, "OnClick")
            .expect("load callback");
        assert_eq!(vm.call_function::<_, i64>(&handler, ()).ok(), Some(1));
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn script_error_does_not_poison_the_next_handler_call() {
        let mut vm = BrowserVm::new().expect("VM");
        let broken = vm
            .load_function(b"error('expected')", "@web/broken.lua")
            .expect("broken handler compiles");
        assert!(vm.call_function::<_, ()>(&broken, ()).is_err());
        let working = vm
            .load_function(b"return 42", "@web/working.lua")
            .expect("working handler compiles");
        assert_eq!(vm.call_function::<_, i64>(&working, ()).ok(), Some(42));
    }

    #[test]
    fn typed_host_callback_converts_arguments_and_return_value() {
        let mut vm = BrowserVm::new().expect("VM");
        let callback = vm
            .create_host_function(
                "JoinLabels",
                |_state, (left, right): (String, String)| -> LuaResult<String> {
                    Ok(format!("{left} / {right}"))
                },
            )
            .expect("typed callback");
        vm.set_global_function("JoinLabels", &callback)
            .expect("publish callback");
        vm.exec_bytes(b"label = JoinLabels('Close', 'All')", "@web/callback.lua")
            .expect("invoke callback from Lua");
        assert_eq!(
            vm.global::<String>("label").ok(),
            Some("Close / All".to_owned())
        );
    }
}
