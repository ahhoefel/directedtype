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
}

impl Item {
    pub fn span(&self) -> Span {
        match self {
            Item::Component(c) => c.span,
            Item::Node(n) => n.span,
            Item::Let(l) => l.span,
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
}

impl ComponentBodyItem {
    pub fn span(&self) -> Span {
        match self {
            ComponentBodyItem::Node(n) => n.span,
            ComponentBodyItem::Children(c) => c.span,
            ComponentBodyItem::Let(l) => l.span,
        }
    }
}

/// Local variable or element binding: `let name = expr;` or `let name = \Node(...);`
#[derive(Debug, Clone, PartialEq)]
pub struct LetBinding {
    pub name: Ident,
    pub type_annotation: Option<TypeRef>,
    pub value: LetValue,
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

/// Parameter definition in a component signature:
/// e.g. `bg_color: Color`, `gap: Number: 16`, or `width: max(children.width) + 32`
#[derive(Debug, Clone, PartialEq)]
pub struct ParamDef {
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
