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
    State(StateBinding),
    Use(UseDeclaration),
}

impl Item {
    pub fn span(&self) -> Span {
        match self {
            Item::Component(c) => c.span,
            Item::Node(n) => n.span,
            Item::Let(l) => l.span,
            Item::Env(e) => e.span,
            Item::State(s) => s.span,
            Item::Use(u) => u.span,
        }
    }
}

/// Module import declaration: `\use "./components/Button.dt" [as Alias];`
#[derive(Debug, Clone, PartialEq)]
pub struct UseDeclaration {
    pub path: String,
    pub alias: Option<Ident>,
    pub span: Span,
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
    State(StateBinding),
}

impl ComponentBodyItem {
    pub fn span(&self) -> Span {
        match self {
            ComponentBodyItem::Node(n) => n.span,
            ComponentBodyItem::Children(c) => c.span,
            ComponentBodyItem::Let(l) => l.span,
            ComponentBodyItem::Env(e) => e.span,
            ComponentBodyItem::Alias(a) => a.span,
            ComponentBodyItem::State(s) => s.span,
        }
    }
}

/// Reactive state variable binding: `state name = expr;`, `state name: Type = expr;`, `state name: Type: expr;`, or uninitialized `state name: Type;` / `state name;`
#[derive(Debug, Clone, PartialEq)]
pub struct StateBinding {
    pub name: Ident,
    pub type_annotation: Option<TypeRef>,
    pub default: Option<Expr>,
    pub span: Span,
}

/// Component identity key: structured list of expressions, e.g. `(row, col)` or `"submit_btn"`
#[derive(Debug, Clone)]
pub struct ComponentKey {
    pub parts: Vec<Expr>,
    pub span: Span,
}

impl ComponentKey {
    pub fn new(parts: Vec<Expr>, span: Span) -> Self {
        Self { parts, span }
    }

    pub fn single(expr: Expr) -> Self {
        let span = expr.span();
        Self {
            parts: vec![expr],
            span,
        }
    }

    pub fn string(s: impl Into<String>) -> Self {
        Self {
            parts: vec![Expr::Literal(Literal::String(s.into(), Span::default()))],
            span: Span::default(),
        }
    }

    pub fn number(n: f64) -> Self {
        Self {
            parts: vec![Expr::Literal(Literal::Number(n, Span::default()))],
            span: Span::default(),
        }
    }

    pub fn tuple(parts: &[Expr]) -> Self {
        let span = parts.first().map(|p| p.span()).unwrap_or_default();
        Self {
            parts: parts.to_vec(),
            span,
        }
    }
}

impl PartialEq for ComponentKey {
    fn eq(&self, other: &Self) -> bool {
        self.parts.len() == other.parts.len()
            && self
                .parts
                .iter()
                .zip(&other.parts)
                .all(|(a, b)| expr_eq_ignore_span(a, b))
    }
}

impl Eq for ComponentKey {}

impl fmt::Display for ComponentKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(")?;
        for (i, p) in self.parts.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{}", p)?;
        }
        write!(f, ")")
    }
}

/// Helper to compare two AST expressions for semantic equivalence, ignoring source spans.
pub fn expr_eq_ignore_span(a: &Expr, b: &Expr) -> bool {
    match (a, b) {
        (Expr::Literal(l1), Expr::Literal(l2)) => match (l1, l2) {
            (Literal::Number(n1, _), Literal::Number(n2, _)) => (n1 - n2).abs() < f64::EPSILON,
            (Literal::String(s1, _), Literal::String(s2, _)) => s1 == s2,
            (Literal::Bool(b1, _), Literal::Bool(b2, _)) => b1 == b2,
            (Literal::Color(c1, _), Literal::Color(c2, _)) => c1 == c2,
            _ => false,
        },
        (Expr::Ident(id1), Expr::Ident(id2)) => id1.as_str() == id2.as_str(),
        (Expr::MemberAccess(m1), Expr::MemberAccess(m2)) => {
            m1.member.as_str() == m2.member.as_str() && expr_eq_ignore_span(&m1.target, &m2.target)
        }
        (Expr::Binary(b1), Expr::Binary(b2)) => {
            b1.op == b2.op
                && expr_eq_ignore_span(&b1.left, &b2.left)
                && expr_eq_ignore_span(&b1.right, &b2.right)
        }
        (Expr::Unary(u1), Expr::Unary(u2)) => {
            u1.op == u2.op && expr_eq_ignore_span(&u1.operand, &u2.operand)
        }
        (Expr::Paren(p1, _), other) => expr_eq_ignore_span(p1, other),
        (other, Expr::Paren(p2, _)) => expr_eq_ignore_span(other, p2),
        (Expr::Ternary(t1), Expr::Ternary(t2)) => {
            expr_eq_ignore_span(&t1.condition, &t2.condition)
                && expr_eq_ignore_span(&t1.then_expr, &t2.then_expr)
                && expr_eq_ignore_span(&t1.else_expr, &t2.else_expr)
        }
        (Expr::Call(c1), Expr::Call(c2)) => {
            c1.callee.as_str() == c2.callee.as_str()
                && c1.args.len() == c2.args.len()
                && c1
                    .args
                    .iter()
                    .zip(&c2.args)
                    .all(|(a1, a2)| expr_eq_ignore_span(a1, a2))
        }
        _ => false,
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

/// Element node instantiation: `\Name [ ( [ key ; ] ports ) ] [ { content } ]`
#[derive(Debug, Clone, PartialEq)]
pub struct ElementNode {
    pub name: Ident,
    pub key: Option<ComponentKey>,
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
