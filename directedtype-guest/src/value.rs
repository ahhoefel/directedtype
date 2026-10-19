use serde::{Deserialize, Serialize};

/// A runtime value exchanged between host layout engine and guest components.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Value {
    Integer(i64),
    Float(f64),
    Distance(f64),
    String(String),
    Bool(bool),
    Color(String),
    Node(u64),
    Enum {
        enum_name: String,
        variant: String,
    },
}

impl Value {
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Integer(i) => Some(*i),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Integer(i) => Some(*i as f64),
            Value::Float(n) | Value::Distance(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_distance(&self) -> Option<f64> {
        match self {
            Value::Distance(d) => Some(*d),
            Value::Integer(0) => Some(0.0),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) | Value::Color(s) => Some(s.as_str()),
            Value::Enum { variant, .. } => Some(variant.as_str()),
            _ => None,
        }
    }

    pub fn is_truthy(&self) -> bool {
        match self {
            Value::Bool(b) => *b,
            Value::Integer(i) => *i != 0,
            Value::Float(n) | Value::Distance(n) => *n != 0.0,
            Value::String(s) => !s.is_empty(),
            Value::Color(_) => true,
            Value::Node(_) => true,
            Value::Enum { .. } => true,
        }
    }
}

impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Value::Float(v)
    }
}

impl From<f32> for Value {
    fn from(v: f32) -> Self {
        Value::Float(v as f64)
    }
}

impl From<i32> for Value {
    fn from(v: i32) -> Self {
        Value::Integer(v as i64)
    }
}

impl From<i64> for Value {
    fn from(v: i64) -> Self {
        Value::Integer(v)
    }
}

impl From<usize> for Value {
    fn from(v: usize) -> Self {
        Value::Integer(v as i64)
    }
}

impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Value::Bool(v)
    }
}

impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Value::String(v.to_string())
    }
}

impl From<String> for Value {
    fn from(v: String) -> Self {
        Value::String(v)
    }
}

