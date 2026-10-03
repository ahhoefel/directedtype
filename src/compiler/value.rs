use crate::compiler::expanded::NodeId;
use std::fmt;

/// A runtime value computed during layout graph evaluation.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Number(f64),
    String(String),
    Bool(bool),
    Color(String),
    Node(NodeId),
}

impl Value {
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Number(n) => Some(*n),
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
            Value::Number(n) => *n != 0.0,
            Value::String(s) => !s.is_empty(),
            Value::Color(_) => true,
            Value::Node(_) => true,
        }
    }

    pub fn to_display_string(&self) -> String {
        match self {
            Value::String(s) => s.clone(),
            Value::Number(n) => {
                if n.fract() == 0.0 && n.is_finite() {
                    format!("{:.0}", n)
                } else {
                    format!("{n}")
                }
            }
            Value::Bool(b) => format!("{b}"),
            Value::Color(c) => c.clone(),
            Value::Node(id) => id.canonical_name(),
        }
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Number(_) => "Number",
            Value::String(_) => "String",
            Value::Bool(_) => "Boolean",
            Value::Color(_) => "Color",
            Value::Node(_) => "Node",
        }
    }

    pub fn matches_type_name(&self, expected: &str) -> bool {
        match expected {
            "Number" => matches!(self, Value::Number(_)),
            "String" => matches!(self, Value::String(_)),
            "Boolean" | "Bool" => matches!(self, Value::Bool(_)),
            "Color" => matches!(self, Value::Color(_)),
            "Node" => matches!(self, Value::Node(_)),
            _ => true,
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Number(n) => write!(f, "{}", n),
            Value::String(s) => write!(f, "\"{}\"", s),
            Value::Bool(b) => write!(f, "{}", b),
            Value::Color(c) => write!(f, "{}", c),
            Value::Node(id) => write!(f, "{}", id.canonical_name()),
        }
    }
}

impl From<f64> for Value {
    fn from(n: f64) -> Self {
        Value::Number(n)
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
