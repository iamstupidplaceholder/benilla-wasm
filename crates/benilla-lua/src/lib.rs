//! Platform-neutral Lua VM boundary for Benilla.
//!
//! Shared callers use [`Vm`], [`Table`], [`Function`] and [`Value`]. The default implementation is
//! rilua with the measured `wow_112` dialect on every target. Native builds can opt into the
//! `mlua-backend` feature for comparison with upstream behavior. Backend-specific VM and handle
//! types stay private to this crate.

use std::{cell::RefCell, rc::Rc};

use rilua::error::{LuaError, RuntimeError};
use rilua::vm::state::LuaState;
use rilua::{LuaApi, LuaApiMut, LuaResult, StdLib, Val};

use rilua::Lua;

mod browser_vm;
mod context;
mod conversion;
mod debug;
mod host_callback;
mod host_data;
mod host_registry;
#[cfg(all(feature = "mlua-backend", not(target_arch = "wasm32")))]
mod native_vm;
mod value;

#[cfg(any(not(feature = "mlua-backend"), target_arch = "wasm32"))]
pub use browser_vm::{
    BrowserCallbackContext as CallbackContext, Vm, VmFunction as Function, VmTable as Table,
};
pub use context::LuaContext;
pub use conversion::{FromLua, FromLuaMulti, IntoLua, IntoLuaMulti, Variadic};
pub use debug::{DebugFrame, DebugWhat};
pub use host_callback::{create_host_callback, register_host_callback};
pub use host_data::HostAppData;
pub use host_registry::{named_registry_value, set_named_registry_value};
#[cfg(all(feature = "mlua-backend", not(target_arch = "wasm32")))]
pub use native_vm::{
    with_native_context, NativeCallbackContext as CallbackContext, NativeFunction as Function,
    NativeTable as Table, NativeVm as Vm,
};
pub use value::{Error, LuaString, MultiValue, Result, Value};

#[cfg(all(feature = "mlua-backend", not(target_arch = "wasm32")))]
pub type AppDataRef<'a, T> = mlua_native::AppDataRef<'a, T>;
#[cfg(all(feature = "mlua-backend", not(target_arch = "wasm32")))]
pub type AppDataRefMut<'a, T> = mlua_native::AppDataRefMut<'a, T>;
#[cfg(any(not(feature = "mlua-backend"), target_arch = "wasm32"))]
pub type AppDataRef<'a, T> = std::cell::Ref<'a, T>;
#[cfg(any(not(feature = "mlua-backend"), target_arch = "wasm32"))]
pub type AppDataRefMut<'a, T> = std::cell::RefMut<'a, T>;

/// Typed Rust state shared by a VM and its host callbacks.
pub type AppData = HostAppData;

const BASELINE_CAPABILITIES: usize = 6;

/// The first real-client-script bridge. The VM stays alive between UI events;
/// only the asset transport and visual button live in `benilla-web`.
pub struct WebLuaSession {
    lua: Lua,
    close: String,
    all: String,
    frame_text: Option<Rc<RefCell<String>>>,
    dialog_visibility: Option<Rc<dyn DialogVisibility>>,
}

/// Host-owned visibility for a FrameXML dialog. The browser entry point can
/// back this with the production widget arena without making the Lua VM crate
/// depend on a particular UI model.
pub trait DialogVisibility: 'static {
    fn set_visible(&self, visible: bool) -> Result<(), String>;
    fn is_visible(&self) -> Result<bool, String>;
}

impl WebLuaSession {
    /// Execute the shipped GlobalStrings.lua bytes, then install one tiny Lua
    /// handler that reads two of its globals. No native UI API is faked here.
    pub fn from_global_strings(source: &[u8]) -> Result<Self, String> {
        let mut lua = Lua::new_with(StdLib::BASE | StdLib::STRING | StdLib::TABLE | StdLib::MATH)
            .map_err(|error| format!("create VM: {error}"))?;
        lua.exec_bytes(source, "@Interface\\FrameXML\\GlobalStrings.lua")
            .map_err(|error| format!("execute GlobalStrings.lua: {error}"))?;
        let close: String = lua
            .global("CLOSE")
            .map_err(|error| format!("read CLOSE: {error}"))?;
        let all: String = lua
            .global("ALL")
            .map_err(|error| format!("read ALL: {error}"))?;
        if close.is_empty() || all.is_empty() {
            return Err("GlobalStrings.lua produced an empty CLOSE or ALL".to_owned());
        }
        lua.exec(
            r#"
                BenillaWebProbeClicks = 0
                function BenillaWebProbe_OnClick()
                    BenillaWebProbeClicks = BenillaWebProbeClicks + 1
                    return CLOSE .. " / " .. ALL .. " #" .. tostring(BenillaWebProbeClicks)
                end
            "#,
        )
        .map_err(|error| format!("install web handler: {error}"))?;
        Ok(Self {
            lua,
            close,
            all,
            frame_text: None,
            dialog_visibility: None,
        })
    }

    pub fn shipped_labels(&self) -> (&str, &str) {
        (&self.close, &self.all)
    }

    /// Invoke the same Lua handler on each click; its counter proves state survives.
    pub fn click(&mut self) -> Result<String, String> {
        self.lua
            .exec("benilla_web_probe_label = BenillaWebProbe_OnClick()")
            .map_err(|error| format!("run web handler: {error}"))?;
        self.lua
            .global("benilla_web_probe_label")
            .map_err(|error| format!("read web handler result: {error}"))
    }

    /// Bind one FrameXML-style frame to a Rust-owned text sink and run its
    /// `OnLoad`. `OnClick` is compiled once into the persistent VM.
    pub fn install_frame_script(
        &mut self,
        name: &str,
        on_load: &str,
        on_click: &str,
    ) -> Result<String, String> {
        let text = Rc::new(RefCell::new(String::new()));
        let sink = self.lua.create_userdata(Rc::clone(&text));
        self.lua
            .set_global("BenillaHostFrameTextSink", sink)
            .map_err(|error| format!("register frame text sink: {error}"))?;
        self.lua
            .register_function("BenillaHostSetFrameText", host_set_frame_text)
            .map_err(|error| format!("register SetText binding: {error}"))?;
        self.lua
            .set_global("BenillaHostFrameName", name.to_owned())
            .map_err(|error| format!("register frame name: {error}"))?;
        self.lua
            .exec(
                r#"
                    BenillaL3Frame = { name = BenillaHostFrameName }
                    function BenillaL3Frame:GetName() return self.name end
                    function BenillaL3Frame:SetText(value) BenillaHostSetFrameText(value) end
                    this = BenillaL3Frame
                "#,
            )
            .map_err(|error| format!("create frame object: {error}"))?;
        self.lua
            .exec_bytes(on_load.as_bytes(), "@web/l3-frame.xml:OnLoad")
            .map_err(|error| format!("run FrameXML OnLoad: {error}"))?;
        let on_click_chunk = format!("function BenillaL3Frame_OnClick()\n{on_click}\nend");
        self.lua
            .exec_bytes(on_click_chunk.as_bytes(), "@web/l3-frame.xml:OnClick")
            .map_err(|error| format!("compile FrameXML OnClick: {error}"))?;
        let initial_text = text.borrow().clone();
        if initial_text.is_empty() {
            return Err("FrameXML OnLoad did not set the host frame text".to_owned());
        }
        self.frame_text = Some(text);
        Ok(initial_text)
    }

    /// Dispatch a browser button press to the compiled FrameXML handler and
    /// return the Rust-owned text that the handler wrote through `SetText`.
    pub fn frame_click(&mut self) -> Result<String, String> {
        let Some(text) = &self.frame_text else {
            return Err("FrameXML frame has not been installed".to_owned());
        };
        self.lua
            .exec("this = BenillaL3Frame; BenillaL3Frame_OnClick()")
            .map_err(|error| format!("run FrameXML OnClick: {error}"))?;
        let value = text.borrow().clone();
        Ok(value)
    }

    /// Bind the shipped BasicControls.xml dialog-button handler. The parent
    /// and button are separate Lua objects; `Hide` crosses into Rust state.
    pub fn install_dialog_script(
        &mut self,
        parent_name: &str,
        button_name: &str,
        label_key: &str,
        on_click: &str,
        visibility: Rc<dyn DialogVisibility>,
    ) -> Result<String, String> {
        let label: String = self
            .lua
            .global(label_key)
            .map_err(|error| format!("read shipped {label_key} label: {error}"))?;
        let sink = self.lua.create_userdata(Rc::clone(&visibility));
        self.lua
            .set_global("BenillaHostDialogVisibilitySink", sink)
            .map_err(|error| format!("register dialog visibility sink: {error}"))?;
        self.lua
            .register_function("BenillaHostHideDialog", host_hide_dialog)
            .map_err(|error| format!("register Hide binding: {error}"))?;
        self.lua
            .set_global("BenillaHostDialogName", parent_name.to_owned())
            .map_err(|error| format!("register dialog name: {error}"))?;
        self.lua
            .set_global("BenillaHostDialogButtonName", button_name.to_owned())
            .map_err(|error| format!("register dialog button name: {error}"))?;
        self.lua
            .exec(
                r#"
                    BenillaL4Dialog = { name = BenillaHostDialogName }
                    function BenillaL4Dialog:Hide() BenillaHostHideDialog() end
                    BenillaL4Button = {
                        name = BenillaHostDialogButtonName,
                        parent = BenillaL4Dialog
                    }
                    function BenillaL4Button:GetParent() return self.parent end
                "#,
            )
            .map_err(|error| format!("create dialog frame objects: {error}"))?;
        let chunk = format!("function BenillaL4Button_OnClick()\n{on_click}\nend");
        self.lua
            .exec_bytes(
                chunk.as_bytes(),
                "@Interface\\FrameXML\\BasicControls.xml:OnClick",
            )
            .map_err(|error| format!("compile shipped dialog OnClick: {error}"))?;
        self.dialog_visibility = Some(visibility);
        Ok(label)
    }

    pub fn dialog_click(&mut self) -> Result<bool, String> {
        let Some(visibility) = &self.dialog_visibility else {
            return Err("shipped dialog has not been installed".to_owned());
        };
        self.lua
            .exec("this = BenillaL4Button; BenillaL4Button_OnClick()")
            .map_err(|error| format!("run shipped dialog OnClick: {error}"))?;
        visibility.is_visible()
    }

    pub fn show_dialog(&mut self) -> Result<(), String> {
        let Some(visibility) = &self.dialog_visibility else {
            return Err("shipped dialog has not been installed".to_owned());
        };
        visibility.set_visible(true)
    }
}

fn host_set_frame_text(state: &mut LuaState) -> LuaResult<u32> {
    let value = String::from_utf8_lossy(&state.check_string(1)?).into_owned();
    let sink = state.get_global_val("BenillaHostFrameTextSink");
    let Val::Userdata(handle) = sink else {
        return Err(frame_bridge_error("FrameXML text sink is missing"));
    };
    let Some(text) = state
        .gc
        .userdata
        .get(handle)
        .and_then(|userdata| userdata.downcast_ref::<Rc<RefCell<String>>>())
    else {
        return Err(frame_bridge_error("FrameXML text sink has the wrong type"));
    };
    *text.borrow_mut() = value;
    Ok(0)
}

fn host_hide_dialog(state: &mut LuaState) -> LuaResult<u32> {
    let sink = state.get_global_val("BenillaHostDialogVisibilitySink");
    let Val::Userdata(handle) = sink else {
        return Err(frame_bridge_error("dialog visibility sink is missing"));
    };
    let Some(visibility) = state
        .gc
        .userdata
        .get(handle)
        .and_then(|userdata| userdata.downcast_ref::<Rc<dyn DialogVisibility>>())
    else {
        return Err(frame_bridge_error(
            "dialog visibility sink has the wrong type",
        ));
    };
    visibility
        .set_visible(false)
        .map_err(|error| frame_bridge_error(&error))?;
    Ok(0)
}

fn frame_bridge_error(message: &str) -> LuaError {
    LuaError::Runtime(RuntimeError {
        message: message.to_owned(),
        level: 0,
        traceback: Vec::new(),
    })
}

#[cfg(test)]
mod web_session_tests {
    use std::{cell::Cell, rc::Rc};

    use super::{DialogVisibility, WebLuaSession};

    struct TestVisibility(Rc<Cell<bool>>);

    impl DialogVisibility for TestVisibility {
        fn set_visible(&self, visible: bool) -> Result<(), String> {
            self.0.set(visible);
            Ok(())
        }

        fn is_visible(&self) -> Result<bool, String> {
            Ok(self.0.get())
        }
    }

    #[test]
    fn shipped_hide_handler_updates_host_visibility() {
        let mut session =
            WebLuaSession::from_global_strings(b"CLOSE = 'Close'; ALL = 'All'; OKAY = 'Okay'")
                .expect("Lua session");
        let state = Rc::new(Cell::new(true));
        let visibility: Rc<dyn DialogVisibility> = Rc::new(TestVisibility(Rc::clone(&state)));
        let label = session
            .install_dialog_script(
                "ScriptErrors",
                "ScriptErrorsButton",
                "OKAY",
                "this:GetParent():Hide()",
                visibility,
            )
            .expect("install handler");
        assert_eq!(label, "Okay");
        assert!(!session.dialog_click().expect("click"));
        assert!(!state.get());
        session.show_dialog().expect("show");
        assert!(state.get());
    }
}

/// One browser/host run of the pure-Rust VM and the measured WoW 1.12.1 dialect differences.
#[derive(Debug)]
pub struct L1Report {
    pub baseline_passed: usize,
    pub baseline_total: usize,
    pub baseline_error: Option<String>,
    pub vanilla_passed: usize,
    pub vanilla_total: usize,
    pub vanilla_gaps: Vec<String>,
    pub gc_bytes: usize,
}

/// Run the isolated L1 compatibility spike. It never panics and does not retain a VM afterward.
pub fn run_l1_probe() -> L1Report {
    let (baseline_passed, baseline_error, gc_bytes) = match run_baseline() {
        Ok(gc_bytes) => (BASELINE_CAPABILITIES, None, gc_bytes),
        Err(error) => (0, Some(short_error(&error)), 0),
    };

    let checks = vanilla_checks();
    let vanilla_total = checks.len();
    let mut vanilla_passed = 0;
    let mut vanilla_gaps = Vec::new();
    for (name, result) in checks {
        match result {
            Ok(true) => vanilla_passed += 1,
            Ok(false) => vanilla_gaps.push(name.to_owned()),
            Err(error) => vanilla_gaps.push(format!("{name}: {}", short_error(&error))),
        }
    }

    L1Report {
        baseline_passed,
        baseline_total: BASELINE_CAPABILITIES,
        baseline_error,
        vanilla_passed,
        vanilla_total,
        vanilla_gaps,
        gc_bytes,
    }
}

fn host_add(state: &mut LuaState) -> LuaResult<u32> {
    let left = state.check_number(1)?;
    let right = state.check_number(2)?;
    state.push_number(left + right);
    Ok(1)
}

fn run_baseline() -> Result<usize, String> {
    let mut lua = Lua::new_with(StdLib::BASE | StdLib::STRING | StdLib::TABLE | StdLib::MATH)
        .map_err(|error| format!("VM/std-lib creation: {error}"))?;

    // 1: source compilation/execution; 2: tables/functions/multiple returns; 3: Rust callback.
    lua.register_function("host_add", host_add)
        .map_err(|error| format!("Rust callback registration: {error}"))?;
    lua.exec(
        r#"
            local function pair(value)
                return value, value * 2
            end
            local first, second = pair(21)
            local probe = { answer = host_add(first, second), label = string.upper("benilla") }
            l1_answer = probe.answer
            l1_label = probe.label
        "#,
    )
    .map_err(|error| format!("table/function/callback chunk: {error}"))?;
    let answer: f64 = lua
        .global("l1_answer")
        .map_err(|error| format!("number global conversion: {error}"))?;
    let label: String = lua
        .global("l1_label")
        .map_err(|error| format!("string global conversion: {error}"))?;
    if answer != 63.0 || label != "BENILLA" {
        return Err(format!(
            "table/function/callback values differ: answer={answer}, label={label:?}"
        ));
    }

    // 4: a host-retained function handle and all returned values.
    let function = lua
        .load("return 4, 5, 9")
        .map_err(|error| format!("function handle load: {error}"))?;
    let values = lua
        .call_function(&function, &[])
        .map_err(|error| format!("function handle call: {error}"))?;
    if values != [Val::Num(4.0), Val::Num(5.0), Val::Num(9.0)] {
        return Err(format!("multiple returns differ: {values:?}"));
    }

    // 5: opaque Rust userdata can cross into Lua and remain borrowable by the host.
    let userdata = lua.create_userdata(41_u32);
    let recovered = lua.borrow_userdata::<u32>(&userdata).copied();
    if recovered != Some(41) {
        return Err(format!("userdata round-trip differs: {recovered:?}"));
    }
    lua.set_global("l1_userdata", userdata)
        .map_err(|error| format!("userdata global conversion: {error}"))?;
    lua.exec("l1_userdata_ok = type(l1_userdata) == 'userdata'")
        .map_err(|error| format!("userdata Lua inspection: {error}"))?;
    let userdata_ok: bool = lua
        .global("l1_userdata_ok")
        .map_err(|error| format!("userdata result conversion: {error}"))?;
    if !userdata_ok {
        return Err("Lua did not observe Rust userdata".to_owned());
    }

    // 6: runtime errors return through Rust instead of trapping the WASM instance.
    let mut error_lua =
        Lua::new_with(StdLib::BASE).map_err(|error| format!("error-probe VM creation: {error}"))?;
    let error = match error_lua.exec("local function inner() error('l1-sentinel') end inner()") {
        Ok(()) => return Err("the explicit Lua error did not propagate".to_owned()),
        Err(error) => error,
    };
    if !error.to_string().contains("l1-sentinel") {
        return Err(format!("runtime error lost its payload: {error}"));
    }

    Ok(lua.gc_count())
}

type DialectResult = Result<bool, String>;

fn vanilla_checks() -> Vec<(&'static str, DialectResult)> {
    vec![
        (
            "bare-table-generic-for",
            exec_bool(
                r#"
                    local total = 0
                    for _, value in { one = 1, two = 2, three = 3 } do
                        total = total + value
                    end
                    l1_ok = total == 6
                "#,
            ),
        ),
        (
            "nested-long-string",
            exec_bool(
                r#"
                    local value = [[outer [[inner]] outer]]
                    l1_ok = value == "outer [[inner]] outer"
                "#,
            ),
        ),
        (
            "constructor-double-semicolon",
            exec_bool(
                "local value = { first = 1;; second = 2 }; l1_ok = value.first + value.second == 3",
            ),
        ),
        (
            "computed-field-next-order",
            exec_bool_with_observation(
                r#"
                    local value = { [3] = "c", [1] = "a", [2] = "b" }
                    local key, order = nil, ""
                    repeat
                        key = next(value, key)
                        if key then order = order .. tostring(key) end
                    until key == nil
                    l1_ok = order == "123"
                    l1_observed = order
                "#,
            ),
        ),
        (
            "reject-vararg-expression",
            rejects_source("return function(...) return ... end"),
        ),
        ("reject-length-operator", rejects_source("return #{}")),
        ("reject-modulo-operator", rejects_source("return 5 % 2")),
        (
            "client-error-quotes",
            exec_bool_with_observation(
                r#"
                    local ok, message = pcall(function() return l1_missing + 1 end)
                    l1_ok = not ok and string.find(message, "`l1_missing'", 1, true) ~= nil
                    l1_observed = message
                "#,
            ),
        ),
    ]
}

fn exec_bool(source: &str) -> DialectResult {
    let mut lua = Lua::new_with(StdLib::BASE | StdLib::STRING | StdLib::TABLE | StdLib::MATH)
        .map_err(|error| error.to_string())?;
    lua.exec(source).map_err(|error| error.to_string())?;
    lua.global("l1_ok").map_err(|error| error.to_string())
}

fn exec_bool_with_observation(source: &str) -> DialectResult {
    let mut lua = Lua::new_with(StdLib::BASE | StdLib::STRING | StdLib::TABLE | StdLib::MATH)
        .map_err(|error| error.to_string())?;
    lua.exec(source).map_err(|error| error.to_string())?;
    let passed: bool = lua.global("l1_ok").map_err(|error| error.to_string())?;
    if passed {
        Ok(true)
    } else {
        let observed: String = lua
            .global("l1_observed")
            .map_err(|error| error.to_string())?;
        Err(format!("observed {observed:?}"))
    }
}

fn rejects_source(source: &str) -> DialectResult {
    let mut lua = Lua::new_empty();
    Ok(lua.load(source).is_err())
}

fn short_error(error: &str) -> String {
    const LIMIT: usize = 180;
    let first_line = error.lines().next().unwrap_or(error);
    if first_line.chars().count() <= LIMIT {
        first_line.to_owned()
    } else {
        let mut shortened: String = first_line.chars().take(LIMIT).collect();
        shortened.push('…');
        shortened
    }
}

#[cfg(test)]
mod neutral_vm_tests {
    use super::{LuaContext, LuaString, Result, Table, Value, Vm};

    fn integer(value: Value) -> i64 {
        match value {
            Value::Integer(value) => value,
            Value::Number(value) => value as i64,
            value => panic!("expected number, got {}", value.type_name()),
        }
    }

    fn exercise_active_context(context: &mut impl LuaContext, table: &Table) -> Result<i64> {
        let previous: i64 = context.table_get(table, "count")?;
        context.table_set(table, "count", previous + 1)?;
        context.put_global("context_seen", true)?;
        if let Some(mut calls) = context.app_data_mut::<u32>() {
            *calls += 1;
        }
        Ok(previous)
    }

    #[test]
    fn tables_methods_and_multiple_returns_use_the_common_surface() {
        let mut vm = Vm::new().expect("VM");
        let frame = vm.create_table().expect("frame");
        let methods = vm.create_table().expect("methods");
        let metatable = vm.create_table().expect("metatable");
        vm.table_set(&frame, "value", 7_i64).expect("field");
        vm.set_global_table("Methods", &methods).expect("methods");
        vm.exec_named_text(
            b"function Methods:Pair(extra) return self.value, self.value + extra end",
            "@benilla/common-vm.lua",
        )
        .expect("method");
        vm.table_set_table(&metatable, "__index", &methods)
            .expect("index");
        vm.table_set_metatable(&frame, Some(&metatable))
            .expect("metatable");

        let values = vm
            .table_call_method(&frame, "Pair", vec![Value::Integer(5)])
            .expect("call");
        assert_eq!(values.len(), 2);
        assert_eq!(integer(values[0].clone()), 7);
        assert_eq!(integer(values[1].clone()), 12);
    }

    #[test]
    fn byte_strings_and_handles_survive_collection() {
        let mut vm = Vm::new().expect("VM");
        let bytes = LuaString::from(&b"a\0\xffz"[..]);
        vm.set_global("bytes", Value::String(bytes.clone()))
            .expect("set bytes");
        let table = vm.create_table().expect("table");
        vm.table_set(&table, "bytes", Value::String(bytes.clone()))
            .expect("table bytes");
        vm.set_global_table("Rooted", &table).expect("root table");
        vm.collect_garbage().expect("collect");

        let Value::String(reloaded) = vm.global("bytes").expect("read bytes") else {
            panic!("bytes global changed type");
        };
        assert_eq!(reloaded.as_bytes(), bytes.as_bytes());
        let rooted = vm.global_table("Rooted").expect("rooted table");
        let Value::String(reloaded) = vm.table_get(&rooted, "bytes").expect("table bytes") else {
            panic!("table bytes changed type");
        };
        assert_eq!(reloaded.as_bytes(), bytes.as_bytes());
    }

    #[test]
    fn foreign_handles_and_failed_calls_do_not_poison_the_vm() {
        let mut first = Vm::new().expect("first VM");
        let table = first.create_table().expect("table");
        let mut second = Vm::new().expect("second VM");
        assert!(second.set_global_table("Foreign", &table).is_err());

        let broken = first
            .load_function(b"error('expected common VM failure')", "@broken.lua")
            .expect("broken function");
        assert!(first.call_function(&broken, Vec::new()).is_err());
        let working = first
            .load_function(b"return 42", "@working.lua")
            .expect("working function");
        let values = first
            .call_function(&working, Vec::new())
            .expect("working call");
        assert_eq!(integer(values[0].clone()), 42);
    }

    #[test]
    fn captured_callback_uses_shared_app_data_and_active_vm_context() {
        let mut vm = Vm::new().expect("VM");
        vm.set_app_data(0_u32);
        vm.exec_named_text(
            b"function BenillaTestDouble(value) return value * 2 end",
            "@double.lua",
        )
        .expect("define double function");
        let double = vm
            .global_function("BenillaTestDouble")
            .expect("double function");
        let callback = vm
            .create_host_function("HostDouble", move |context, arguments| {
                let mut calls = context.app_data_mut::<u32>().expect("counter");
                *calls += 1;
                drop(calls);
                context.call_function(&double, arguments)
            })
            .expect("host callback");
        vm.set_global_function("HostDouble", &callback)
            .expect("publish callback");
        vm.exec_named_text(b"answer = HostDouble(21)", "@callback.lua")
            .expect("invoke callback");

        assert_eq!(integer(vm.global("answer").expect("answer")), 42);
        assert_eq!(*vm.app_data_ref::<u32>().expect("counter"), 1);
    }

    #[test]
    fn owned_vm_and_callback_share_the_production_context_contract() {
        let mut vm = Vm::new().expect("VM");
        vm.set_app_data(0_u32);
        let table = vm.create_table().expect("table");
        LuaContext::table_set(&mut vm, &table, 1_i64, "first").expect("array field");
        LuaContext::table_set(&mut vm, &table, "count", 1_i64).expect("count");

        let metatable = vm.create_table().expect("metatable");
        LuaContext::table_set(&mut vm, &metatable, "marker", 42_i64).expect("marker");
        LuaContext::table_set_metatable(&mut vm, &table, Some(&metatable)).expect("metatable");
        assert_eq!(LuaContext::table_raw_len(&mut vm, &table).unwrap(), 1);
        let table_id = LuaContext::table_identity(&mut vm, &table).expect("identity");
        assert_eq!(
            LuaContext::table_identity(&mut vm, &table).expect("stable identity"),
            table_id
        );
        let installed = LuaContext::table_metatable(&mut vm, &table)
            .expect("read metatable")
            .expect("installed metatable");
        assert_eq!(
            LuaContext::table_get::<_, i64>(&mut vm, &installed, "marker").unwrap(),
            42
        );
        assert!(
            LuaContext::table_entries(&mut vm, &table)
                .expect("entries")
                .len()
                >= 2
        );

        assert_eq!(exercise_active_context(&mut vm, &table).unwrap(), 1);
        let callback_table = table.clone();
        let callback = vm
            .create_function("ContextIncrement", move |context, (): ()| {
                exercise_active_context(context, &callback_table)
            })
            .expect("callback");
        vm.set_global_function("ContextIncrement", &callback)
            .expect("publish callback");
        vm.exec_named_text(b"previous = ContextIncrement()", "@context.lua")
            .expect("invoke callback");

        assert_eq!(integer(vm.global("previous").expect("previous")), 2);
        assert_eq!(
            LuaContext::table_get::<_, i64>(&mut vm, &table, "count").unwrap(),
            3
        );
        assert!(LuaContext::get_global::<bool>(&mut vm, "context_seen").unwrap());
        assert_eq!(*LuaContext::app_data_ref::<u32>(&vm).unwrap(), 2);
    }

    #[cfg(all(feature = "mlua-backend", not(target_arch = "wasm32")))]
    #[test]
    fn adopted_native_state_has_one_registry_globals_and_app_data_store() {
        let raw = mlua_native::Lua::new();
        raw.set_app_data(String::from("shared-model"));
        raw.globals().set("raw_side", 20_i64).unwrap();

        let mut vm = Vm::from_native_lua(raw);
        assert_eq!(
            vm.app_data_ref::<String>().as_deref().map(String::as_str),
            Some("shared-model")
        );
        assert_eq!(integer(vm.global("raw_side").unwrap()), 20);

        vm.set_global("neutral_side", 22_i64).unwrap();
        let value: i64 = vm.native_lua().globals().get("neutral_side").unwrap();
        assert_eq!(value, 22);

        vm.set_named_registry_value("shared_registry", 42_i64)
            .unwrap();
        let value: i64 = vm
            .native_lua()
            .named_registry_value("shared_registry")
            .unwrap();
        assert_eq!(value, 42);

        crate::with_native_context(vm.native_lua(), |context| {
            context.set_global("borrowed_side", 24_i64)?;
            context.set_named_registry_value("borrowed_registry", 44_i64)
        })
        .unwrap();
        assert_eq!(integer(vm.global("borrowed_side").unwrap()), 24);
        assert_eq!(
            vm.named_registry_value::<i64>("borrowed_registry").unwrap(),
            44
        );
    }

    #[test]
    fn typed_callbacks_and_named_registry_share_the_common_surface() {
        let mut vm = Vm::new().expect("VM");
        vm.set_global("object_identity", Value::LightUserData(0x12_345))
            .expect("light userdata identity");
        let Value::LightUserData(identity) = vm.global("object_identity").expect("identity") else {
            panic!("object identity changed type");
        };
        assert_eq!(identity, 0x12_345);

        let methods = vm.create_table().expect("methods table");
        vm.table_set(&methods, "marker", 17_i64).expect("marker");
        vm.set_named_registry_value("test_methods", &methods)
            .expect("root methods");
        drop(methods);
        vm.collect_garbage().expect("collect");

        let methods = vm
            .named_registry_value::<super::Table>("test_methods")
            .expect("rooted methods");
        assert_eq!(
            integer(vm.table_get(&methods, "marker").expect("marker")),
            17
        );

        let typed = vm
            .create_function(
                "TypedAdd",
                |_context, (left, right, rest): (i64, Option<i64>, super::MultiValue)| {
                    Ok((left + right.unwrap_or_default(), rest.len() as i64))
                },
            )
            .expect("typed callback");
        vm.set_global_function("TypedAdd", &typed)
            .expect("publish typed callback");
        vm.exec_named_text(
            b"sum, rest_count = TypedAdd('20', 22, 'tail', true)",
            "@typed-callback.lua",
        )
        .expect("invoke typed callback");
        assert_eq!(integer(vm.global("sum").expect("sum")), 42);
        assert_eq!(integer(vm.global("rest_count").expect("rest count")), 2);

        vm.remove_named_registry_value("test_methods")
            .expect("remove methods");
        let missing = vm
            .named_registry_value::<Option<super::Table>>("test_methods")
            .expect("missing registry value");
        assert!(missing.is_none());

        vm.set_app_data(String::from("temporary"));
        assert_eq!(vm.remove_app_data::<String>().as_deref(), Some("temporary"));
        assert!(vm.app_data_ref::<String>().is_none());
    }

    #[test]
    fn traced_errors_and_instruction_budget_leave_the_vm_usable() {
        let mut vm = Vm::new().expect("VM");
        let error = vm
            .exec_named_text(
                b"local function boom() error('named failure') end boom()",
                "@Interface\\AddOns\\Broken\\Broken.lua",
            )
            .expect_err("chunk must fail");
        assert!(error.as_str().contains("named failure"), "{error}");
        assert!(error.as_str().contains("Broken.lua"), "{error}");

        vm.exec_named_text(b"after_error = 2 + 3", "@after-error.lua")
            .expect("VM recovers after traced error");
        assert_eq!(integer(vm.global("after_error").expect("recovery")), 5);

        vm.set_instruction_budget(1_000, 5_000)
            .expect("install instruction budget");
        let error = vm
            .exec_named_text(b"local i = 0 while true do i = i + 1 end", "@runaway.lua")
            .expect_err("runaway chunk must stop");
        assert!(
            error.as_str().contains("instruction budget exhausted"),
            "{error}"
        );
        assert!(vm.instructions_used() >= 5_000);

        vm.exec_named_text(b"after_budget = 6 * 7", "@after-budget.lua")
            .expect("VM recovers after budget error");
        assert_eq!(integer(vm.global("after_budget").expect("recovery")), 42);
        vm.clear_instruction_budget();
    }

    #[test]
    fn debug_frames_name_frames_sources_and_the_main_chunk() {
        use super::{DebugFrame, DebugWhat};
        use crate::LuaContext;

        let mut vm = Vm::new().expect("VM");
        // A named Lua function calling back into the host: the walk must see
        // the host frame innermost, then the named frame with its chunk, then
        // the main chunk at the bottom.
        let seen = std::rc::Rc::new(std::cell::Cell::new(Vec::new()));
        let probe = std::rc::Rc::clone(&seen);
        let callback = vm
            .create_function("BenillaDebugProbe", move |context, ()| {
                let frames: Vec<DebugFrame> = context.debug_frames();
                let summary: Vec<(String, String)> = frames
                    .iter()
                    .map(|frame| {
                        let what = match frame.what {
                            DebugWhat::Main => "main",
                            DebugWhat::Lua => "lua",
                            DebugWhat::C => "c",
                        };
                        (
                            what.to_owned(),
                            frame
                                .name
                                .clone()
                                .or_else(|| {
                                    frame.source.clone().map(|source| {
                                        source
                                            .rsplit(['/', '\\'])
                                            .next()
                                            .unwrap_or(&source)
                                            .to_owned()
                                    })
                                })
                                .unwrap_or_default(),
                        )
                    })
                    .collect();
                probe.set(summary);
                Ok(())
            })
            .expect("debug probe");
        vm.set_global_function("BenillaDebugProbe", &callback)
            .expect("publish probe");
        vm.exec_named_text(
            b"function BenillaDebugTarget() return BenillaDebugProbe() end BenillaDebugTarget()",
            "@Interface\\AddOns\\Probe\\Probe.lua",
        )
        .expect("run probe");
        let seen = seen.take();
        assert!(seen.len() >= 3, "host, named and main frames: {seen:?}");
        assert_eq!(seen[0].0.as_str(), "c", "{seen:?}");
        assert_eq!(
            seen[1],
            ("lua".to_owned(), "BenillaDebugTarget".to_owned()),
            "{seen:?}"
        );
        let last = seen.last().expect("main frame");
        assert_eq!(last.0.as_str(), "main", "{seen:?}");
        assert!(
            last.1.contains("Probe.lua"),
            "main chunk keeps its source: {seen:?}"
        );
    }
}
