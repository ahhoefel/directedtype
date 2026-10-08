pub mod color;
pub mod error;
pub mod headless;
pub mod scene;
pub mod viewer;

pub use color::parse_color;
pub use error::RenderError;
pub use headless::HeadlessRenderer;
pub use scene::{
    build_scene, build_scene_without_cache, CachedClipScene, CachedTextScene, ClipSceneCache,
    SceneOptions, SpanRenderKey, TextRenderKey, TextSceneCache,
};
pub use viewer::{
    run_viewer, run_viewer_with_document, run_viewer_with_file,
    run_viewer_with_file_and_registry, PendingScroll, ViewerApp, ViewerConfig, ViewerUserEvent,
};
