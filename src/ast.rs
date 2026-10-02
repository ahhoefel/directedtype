use std::fmt;

use crate::dom::NodeHandle;
use crate::span::Span;

/// An identifier with its source span.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Ident {
    pub name: String,
    pub span: Span,
}

impl Ident {
    pub fn new(name: impl Into<String>, span: Span) -> Self {
        Self {
            name: name.into(),
            span,
        }
    }

    pub fn as_str(&self) -> &str {
        &self.name
    }
}

/// A parsed translation unit containing top-level component definitions and node instantiations.
#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    pub items: Vec<Item>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Component(ComponentDef),
    Node(ElementNode),
    Let(LetBinding),
    Env(EnvBinding),
}

impl Item {
    pub fn span(&self) -> Span {
        match self {
            Item::Component(c) => c.span,
            Item::Node(n) => n.span,
            Item::Let(l) => l.span,
            Item::Env(e) => e.span,
        }
    }
}

/// Component definition: `\Component Name(params) { body }`
#[derive(Debug, Clone, PartialEq)]
pub struct ComponentDef {
    pub name: Ident,
    pub params: Vec<ParamDef>,
    pub body: Vec<ComponentBodyItem>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ComponentBodyItem {
    Node(ElementNode),
    Children(ChildrenDirective),
    Let(LetBinding),
    Env(EnvBinding),
    Alias(AliasBinding),
}

impl ComponentBodyItem {
    pub fn span(&self) -> Span {
        match self {
            ComponentBodyItem::Node(n) => n.span,
            ComponentBodyItem::Children(c) => c.span,
            ComponentBodyItem::Let(l) => l.span,
            ComponentBodyItem::Env(e) => e.span,
            ComponentBodyItem::Alias(a) => a.span,
        }
    }
}

/// Public immutable alias port binding: `alias name = expr;` or `alias name: Type = expr;`
#[derive(Debug, Clone, PartialEq)]
pub struct AliasBinding {
    pub name: Ident,
    pub type_annotation: Option<TypeRef>,
    pub value: Expr,
    pub span: Span,
}

/// Local variable or element binding: `let name = expr;`, `let name = \Node(...);`, or uninitialized `let name;`
#[derive(Debug, Clone, PartialEq)]
pub struct LetBinding {
    pub name: Ident,
    pub type_annotation: Option<TypeRef>,
    pub value: Option<LetValue>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LetValue {
    Expr(Expr),
    Node(ElementNode),
}

impl LetValue {
    pub fn span(&self) -> Span {
        match self {
            LetValue::Expr(e) => e.span(),
            LetValue::Node(n) => n.span,
        }
    }
}

/// Environmental variable binding: `env name = expr;` or uninitialized `env name;`
#[derive(Debug, Clone, PartialEq)]
pub struct EnvBinding {
    pub name: Ident,
    pub type_annotation: Option<TypeRef>,
    pub value: Option<Expr>,
    pub span: Span,
}

/// Parameter definition in a component signature:
/// e.g. `bg_color: Color`, `gap: Number: 16`, `env theme: String`, or `width: max(children.width) + 32`
#[derive(Debug, Clone, PartialEq)]
pub struct ParamDef {
    pub is_env: bool,
    pub name: Ident,
    pub type_annotation: Option<TypeRef>,
    pub default_edge: Option<Expr>,
    pub span: Span,
}

/// A type reference, e.g. `Color`, `Number`, `String`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TypeRef {
    pub name: Ident,
    pub span: Span,
}

/// Element node instantiation: `\Name(ports) { content }`
#[derive(Debug, Clone, PartialEq)]
pub struct ElementNode {
    pub name: Ident,
    pub ports: Vec<PortBinding>,
    pub content: Option<ContentSlot>,
    pub span: Span,
    pub handle: Option<NodeHandle>,
}

/// The `\Children` directive: `\Children { x: parent.left, y: prev ? prev.bottom + gap : parent.top }`
#[derive(Debug, Clone, PartialEq)]
pub struct ChildrenDirective {
    pub ports: Vec<PortBinding>,
    pub span: Span,
}

/// Port binding: strictly `name: expr`
#[derive(Debug, Clone, PartialEq)]
pub struct PortBinding {
    pub name: Ident,
    pub expr: Expr,
    pub span: Span,
}

/// Content slot delimited by `{ ... }`
#[derive(Debug, Clone, PartialEq)]
pub struct ContentSlot {
    pub items: Vec<ContentItem>,
    pub span: Span,
}

/// Content items inside a content slot: raw text runs interleaved with inline nodes.
#[derive(Debug, Clone, PartialEq)]
pub enum ContentItem {
    Text(TextChunk),
    Node(ElementNode),
    Children(ChildrenDirective),
}

impl ContentItem {
    pub fn span(&self) -> Span {
        match self {
            ContentItem::Text(t) => t.span,
            ContentItem::Node(n) => n.span,
            ContentItem::Children(c) => c.span,
        }
    }
}

/// A normalized raw text chunk inside a content slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextChunk {
    pub text: String,
    pub span: Span,
}

/// Late-bound algebraic expression representing an edge in the layout DAG.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Literal(Literal),
    Ident(Ident),
    MemberAccess(MemberAccessExpr),
    Ternary(TernaryExpr),
    Call(CallExpr),
    Binary(BinaryExpr),
    Unary(UnaryExpr),
    Paren(Box<Expr>, Span),
    Node(Box<ElementNode>),
}

impl Expr {
    pub fn span(&self) -> Span {
        match self {
            Expr::Literal(lit) => lit.span(),
            Expr::Ident(id) => id.span,
            Expr::MemberAccess(m) => m.span,
            Expr::Ternary(t) => t.span,
            Expr::Call(c) => c.span,
            Expr::Binary(b) => b.span,
            Expr::Unary(u) => u.span,
            Expr::Paren(_, span) => *span,
            Expr::Node(n) => n.span,
        }
    }

    pub fn number(val: f64) -> Self {
        Expr::Literal(Literal::Number(val, Span::default()))
    }

    pub fn lit(val: f64) -> Self {
        Self::number(val)
    }

    pub fn string(val: impl Into<String>) -> Self {
        Expr::Literal(Literal::String(val.into(), Span::default()))
    }

    pub fn bool(val: bool) -> Self {
        Expr::Literal(Literal::Bool(val, Span::default()))
    }

    pub fn color(val: impl Into<String>) -> Self {
        Expr::Literal(Literal::Color(val.into(), Span::default()))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MemberAccessExpr {
    pub target: Box<Expr>,
    pub member: Ident,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TernaryExpr {
    pub condition: Box<Expr>,
    pub then_expr: Box<Expr>,
    pub else_expr: Box<Expr>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CallExpr {
    pub callee: Ident,
    pub args: Vec<Expr>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BinaryExpr {
    pub op: BinaryOp,
    pub left: Box<Expr>,
    pub right: Box<Expr>,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UnaryExpr {
    pub op: UnaryOp,
    pub operand: Box<Expr>,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Not,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    Number(f64, Span),
    String(String, Span),
    Bool(bool, Span),
    Color(String, Span),
}

impl Literal {
    pub fn span(&self) -> Span {
        match self {
            Literal::Number(_, span)
            | Literal::String(_, span)
            | Literal::Bool(_, span)
            | Literal::Color(_, span) => *span,
        }
    }
}

impl fmt::Display for Ident {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name)
    }
}

impl fmt::Display for BinaryOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BinaryOp::Add => write!(f, "+"),
            BinaryOp::Sub => write!(f, "-"),
            BinaryOp::Mul => write!(f, "*"),
            BinaryOp::Div => write!(f, "/"),
            BinaryOp::Rem => write!(f, "%"),
            BinaryOp::Eq => write!(f, "=="),
            BinaryOp::Ne => write!(f, "!="),
            BinaryOp::Lt => write!(f, "<"),
            BinaryOp::Le => write!(f, "<="),
            BinaryOp::Gt => write!(f, ">"),
            BinaryOp::Ge => write!(f, ">="),
            BinaryOp::And => write!(f, "&&"),
            BinaryOp::Or => write!(f, "||"),
        }
    }
}

impl fmt::Display for UnaryOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UnaryOp::Neg => write!(f, "-"),
            UnaryOp::Not => write!(f, "!"),
        }
    }
}

impl fmt::Display for Literal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Literal::Number(n, _) => {
                if n.fract() == 0.0 && n.abs() < 1e15 {
                    write!(f, "{}", *n as i64)
                } else {
                    write!(f, "{}", n)
                }
            }
            Literal::String(s, _) => write!(f, "\"{}\"", s),
            Literal::Bool(b, _) => write!(f, "{}", b),
            Literal::Color(c, _) => write!(f, "{}", c),
        }
    }
}

impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Expr::Literal(lit) => write!(f, "{}", lit),
            Expr::Ident(id) => write!(f, "{}", id),
            Expr::MemberAccess(m) => write!(f, "{}.{}", m.target, m.member),
            Expr::Ternary(t) => write!(f, "{} ? {} : {}", t.condition, t.then_expr, t.else_expr),
            Expr::Call(c) => {
                write!(f, "{}(", c.callee)?;
                for (i, arg) in c.args.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", arg)?;
                }
                write!(f, ")")
            }
            Expr::Binary(b) => write!(f, "{} {} {}", b.left, b.op, b.right),
            Expr::Unary(u) => write!(f, "{}{}", u.op, u.operand),
            Expr::Paren(inner, _) => write!(f, "({})", inner),
            Expr::Node(node) => write!(f, "\\{}(...)", node.name),
        }
    }
}
