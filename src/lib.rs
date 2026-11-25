pub mod ast;
pub mod compdoc;
pub mod compiler;
pub mod component;
#[cfg(feature = "vello-renderer")]
pub mod docgen;
pub mod dom;
pub mod error;
#[cfg(feature = "vello-renderer")]
pub mod inspector;
pub mod interaction;
pub mod lexer;
pub mod parser;
#[cfg(feature = "pdf")]
pub mod pdf;
pub mod render;
pub mod span;
pub mod symbol;
pub mod token;

extern crate self as directedtype;

pub use ast::Document;
pub use component::{
    Callback, Component, ComponentFactory, ComponentRegistry, Context, DispatchError, EmitCallback,
    EventHandlerBinding, EventHandlerTarget, FromPayload, InstanceManager, PrepareContext,
};
pub use directedtype_macros::component;
pub use compiler::{
    evaluate_document, evaluate_document_with_window, project_inline_fragments, CursorKind,
    IndefiniteTimer, Rect, ResolvedLayout, ResolvedNode, SpanStyle, TextSpan, Timer, Value,
};
pub use dom::{Dom, DomError, DomHitTestResult, NodeHandle, Transaction};
pub use error::ParseError;
#[cfg(feature = "vello-renderer")]
pub use inspector::{
    InspectOverlayComponent, InspectOverlayStyle, InspectPanelComponent, InspectorScrollbarKind,
    InspectTargetInfo, InspectorState,
};
pub use interaction::{Event, EventKind, HitTestResult, Modifiers, MouseButton, Point};
pub use parser::parse_document as parse;

pub mod prelude {
    pub use crate::compiler::{IndefiniteTimer, Timer, Value};
    pub use crate::component::{
        Callback, Component, Context, DispatchError, DomLinkInfo, EmitCallback, FromPayload,
        GuestDomError, GuestNodeHandle, GuestValue, PrepareContext,
    };
    pub use crate::interaction::{Event, EventKind, Modifiers, MouseButton, Point};
    pub use directedtype_macros::component;
}
