use std::{fmt, sync::Arc};

use crate::{Function, Table};

/// A Lua string kept as bytes until a caller explicitly requests UTF-8.
#[derive(Clone, Default, Eq, Hash, PartialEq)]
pub struct LuaString(Arc<[u8]>);

impl LuaString {
    pub fn new(bytes: impl Into<Arc<[u8]>>) -> Self {
        Self(bytes.into())
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn as_str(&self) -> std::result::Result<&str, std::str::Utf8Error> {
        std::str::from_utf8(&self.0)
    }

    pub fn to_string_lossy(&self) -> std::borrow::Cow<'_, str> {
        String::from_utf8_lossy(&self.0)
    }
}

impl fmt::Debug for LuaString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("LuaString")
            .field(&self.to_string_lossy())
            .finish()
    }
}

impl From<&str> for LuaString {
    fn from(value: &str) -> Self {
        Self::new(Arc::<[u8]>::from(value.as_bytes()))
    }
}

impl From<String> for LuaString {
    fn from(value: String) -> Self {
        Self::new(Arc::<[u8]>::from(value.into_bytes()))
    }
}

impl From<&[u8]> for LuaString {
    fn from(value: &[u8]) -> Self {
        Self::new(Arc::<[u8]>::from(value))
    }
}

impl From<Vec<u8>> for LuaString {
    fn from(value: Vec<u8>) -> Self {
        Self::new(Arc::<[u8]>::from(value))
    }
}

/// The value surface shared by the native and browser VM backends.
#[derive(Clone, Debug)]
pub enum Value {
    Nil,
    Boolean(bool),
    Integer(i64),
    Number(f64),
    String(LuaString),
    Table(Table),
    Function(Function),
    LightUserData(usize),
}

impl Value {
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Nil => "nil",
            Self::Boolean(_) => "boolean",
            Self::Integer(_) | Self::Number(_) => "number",
            Self::String(_) => "string",
            Self::Table(_) => "table",
            Self::Function(_) => "function",
            Self::LightUserData(_) => "userdata",
        }
    }
}

impl From<bool> for Value {
    fn from(value: bool) -> Self {
        Self::Boolean(value)
    }
}

macro_rules! integer_values {
    ($($ty:ty),* $(,)?) => {
        $(
            impl From<$ty> for Value {
                fn from(value: $ty) -> Self {
                    Self::Integer(value as i64)
                }
            }
        )*
    };
}

integer_values!(i8, i16, i32, i64, u8, u16, u32);

impl From<f32> for Value {
    fn from(value: f32) -> Self {
        Self::Number(f64::from(value))
    }
}

impl From<f64> for Value {
    fn from(value: f64) -> Self {
        Self::Number(value)
    }
}

impl From<LuaString> for Value {
    fn from(value: LuaString) -> Self {
        Self::String(value)
    }
}

impl From<&str> for Value {
    fn from(value: &str) -> Self {
        Self::String(value.into())
    }
}

impl From<String> for Value {
    fn from(value: String) -> Self {
        Self::String(value.into())
    }
}

impl From<&[u8]> for Value {
    fn from(value: &[u8]) -> Self {
        Self::String(value.into())
    }
}

impl From<Vec<u8>> for Value {
    fn from(value: Vec<u8>) -> Self {
        Self::String(value.into())
    }
}

impl From<Table> for Value {
    fn from(value: Table) -> Self {
        Self::Table(value)
    }
}

impl From<Function> for Value {
    fn from(value: Function) -> Self {
        Self::Function(value)
    }
}

impl<T: Into<Value>> From<Option<T>> for Value {
    fn from(value: Option<T>) -> Self {
        value.map(Into::into).unwrap_or(Self::Nil)
    }
}

pub type MultiValue = Vec<Value>;

/// Backend-neutral Lua failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Error {
    message: String,
}

impl Error {
    pub(crate) fn backend(error: impl fmt::Display) -> Self {
        Self {
            message: error.to_string(),
        }
    }

    pub(crate) fn message(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// Construct an ordinary host-raised Lua runtime error.
    pub fn runtime(message: impl Into<String>) -> Self {
        Self::message(message)
    }

    pub(crate) fn type_mismatch(expected: &str, actual: &Value) -> Self {
        Self::message(format!(
            "expected Lua {expected}, got {}",
            actual.type_name()
        ))
    }

    pub fn as_str(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

pub type Result<T, E = Error> = std::result::Result<T, E>;
