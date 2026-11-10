use crate::compiler::expanded::NodeId;
use std::collections::HashMap;
use std::fmt;
use std::ops::Deref;
use std::sync::{OnceLock, RwLock};

const TAG_SHIFT: u64 = 62;
const TAG_STR: u64 = 0 << TAG_SHIFT;
const TAG_NODE: u64 = 1 << TAG_SHIFT;
const TAG_NEXT_OF: u64 = 2 << TAG_SHIFT;
const TAG_AGG_CHILD: u64 = 3 << TAG_SHIFT;

const NODE_WINDOW_RAW: u64 = u32::MAX as u64;
const NODE_TEMPLATE_SELF_RAW: u64 = (u32::MAX - 1) as u64;

/// Pre-interned static strings for zero-lock, compile-time constant `Symbol` lookup.
const PREINTERNED: &[&str] = &[
    "",
    "window",
    "self",
    "parent",
    "prev",
    "next",
    "child",
    "children",
    "Children",
    "Component",
    "env",
    "x",
    "y",
    "z",
    "width",
    "height",
    "left",
    "top",
    "right",
    "bottom",
    "font",
    "clip",
    "color",
    "bg_color",
    "size",
    "weight",
    "family",
    "italic",
    "underline",
    "text",
    "line_height",
    "baseline_shift",
    "__baseline_offset",
    "radius",
    "radius_left",
    "radius_right",
    "radius_top",
    "radius_bottom",
    "radius_top_left",
    "radius_top_right",
    "radius_bottom_right",
    "radius_bottom_left",
    "opacity",
    "border_width",
    "border_color",
    "scroll_x",
    "scroll_y",
    "vscroll",
    "hscroll",
    "title",
    "dpi",
    "url",
    "name",
    "path",
    "append",
    "len",
    "only",
    "unset",
    "None",
    "true",
    "false",
    "Rect",
    "Text",
    "Line",
    "Font",
    "Anchor",
    "Link",
    "Timer",
    "IndefiniteTimer",
    "Distance",
    "Duration",
    "Float",
    "Integer",
    "Boolean",
    "String",
    "Color",
    "Handler",
    "text_width",
    "text_height",
    "font_line_height",
    "font_ascender",
    "font_descender",
    "num_children",
    "outer",
    "Expr",
    "Expression",
    "match",
    "_",
];

struct SymbolInterner {
    map: HashMap<&'static str, Symbol>,
    vec: Vec<&'static str>,
}

static INTERNER: OnceLock<RwLock<SymbolInterner>> = OnceLock::new();

#[inline]
fn get_interner() -> &'static RwLock<SymbolInterner> {
    INTERNER.get_or_init(|| {
        let mut map = HashMap::with_capacity(256);
        for (i, &s) in PREINTERNED.iter().enumerate() {
            map.insert(s, Symbol(TAG_STR | (i as u64)));
        }
        RwLock::new(SymbolInterner {
            map,
            vec: Vec::with_capacity(256),
        })
    })
}

fn intern_dynamic_str(s: &str) -> Symbol {
    let lock = get_interner();
    {
        let read = lock.read().unwrap();
        if let Some(&sym) = read.map.get(s) {
            return sym;
        }
    }
    let mut write = lock.write().unwrap();
    if let Some(&sym) = write.map.get(s) {
        return sym;
    }
    let leaked: &'static str = Box::leak(s.to_string().into_boxed_str());
    let idx = PREINTERNED.len() + write.vec.len();
    let sym = Symbol(TAG_STR | (idx as u64));
    write.vec.push(leaked);
    write.map.insert(leaked, sym);
    sym
}

/// A compact 64-bit copyable identifier representing either an interned string symbol
/// or a direct node reference (`NodeId`, `NextOf(NodeId)`, `AggChildOf(NodeId, usize)`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Symbol(pub u64);

impl Symbol {
    pub const EMPTY: Self = Self(0);
    pub const WINDOW: Self = Self(1);
    pub const SELF: Self = Self(2);
    pub const PARENT: Self = Self(3);
    pub const PREV: Self = Self(4);
    pub const NEXT: Self = Self(5);
    pub const CHILD: Self = Self(6);
    pub const CHILDREN: Self = Self(7);
    pub const CHILDREN_DIRECTIVE: Self = Self(8);
    pub const COMPONENT_KW: Self = Self(9);
    pub const ENV: Self = Self(10);
    pub const X: Self = Self(11);
    pub const Y: Self = Self(12);
    pub const Z: Self = Self(13);
    pub const WIDTH: Self = Self(14);
    pub const HEIGHT: Self = Self(15);
    pub const LEFT: Self = Self(16);
    pub const TOP: Self = Self(17);
    pub const RIGHT: Self = Self(18);
    pub const BOTTOM: Self = Self(19);
    pub const FONT: Self = Self(20);
    pub const CLIP: Self = Self(21);
    pub const COLOR: Self = Self(22);
    pub const BG_COLOR: Self = Self(23);
    pub const SIZE: Self = Self(24);
    pub const WEIGHT: Self = Self(25);
    pub const FAMILY: Self = Self(26);
    pub const ITALIC: Self = Self(27);
    pub const UNDERLINE: Self = Self(28);
    pub const TEXT: Self = Self(29);
    pub const LINE_HEIGHT: Self = Self(30);
    pub const BASELINE_SHIFT: Self = Self(31);
    pub const BASELINE_OFFSET: Self = Self(32);
    pub const RADIUS: Self = Self(33);
    pub const RADIUS_LEFT: Self = Self(34);
    pub const RADIUS_RIGHT: Self = Self(35);
    pub const RADIUS_TOP: Self = Self(36);
    pub const RADIUS_BOTTOM: Self = Self(37);
    pub const RADIUS_TOP_LEFT: Self = Self(38);
    pub const RADIUS_TOP_RIGHT: Self = Self(39);
    pub const RADIUS_BOTTOM_RIGHT: Self = Self(40);
    pub const RADIUS_BOTTOM_LEFT: Self = Self(41);
    pub const OPACITY: Self = Self(42);
    pub const BORDER_WIDTH: Self = Self(43);
    pub const BORDER_COLOR: Self = Self(44);
    pub const SCROLL_X: Self = Self(45);
    pub const SCROLL_Y: Self = Self(46);
    pub const VSCROLL: Self = Self(47);
    pub const HSCROLL: Self = Self(48);
    pub const TITLE: Self = Self(49);
    pub const DPI: Self = Self(50);
    pub const URL: Self = Self(51);
    pub const NAME: Self = Self(52);
    pub const PATH: Self = Self(53);
    pub const APPEND: Self = Self(54);
    pub const LEN: Self = Self(55);
    pub const ONLY: Self = Self(56);
    pub const UNSET: Self = Self(57);
    pub const NONE: Self = Self(58);
    pub const TRUE: Self = Self(59);
    pub const FALSE: Self = Self(60);
    pub const RECT_PRIM: Self = Self(61);
    pub const TEXT_PRIM: Self = Self(62);
    pub const LINE_PRIM: Self = Self(63);
    pub const FONT_PRIM: Self = Self(64);
    pub const ANCHOR_PRIM: Self = Self(65);
    pub const LINK_PRIM: Self = Self(66);
    pub const TIMER_PRIM: Self = Self(67);
    pub const INDEFINITE_TIMER_PRIM: Self = Self(68);
    pub const TYPE_DISTANCE: Self = Self(69);
    pub const TYPE_DURATION: Self = Self(70);
    pub const TYPE_FLOAT: Self = Self(71);
    pub const TYPE_INTEGER: Self = Self(72);
    pub const TYPE_BOOLEAN: Self = Self(73);
    pub const TYPE_STRING: Self = Self(74);
    pub const TYPE_COLOR: Self = Self(75);
    pub const TYPE_HANDLER: Self = Self(76);
    pub const FN_TEXT_WIDTH: Self = Self(77);
    pub const FN_TEXT_HEIGHT: Self = Self(78);
    pub const FN_FONT_LINE_HEIGHT: Self = Self(79);
    pub const FN_FONT_ASCENDER: Self = Self(80);
    pub const FN_FONT_DESCENDER: Self = Self(81);
    pub const NUM_CHILDREN: Self = Self(82);
    pub const OUTER: Self = Self(83);
    pub const EXPR_KW: Self = Self(84);
    pub const EXPRESSION_KW: Self = Self(85);
    pub const MATCH_KW: Self = Self(86);
    pub const UNDERSCORE: Self = Self(87);

    /// Canonical node symbol for `NodeId::WINDOW` (`"__window"`).
    pub const WINDOW_NODE: Self = Self(TAG_NODE | NODE_WINDOW_RAW);
    /// Canonical node symbol for `NodeId::TEMPLATE_SELF` (`"__template_self"`).
    pub const TEMPLATE_SELF_NODE: Self = Self(TAG_NODE | NODE_TEMPLATE_SELF_RAW);

    /// Creates a `Symbol` directly representing a concrete `NodeId` in O(1) with zero allocation.
    #[inline]
    pub const fn from_node(id: NodeId) -> Self {
        if id.is_window() {
            Self::WINDOW_NODE
        } else if id.is_template_self() {
            Self::TEMPLATE_SELF_NODE
        } else {
            Self(TAG_NODE | (id.0 as u64))
        }
    }

    /// Creates a deferred `Symbol` representing the next sibling of `id` (`__next_of___node_N`).
    #[inline]
    pub const fn next_of(id: NodeId) -> Self {
        let raw = if id.is_window() {
            NODE_WINDOW_RAW
        } else {
            id.0 as u64
        };
        Self(TAG_NEXT_OF | raw)
    }

    /// Creates a deferred `Symbol` representing child `idx` of `parent` in an aggregation (`__agg_child_of___node_P_I`).
    #[inline]
    pub const fn agg_child_of(parent: NodeId, idx: usize) -> Self {
        let raw_parent = if parent.is_window() {
            NODE_WINDOW_RAW
        } else {
            parent.0 as u64
        };
        Self(TAG_AGG_CHILD | (raw_parent << 30) | ((idx as u64) & 0x3FFF_FFFF))
    }

    /// Returns `Some(NodeId)` if this symbol is a concrete node reference (`__node_N`, `__window`, or `window`).
    #[inline]
    pub const fn node_id(self) -> Option<NodeId> {
        let tag = self.0 & (3 << TAG_SHIFT);
        if tag == TAG_NODE {
            let raw = self.0 & 0xFFFF_FFFF;
            if raw == NODE_WINDOW_RAW {
                Some(NodeId::WINDOW)
            } else if raw == NODE_TEMPLATE_SELF_RAW {
                Some(NodeId::TEMPLATE_SELF)
            } else {
                Some(NodeId(raw as usize))
            }
        } else if self.0 == Self::WINDOW.0 {
            Some(NodeId::WINDOW)
        } else {
            None
        }
    }

    /// Returns `true` if this symbol is a canonical node reference (`__node_N` or `__window`), excluding `"window"`.
    #[inline]
    pub const fn is_canonical_node_ref(self) -> bool {
        (self.0 & (3 << TAG_SHIFT)) == TAG_NODE
    }

    /// Returns `Some(NodeId)` if this symbol is a deferred next-sibling reference (`__next_of_...`).
    #[inline]
    pub const fn next_of_node(self) -> Option<NodeId> {
        if (self.0 & (3 << TAG_SHIFT)) == TAG_NEXT_OF {
            let raw = self.0 & 0xFFFF_FFFF;
            if raw == NODE_WINDOW_RAW {
                Some(NodeId::WINDOW)
            } else {
                Some(NodeId(raw as usize))
            }
        } else {
            None
        }
    }

    /// Returns `Some((parent_id, child_idx))` if this symbol is a deferred aggregation child reference (`__agg_child_of_...`).
    #[inline]
    pub const fn agg_child_of_node(self) -> Option<(NodeId, usize)> {
        if (self.0 & (3 << TAG_SHIFT)) == TAG_AGG_CHILD {
            let raw_parent = (self.0 >> 30) & 0xFFFF_FFFF;
            let parent = if raw_parent == NODE_WINDOW_RAW {
                NodeId::WINDOW
            } else {
                NodeId(raw_parent as usize)
            };
            let idx = (self.0 & 0x3FFF_FFFF) as usize;
            Some((parent, idx))
        } else {
            None
        }
    }

    /// Interns a string slice into a `Symbol`.
    /// Recognizes canonical node strings (`__node_N`, `__window`, `__next_of_...`, `__agg_child_of_...`)
    /// and pre-interned built-in names without locking or allocating.
    #[inline]
    pub fn new(s: &str) -> Self {
        if s.as_bytes().first() == Some(&b'_') {
            if s == "__window" {
                return Self::WINDOW_NODE;
            }
            if s == "__baseline_offset" {
                return Self::BASELINE_OFFSET;
            }
            if let Some(num_str) = s.strip_prefix("__node_") {
                if let Ok(n) = num_str.parse::<usize>() {
                    return Self::from_node(NodeId(n));
                }
            }
            if let Some(rest) = s.strip_prefix("__next_of___node_") {
                if let Ok(n) = rest.parse::<usize>() {
                    return Self::next_of(NodeId(n));
                }
            }
            if let Some(rest) = s.strip_prefix("__agg_child_of___node_") {
                if let Some((p_str, i_str)) = rest.split_once('_') {
                    if let (Ok(p), Ok(i)) = (p_str.parse::<usize>(), i_str.parse::<usize>()) {
                        return Self::agg_child_of(NodeId(p), i);
                    }
                }
            }
        }

        match s {
            "" => Self::EMPTY,
            "window" => Self::WINDOW,
            "self" => Self::SELF,
            "parent" => Self::PARENT,
            "prev" => Self::PREV,
            "next" => Self::NEXT,
            "child" => Self::CHILD,
            "children" => Self::CHILDREN,
            "Children" => Self::CHILDREN_DIRECTIVE,
            "Component" => Self::COMPONENT_KW,
            "env" => Self::ENV,
            "x" => Self::X,
            "y" => Self::Y,
            "z" => Self::Z,
            "width" => Self::WIDTH,
            "height" => Self::HEIGHT,
            "left" => Self::LEFT,
            "top" => Self::TOP,
            "right" => Self::RIGHT,
            "bottom" => Self::BOTTOM,
            "font" => Self::FONT,
            "clip" => Self::CLIP,
            "color" => Self::COLOR,
            "bg_color" => Self::BG_COLOR,
            "size" => Self::SIZE,
            "weight" => Self::WEIGHT,
            "family" => Self::FAMILY,
            "italic" => Self::ITALIC,
            "underline" => Self::UNDERLINE,
            "text" => Self::TEXT,
            "line_height" => Self::LINE_HEIGHT,
            "baseline_shift" => Self::BASELINE_SHIFT,
            "radius" => Self::RADIUS,
            "radius_left" => Self::RADIUS_LEFT,
            "radius_right" => Self::RADIUS_RIGHT,
            "radius_top" => Self::RADIUS_TOP,
            "radius_bottom" => Self::RADIUS_BOTTOM,
            "radius_top_left" => Self::RADIUS_TOP_LEFT,
            "radius_top_right" => Self::RADIUS_TOP_RIGHT,
            "radius_bottom_right" => Self::RADIUS_BOTTOM_RIGHT,
            "radius_bottom_left" => Self::RADIUS_BOTTOM_LEFT,
            "opacity" => Self::OPACITY,
            "border_width" => Self::BORDER_WIDTH,
            "border_color" => Self::BORDER_COLOR,
            "scroll_x" => Self::SCROLL_X,
            "scroll_y" => Self::SCROLL_Y,
            "vscroll" => Self::VSCROLL,
            "hscroll" => Self::HSCROLL,
            "title" => Self::TITLE,
            "dpi" => Self::DPI,
            "url" => Self::URL,
            "name" => Self::NAME,
            "path" => Self::PATH,
            "append" => Self::APPEND,
            "len" => Self::LEN,
            "only" => Self::ONLY,
            "unset" => Self::UNSET,
            "None" => Self::NONE,
            "true" => Self::TRUE,
            "false" => Self::FALSE,
            "Rect" => Self::RECT_PRIM,
            "Text" => Self::TEXT_PRIM,
            "Line" => Self::LINE_PRIM,
            "Font" => Self::FONT_PRIM,
            "Anchor" => Self::ANCHOR_PRIM,
            "Link" => Self::LINK_PRIM,
            "Timer" => Self::TIMER_PRIM,
            "IndefiniteTimer" => Self::INDEFINITE_TIMER_PRIM,
            "Distance" => Self::TYPE_DISTANCE,
            "Duration" => Self::TYPE_DURATION,
            "Float" => Self::TYPE_FLOAT,
            "Integer" => Self::TYPE_INTEGER,
            "Boolean" => Self::TYPE_BOOLEAN,
            "String" => Self::TYPE_STRING,
            "Color" => Self::TYPE_COLOR,
            "Handler" => Self::TYPE_HANDLER,
            "text_width" => Self::FN_TEXT_WIDTH,
            "text_height" => Self::FN_TEXT_HEIGHT,
            "font_line_height" => Self::FN_FONT_LINE_HEIGHT,
            "font_ascender" => Self::FN_FONT_ASCENDER,
            "font_descender" => Self::FN_FONT_DESCENDER,
            "num_children" => Self::NUM_CHILDREN,
            "outer" => Self::OUTER,
            "Expr" => Self::EXPR_KW,
            "Expression" => Self::EXPRESSION_KW,
            "match" => Self::MATCH_KW,
            "_" => Self::UNDERSCORE,
            other => intern_dynamic_str(other),
        }
    }

    /// Resolves this symbol to its `'static` string representation.
    #[inline]
    pub fn as_str(self) -> &'static str {
        match self.0 & (3 << TAG_SHIFT) {
            TAG_STR => {
                let idx = (self.0 & 0x3FFF_FFFF) as usize;
                if let Some(&s) = PREINTERNED.get(idx) {
                    return s;
                }
                let lock = get_interner();
                let read = lock.read().unwrap();
                read.vec[idx - PREINTERNED.len()]
            }
            TAG_NODE => {
                let raw = self.0 & 0xFFFF_FFFF;
                if raw == NODE_WINDOW_RAW {
                    "__window"
                } else if raw == NODE_TEMPLATE_SELF_RAW {
                    "__template_self"
                } else {
                    NodeId(raw as usize).canonical_str()
                }
            }
            TAG_NEXT_OF => {
                let raw = self.0 & 0xFFFF_FFFF;
                let nid = if raw == NODE_WINDOW_RAW {
                    NodeId::WINDOW
                } else {
                    NodeId(raw as usize)
                };
                let formatted = format!("__next_of_{}", nid.canonical_str());
                intern_dynamic_str(&formatted).as_str()
            }
            TAG_AGG_CHILD => {
                let raw_parent = (self.0 >> 30) & 0xFFFF_FFFF;
                let parent = if raw_parent == NODE_WINDOW_RAW {
                    NodeId::WINDOW
                } else {
                    NodeId(raw_parent as usize)
                };
                let idx = (self.0 & 0x3FFF_FFFF) as usize;
                let formatted = format!("__agg_child_of_{}_{}", parent.canonical_str(), idx);
                intern_dynamic_str(&formatted).as_str()
            }
            _ => unreachable!(),
        }
    }
}

impl Deref for Symbol {
    type Target = str;

    #[inline]
    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<str> for Symbol {
    #[inline]
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for Symbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl fmt::Debug for Symbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), f)
    }
}

impl PartialOrd for Symbol {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Symbol {
    #[inline]
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        if self.0 == other.0 {
            std::cmp::Ordering::Equal
        } else {
            self.as_str().cmp(other.as_str())
        }
    }
}

impl PartialEq<str> for Symbol {
    #[inline]
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<&str> for Symbol {
    #[inline]
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl PartialEq<String> for Symbol {
    #[inline]
    fn eq(&self, other: &String) -> bool {
        self.as_str() == other.as_str()
    }
}

impl PartialEq<Symbol> for str {
    #[inline]
    fn eq(&self, other: &Symbol) -> bool {
        self == other.as_str()
    }
}

impl PartialEq<Symbol> for &str {
    #[inline]
    fn eq(&self, other: &Symbol) -> bool {
        *self == other.as_str()
    }
}

impl PartialEq<Symbol> for String {
    #[inline]
    fn eq(&self, other: &Symbol) -> bool {
        self.as_str() == other.as_str()
    }
}

impl From<&str> for Symbol {
    #[inline]
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

impl From<&Symbol> for Symbol {
    #[inline]
    fn from(s: &Symbol) -> Self {
        *s
    }
}

impl From<String> for Symbol {
    #[inline]
    fn from(s: String) -> Self {
        Self::new(&s)
    }
}

impl From<&String> for Symbol {
    #[inline]
    fn from(s: &String) -> Self {
        Self::new(s.as_str())
    }
}

impl From<std::borrow::Cow<'_, str>> for Symbol {
    #[inline]
    fn from(s: std::borrow::Cow<'_, str>) -> Self {
        Self::new(s.as_ref())
    }
}

impl From<Symbol> for String {
    #[inline]
    fn from(s: Symbol) -> Self {
        s.as_str().to_string()
    }
}

