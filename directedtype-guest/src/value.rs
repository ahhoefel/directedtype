use serde::{Deserialize, Serialize};

/// A runtime value exchanged between host layout engine and guest components.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Value {
    Integer(i64),
    Float(f64),
    Distance(f64),
    Duration(i64),
    String(String),
    Bool(bool),
    Color(String),
    Node(u64),
    Enum {
        enum_name: String,
        variant: String,
    },
    Timer {
        time: i64,
        duration: i64,
        dt: i64,
        frames: i64,
    },
    IndefiniteTimer {
        time: i64,
        dt: i64,
        frames: i64,
    },
    Unset,
}

/// Helper type for constructing a fixed-length `\Timer` value from guest component code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timer {
    pub duration_micros: i64,
}

impl Timer {
    pub fn new(duration: std::time::Duration) -> Self {
        Self {
            duration_micros: duration.as_micros() as i64,
        }
    }

    pub fn from_micros(micros: i64) -> Self {
        Self {
            duration_micros: micros,
        }
    }

    pub fn from_millis(millis: i64) -> Self {
        Self {
            duration_micros: millis.saturating_mul(1_000),
        }
    }

    pub fn from_secs_f64(secs: f64) -> Self {
        Self {
            duration_micros: (secs * 1_000_000.0).round() as i64,
        }
    }
}

/// Helper type for constructing an `\IndefiniteTimer()` value from guest component code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct IndefiniteTimer;

impl IndefiniteTimer {
    pub fn new() -> Self {
        Self
    }
}

impl Value {
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Integer(i) | Value::Duration(i) => Some(*i),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Integer(i) => Some(*i as f64),
            Value::Float(n) | Value::Distance(n) => Some(*n),
            Value::Duration(us) => Some(*us as f64 / 1_000_000.0),
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

    pub fn as_duration(&self) -> Option<i64> {
        match self {
            Value::Duration(d) => Some(*d),
            Value::Integer(0) => Some(0),
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
            Value::Integer(i) | Value::Duration(i) => *i != 0,
            Value::Float(n) | Value::Distance(n) => *n != 0.0,
            Value::String(s) => !s.is_empty(),
            Value::Color(_) => true,
            Value::Node(_) => true,
            Value::Enum { .. } => true,
            Value::Timer { time, duration, .. } => *duration > 0 && *time < *duration,
            Value::IndefiniteTimer { .. } => true,
            Value::Unset => false,
        }
    }
}

impl From<Timer> for Value {
    fn from(t: Timer) -> Self {
        if t.duration_micros <= 0 {
            Value::Unset
        } else {
            Value::Timer {
                time: 0,
                duration: t.duration_micros,
                dt: 0,
                frames: 0,
            }
        }
    }
}

impl From<IndefiniteTimer> for Value {
    fn from(_: IndefiniteTimer) -> Self {
        Value::IndefiniteTimer {
            time: 0,
            dt: 0,
            frames: 0,
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

