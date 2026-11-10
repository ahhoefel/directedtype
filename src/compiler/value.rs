use crate::compiler::expanded::NodeId;
use std::fmt;

/// A runtime value computed during layout graph evaluation.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Integer(i64),
    Float(f64),
    Distance(f64),
    Area(f64),
    Duration(i64),
    String(String),
    Bool(bool),
    Color(String),
    Node(NodeId),
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

/// Helper type for constructing a fixed-length `\Timer` value from Rust/WASM companion code.
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

/// Helper type for constructing an `\IndefiniteTimer()` value from Rust/WASM companion code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct IndefiniteTimer;

impl IndefiniteTimer {
    pub fn new() -> Self {
        Self
    }
}

impl Value {
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Integer(n) | Value::Duration(n) => Some(*n as f64),
            Value::Float(n) | Value::Distance(n) | Value::Area(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Integer(n) | Value::Duration(n) => Some(*n),
            Value::Float(n) | Value::Distance(n) | Value::Area(n) => Some(*n as i64),
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

    pub fn as_area(&self) -> Option<f64> {
        match self {
            Value::Area(a) => Some(*a),
            Value::Integer(0) => Some(0.0),
            _ => None,
        }
    }

    /// Returns the duration in microseconds (`1s = 1_000_000`, `1ms = 1_000`).
    pub fn as_duration(&self) -> Option<i64> {
        match self {
            Value::Duration(d) => Some(*d),
            Value::Integer(0) => Some(0),
            _ => None,
        }
    }

    /// Returns the duration in seconds as a floating-point number.
    pub fn as_duration_secs(&self) -> Option<f64> {
        self.as_duration().map(|us| us as f64 / 1_000_000.0)
    }

    pub fn is_zero_integer(&self) -> bool {
        matches!(self, Value::Integer(0))
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

    pub fn as_node(&self) -> Option<NodeId> {
        match self {
            Value::Node(id) => Some(*id),
            _ => None,
        }
    }

    pub fn as_truthy(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            Value::Timer { time, duration, .. } => Some(*duration > 0 && *time < *duration),
            Value::IndefiniteTimer { .. } => Some(true),
            Value::Unset => Some(false),
            _ => None,
        }
    }

    pub fn to_display_string(&self) -> String {
        match self {
            Value::String(s) => s.clone(),
            Value::Integer(n) => format!("{n}"),
            Value::Float(n) => {
                if n.fract() == 0.0 && n.is_finite() {
                    format!("{:.0}", n)
                } else {
                    format!("{n}")
                }
            }
            Value::Distance(n) => {
                if n.fract() == 0.0 && n.is_finite() {
                    format!("{:.0}px", n)
                } else {
                    format!("{n}px")
                }
            }
            Value::Area(n) => {
                if n.fract() == 0.0 && n.is_finite() {
                    format!("{:.0}px^2", n)
                } else {
                    format!("{n}px^2")
                }
            }
            Value::Duration(us) => crate::ast::DurationUnit::S.format_micros(*us),
            Value::Bool(b) => format!("{b}"),
            Value::Color(c) => c.clone(),
            Value::Node(id) => id.canonical_name(),
            Value::Enum { enum_name, variant } => format!("{}.{}", enum_name, variant),
            Value::Timer { time, duration, .. } => format!(
                "Timer({} / {})",
                crate::ast::DurationUnit::S.format_micros(*time),
                crate::ast::DurationUnit::S.format_micros(*duration)
            ),
            Value::IndefiniteTimer { time, .. } => format!(
                "IndefiniteTimer({})",
                crate::ast::DurationUnit::S.format_micros(*time)
            ),
            Value::Unset => "unset".to_string(),
        }
    }

    pub fn type_name(&self) -> &str {
        match self {
            Value::Integer(_) => "Integer",
            Value::Float(_) => "Float",
            Value::Distance(_) => "Distance",
            Value::Area(_) => "Area",
            Value::Duration(_) => "Duration",
            Value::String(_) => "String",
            Value::Bool(_) => "Boolean",
            Value::Color(_) => "Color",
            Value::Node(_) => "Node",
            Value::Enum { enum_name, .. } => enum_name.as_str(),
            Value::Timer { .. } => "Timer",
            Value::IndefiniteTimer { .. } => "IndefiniteTimer",
            Value::Unset => "unset",
        }
    }

    pub fn matches_type_name(&self, expected: &str) -> bool {
        match expected {
            "Integer" => matches!(self, Value::Integer(_)),
            "Float" => matches!(self, Value::Float(_) | Value::Integer(_)),
            "Distance" => matches!(self, Value::Distance(_) | Value::Integer(0)),
            "Area" => matches!(self, Value::Area(_) | Value::Integer(0)),
            "Duration" => matches!(self, Value::Duration(_) | Value::Integer(0)),
            "String" => matches!(self, Value::String(_)),
            "Boolean" => matches!(self, Value::Bool(_)),
            "Color" => matches!(self, Value::Color(_)),
            "Node" | "ViewPort" => matches!(self, Value::Node(_)),
            "Timer" => matches!(self, Value::Timer { .. } | Value::Unset),
            "IndefiniteTimer" => matches!(self, Value::IndefiniteTimer { .. } | Value::Unset),
            "Handler" => matches!(self, Value::Unset),
            custom => match self {
                Value::Enum { enum_name, .. } => enum_name == custom,
                Value::Node(_) => true,
                _ => false,
            },
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Integer(n) => write!(f, "{}", n),
            Value::Float(n) => write!(f, "{}", n),
            Value::Distance(n) => {
                if n.fract() == 0.0 && n.is_finite() {
                    write!(f, "{:.0}px", n)
                } else {
                    write!(f, "{}px", n)
                }
            }
            Value::Area(n) => {
                if n.fract() == 0.0 && n.is_finite() {
                    write!(f, "{:.0}px^2", n)
                } else {
                    write!(f, "{}px^2", n)
                }
            }
            Value::Duration(us) => write!(f, "{}", crate::ast::DurationUnit::S.format_micros(*us)),
            Value::String(s) => write!(f, "\"{}\"", s),
            Value::Bool(b) => write!(f, "{}", b),
            Value::Color(c) => write!(f, "{}", c),
            Value::Node(id) => write!(f, "{}", id.canonical_name()),
            Value::Enum { enum_name, variant } => write!(f, "{}.{}", enum_name, variant),
            Value::Timer { time, duration, .. } => write!(
                f,
                "Timer({} / {})",
                crate::ast::DurationUnit::S.format_micros(*time),
                crate::ast::DurationUnit::S.format_micros(*duration)
            ),
            Value::IndefiniteTimer { time, .. } => write!(
                f,
                "IndefiniteTimer({})",
                crate::ast::DurationUnit::S.format_micros(*time)
            ),
            Value::Unset => write!(f, "unset"),
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

impl From<std::time::Duration> for Value {
    fn from(d: std::time::Duration) -> Self {
        Value::Duration(d.as_micros() as i64)
    }
}

impl From<f64> for Value {
    fn from(n: f64) -> Self {
        Value::Float(n)
    }
}

impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Value::Bool(b)
    }
}

impl From<String> for Value {
    fn from(s: String) -> Self {
        Value::String(s)
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::String(s.to_string())
    }
}

impl From<f32> for Value {
    fn from(n: f32) -> Self {
        Value::Float(n as f64)
    }
}

impl From<i32> for Value {
    fn from(n: i32) -> Self {
        Value::Integer(n as i64)
    }
}

impl From<i64> for Value {
    fn from(n: i64) -> Self {
        Value::Integer(n)
    }
}

impl From<usize> for Value {
    fn from(n: usize) -> Self {
        Value::Integer(n as i64)
    }
}

impl From<()> for Value {
    fn from(_: ()) -> Self {
        Value::Bool(true)
    }
}

