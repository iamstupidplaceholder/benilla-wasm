//! Backend-neutral conversions at the Lua/host boundary.
//!
//! These traits intentionally cover the shapes used by Benilla bindings rather than mirroring
//! every conversion exposed by either backend.

use std::ops::{Deref, DerefMut};

use crate::{Error, Function, LuaString, MultiValue, Result, Table, Value};

pub trait FromLua: Sized {
    fn from_lua(value: Value) -> Result<Self>;
}

pub trait IntoLua {
    fn into_lua(self) -> Result<Value>;
}

pub trait FromLuaMulti: Sized {
    fn from_lua_multi(values: MultiValue) -> Result<Self>;
}

pub trait IntoLuaMulti {
    fn into_lua_multi(self) -> Result<MultiValue>;
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Variadic<T>(pub Vec<T>);

impl<T> Variadic<T> {
    pub fn into_inner(self) -> Vec<T> {
        self.0
    }
}

impl<T> From<Vec<T>> for Variadic<T> {
    fn from(values: Vec<T>) -> Self {
        Self(values)
    }
}

impl<T> Deref for Variadic<T> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> DerefMut for Variadic<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<T> IntoIterator for Variadic<T> {
    type Item = T;
    type IntoIter = std::vec::IntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl IntoLua for Value {
    fn into_lua(self) -> Result<Value> {
        Ok(self)
    }
}

macro_rules! infallible_into_lua {
    ($($ty:ty),* $(,)?) => {
        $(
            impl IntoLua for $ty {
                fn into_lua(self) -> Result<Value> {
                    Ok(Value::from(self))
                }
            }
        )*
    };
}

infallible_into_lua!(
    bool,
    i8,
    i16,
    i32,
    i64,
    u8,
    u16,
    u32,
    f32,
    f64,
    LuaString,
    String,
    Vec<u8>,
    Table,
    Function,
);

impl IntoLua for &Value {
    fn into_lua(self) -> Result<Value> {
        Ok(self.clone())
    }
}

impl IntoLua for &str {
    fn into_lua(self) -> Result<Value> {
        Ok(Value::from(self))
    }
}

impl IntoLua for &[u8] {
    fn into_lua(self) -> Result<Value> {
        Ok(Value::from(self))
    }
}

impl IntoLua for usize {
    fn into_lua(self) -> Result<Value> {
        i64::try_from(self)
            .map(Value::Integer)
            .map_err(|_| Error::message("usize does not fit in a Lua integer"))
    }
}

impl IntoLua for isize {
    fn into_lua(self) -> Result<Value> {
        i64::try_from(self)
            .map(Value::Integer)
            .map_err(|_| Error::message("isize does not fit in a Lua integer"))
    }
}

impl IntoLua for u64 {
    fn into_lua(self) -> Result<Value> {
        i64::try_from(self)
            .map(Value::Integer)
            .map_err(|_| Error::message("u64 does not fit in a Lua integer"))
    }
}

impl IntoLua for &LuaString {
    fn into_lua(self) -> Result<Value> {
        Ok(Value::String(self.clone()))
    }
}

impl IntoLua for &String {
    fn into_lua(self) -> Result<Value> {
        Ok(Value::String(self.as_str().into()))
    }
}

impl IntoLua for &Table {
    fn into_lua(self) -> Result<Value> {
        Ok(Value::Table(self.clone()))
    }
}

impl IntoLua for &Function {
    fn into_lua(self) -> Result<Value> {
        Ok(Value::Function(self.clone()))
    }
}

impl<T: IntoLua> IntoLua for Option<T> {
    fn into_lua(self) -> Result<Value> {
        self.map(IntoLua::into_lua).unwrap_or(Ok(Value::Nil))
    }
}

impl FromLua for Value {
    fn from_lua(value: Value) -> Result<Self> {
        Ok(value)
    }
}

impl FromLua for bool {
    fn from_lua(value: Value) -> Result<Self> {
        // lua_toboolean / the original mlua conversion: only nil and false
        // are false. Stock FrameXML passes 1/nil to boolean host arguments;
        // numeric zero and empty strings are truthy too.
        Ok(!matches!(value, Value::Nil | Value::Boolean(false)))
    }
}

impl FromLua for LuaString {
    fn from_lua(value: Value) -> Result<Self> {
        match value {
            Value::String(value) => Ok(value),
            Value::Integer(value) => Ok(value.to_string().into()),
            Value::Number(value) => Ok(value.to_string().into()),
            value => Err(Error::type_mismatch("string", &value)),
        }
    }
}

impl FromLua for String {
    fn from_lua(value: Value) -> Result<Self> {
        let value = LuaString::from_lua(value)?;
        value
            .as_str()
            .map(ToOwned::to_owned)
            .map_err(|error| Error::message(format!("Lua string is not valid UTF-8: {error}")))
    }
}

impl FromLua for Vec<u8> {
    fn from_lua(value: Value) -> Result<Self> {
        Ok(LuaString::from_lua(value)?.as_bytes().to_vec())
    }
}

impl FromLua for Table {
    fn from_lua(value: Value) -> Result<Self> {
        match value {
            Value::Table(value) => Ok(value),
            value => Err(Error::type_mismatch("table", &value)),
        }
    }
}

impl FromLua for Function {
    fn from_lua(value: Value) -> Result<Self> {
        match value {
            Value::Function(value) => Ok(value),
            value => Err(Error::type_mismatch("function", &value)),
        }
    }
}

impl<T: FromLua> FromLua for Option<T> {
    fn from_lua(value: Value) -> Result<Self> {
        match value {
            Value::Nil => Ok(None),
            value => T::from_lua(value).map(Some),
        }
    }
}

fn number(value: Value) -> Result<f64> {
    match value {
        Value::Integer(value) => Ok(value as f64),
        Value::Number(value) => Ok(value),
        Value::String(value) => value
            .as_str()
            .ok()
            .and_then(|value| value.trim().parse().ok())
            .ok_or_else(|| Error::message("expected a Lua number or numeric string")),
        value => Err(Error::type_mismatch("number", &value)),
    }
}

impl FromLua for f64 {
    fn from_lua(value: Value) -> Result<Self> {
        number(value)
    }
}

impl FromLua for f32 {
    fn from_lua(value: Value) -> Result<Self> {
        Ok(number(value)? as Self)
    }
}

macro_rules! signed_from_lua {
    ($($ty:ty),* $(,)?) => {
        $(
            impl FromLua for $ty {
                fn from_lua(value: Value) -> Result<Self> {
                    let value = number(value)?;
                    if !value.is_finite() || value < <$ty>::MIN as f64 || value > <$ty>::MAX as f64 {
                        return Err(Error::message(concat!("Lua number does not fit in ", stringify!($ty))));
                    }
                    Ok(value as Self)
                }
            }
        )*
    };
}

macro_rules! unsigned_from_lua {
    ($($ty:ty),* $(,)?) => {
        $(
            impl FromLua for $ty {
                fn from_lua(value: Value) -> Result<Self> {
                    let value = number(value)?;
                    if !value.is_finite() || value < 0.0 || value > <$ty>::MAX as f64 {
                        return Err(Error::message(concat!("Lua number does not fit in ", stringify!($ty))));
                    }
                    Ok(value as Self)
                }
            }
        )*
    };
}

signed_from_lua!(i8, i16, i32, i64, isize);
unsigned_from_lua!(u8, u16, u32, u64, usize);

impl<T: FromLua> FromLuaMulti for T {
    fn from_lua_multi(mut values: MultiValue) -> Result<Self> {
        let value = if values.is_empty() {
            Value::Nil
        } else {
            values.remove(0)
        };
        T::from_lua(value)
    }
}

impl FromLuaMulti for () {
    fn from_lua_multi(_values: MultiValue) -> Result<Self> {
        Ok(())
    }
}

impl FromLuaMulti for MultiValue {
    fn from_lua_multi(values: MultiValue) -> Result<Self> {
        Ok(values)
    }
}

impl<T: FromLua> FromLuaMulti for Variadic<T> {
    fn from_lua_multi(values: MultiValue) -> Result<Self> {
        values
            .into_iter()
            .map(T::from_lua)
            .collect::<Result<Vec<_>>>()
            .map(Self)
    }
}

impl<T: IntoLua> IntoLuaMulti for T {
    fn into_lua_multi(self) -> Result<MultiValue> {
        Ok(vec![self.into_lua()?])
    }
}

impl IntoLuaMulti for () {
    fn into_lua_multi(self) -> Result<MultiValue> {
        Ok(Vec::new())
    }
}

impl IntoLuaMulti for MultiValue {
    fn into_lua_multi(self) -> Result<Self> {
        Ok(self)
    }
}

impl<T: IntoLua> IntoLuaMulti for Variadic<T> {
    fn into_lua_multi(self) -> Result<MultiValue> {
        self.0.into_iter().map(IntoLua::into_lua).collect()
    }
}

macro_rules! tuple_multi {
    // Single-element tuples take the whole value list at once, so there is no
    // cursor to advance and no mutable binding. The multi-element arm below
    // keeps one: each head consumes its value through the iterator.
    ($last:ident) => {
        impl<$last> FromLuaMulti for ($last,)
        where
            $last: FromLuaMulti,
        {
            #[allow(non_snake_case)]
            fn from_lua_multi(values: MultiValue) -> Result<Self> {
                let $last = $last::from_lua_multi(values)?;
                Ok(($last,))
            }
        }

        impl<$last> IntoLuaMulti for ($last,)
        where
            $last: IntoLuaMulti,
        {
            #[allow(non_snake_case)]
            fn into_lua_multi(self) -> Result<MultiValue> {
                let ($last,) = self;
                $last.into_lua_multi()
            }
        }
    };
    ($last:ident $(, $head:ident)+) => {
        impl<$($head,)* $last> FromLuaMulti for ($($head,)* $last,)
        where
            $($head: FromLua,)*
            $last: FromLuaMulti,
        {
            #[allow(non_snake_case)]
            fn from_lua_multi(values: MultiValue) -> Result<Self> {
                let mut values = values.into_iter();
                $(let $head = $head::from_lua(values.next().unwrap_or(Value::Nil))?;)*
                let $last = $last::from_lua_multi(values.collect())?;
                Ok(($($head,)* $last,))
            }
        }

        impl<$($head,)* $last> IntoLuaMulti for ($($head,)* $last,)
        where
            $($head: IntoLua,)*
            $last: IntoLuaMulti,
        {
            #[allow(non_snake_case)]
            fn into_lua_multi(self) -> Result<MultiValue> {
                let ($($head,)* $last,) = self;
                let mut values = Vec::new();
                $(values.push($head.into_lua()?);)*
                values.extend($last.into_lua_multi()?);
                Ok(values)
            }
        }
    };
}

tuple_multi!(A);
tuple_multi!(B, A);
tuple_multi!(C, A, B);
tuple_multi!(D, A, B, C);
tuple_multi!(E, A, B, C, D);
tuple_multi!(F, A, B, C, D, E);
tuple_multi!(G, A, B, C, D, E, F);
tuple_multi!(H, A, B, C, D, E, F, G);
tuple_multi!(I, A, B, C, D, E, F, G, H);
tuple_multi!(J, A, B, C, D, E, F, G, H, I);
tuple_multi!(K, A, B, C, D, E, F, G, H, I, J);
tuple_multi!(L, A, B, C, D, E, F, G, H, I, J, K);
