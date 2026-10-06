pub mod ast;
pub mod compiler;
pub mod component;
pub mod dom;
pub mod error;
pub mod inspector;
pub mod interaction;
pub mod lexer;
pub mod parser;
pub mod render;
pub mod span;
pub mod token;

pub use ast::Document;
pub use component::{
    Component, ComponentFactory, ComponentRegistry, Context, DispatchError, EventHandlerBinding,
    EventHandlerTarget, InstanceManager,
};
pub use compiler::{
    evaluate_document, evaluate_document_with_window, project_inline_fragments, CursorKind, Rect,
    ResolvedLayout, ResolvedNode, SpanStyle, TextSpan, Value,
};
pub use dom::{Dom, DomError, DomHitTestResult, NodeHandle, Transaction};
pub use error::ParseError;
pub use inspector::{
    InspectOverlayComponent, InspectOverlayStyle, InspectPanelComponent, InspectTargetInfo,
    InspectorState,
};
pub use interaction::{Event, EventKind, HitTestResult, Modifiers, MouseButton, Point};
pub use parser::parse_document as parse;
