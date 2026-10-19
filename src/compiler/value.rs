use crate::compiler::expanded::NodeId;
use std::fmt;

/// A runtime value computed during layout graph evaluation.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Integer(i64),
    Float(f64),
    Distance(f64),
    String(String),
    Bool(bool),
    Color(String),
    Node(NodeId),
    Enum {
        enum_name: String,
        variant: String,
    },
}

impl Value {
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Integer(n) => Some(*n as f64),
            Value::Float(n) | Value::Distance(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Integer(n) => Some(*n),
            Value::Float(n) | Value::Distance(n) => Some(*n as i64),
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

    pub fn is_truthy(&self) -> bool {
        match self {
            Value::Bool(b) => *b,
            Value::Integer(n) => *n != 0,
            Value::Float(n) | Value::Distance(n) => *n != 0.0,
            Value::String(s) => !s.is_empty(),
            Value::Color(_) => true,
            Value::Node(_) => true,
            Value::Enum { .. } => true,
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
            Value::Bool(b) => format!("{b}"),
            Value::Color(c) => c.clone(),
            Value::Node(id) => id.canonical_name(),
            Value::Enum { enum_name, variant } => format!("{}.{}", enum_name, variant),
        }
    }

    pub fn type_name(&self) -> &str {
        match self {
            Value::Integer(_) => "Integer",
            Value::Float(_) => "Float",
            Value::Distance(_) => "Distance",
            Value::String(_) => "String",
            Value::Bool(_) => "Boolean",
            Value::Color(_) => "Color",
            Value::Node(_) => "Node",
            Value::Enum { enum_name, .. } => enum_name.as_str(),
        }
    }

    pub fn matches_type_name(&self, expected: &str) -> bool {
        match expected {
            "Integer" | "Int" => matches!(self, Value::Integer(_)),
            "Float" | "Number" => matches!(self, Value::Float(_) | Value::Integer(_)),
            "Distance" => matches!(self, Value::Distance(_) | Value::Integer(0)),
            "String" => matches!(self, Value::String(_)),
            "Boolean" | "Bool" => matches!(self, Value::Bool(_)),
            "Color" => matches!(self, Value::Color(_)),
            "Node" => matches!(self, Value::Node(_)),
            custom => match self {
                Value::Enum { enum_name, .. } => enum_name == custom,
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
            Value::String(s) => write!(f, "\"{}\"", s),
            Value::Bool(b) => write!(f, "{}", b),
            Value::Color(c) => write!(f, "{}", c),
            Value::Node(id) => write!(f, "{}", id.canonical_name()),
            Value::Enum { enum_name, variant } => write!(f, "{}.{}", enum_name, variant),
        }
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

