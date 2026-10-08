pub mod components;
pub mod model;
pub mod overlay;
pub mod panel;
pub mod state;
pub mod view;

pub use components::{
    standard_inspector_components, INSPECT_BADGE_DTML, INSPECT_CLIP_GUIDE_DTML,
    INSPECT_HIGHLIGHT_DTML, INSPECT_OVERLAY_DTML,
};
pub use model::{
    build_property_dag_trace, format_dag_expr, BoxModelValues, DagPropertyTrace,
    DagTraceDependency, InspectTargetInfo, PortEquationEntry, PortOriginKind,
};
pub use overlay::{InspectOverlayComponent, InspectOverlayStyle};
pub use panel::{InspectPanelComponent, PanelHitResult};
pub use state::{InspectorState, InspectorTab};
pub use view::{
    build_tree_items, build_tree_items_from_layout, extract_box_model, extract_port_equations,
    DomTreeItem,
};
