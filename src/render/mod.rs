#[cfg(feature = "vello-renderer")]
pub mod browser_bar;
pub mod color;
#[cfg(feature = "vello-renderer")]
pub mod error;
#[cfg(feature = "vello-renderer")]
pub mod headless;
#[cfg(feature = "vello-renderer")]
pub mod image;
#[cfg(feature = "vello-renderer")]
pub mod scene;
#[cfg(feature = "vello-renderer")]
pub mod viewer;

#[cfg(feature = "pdf")]
pub use crate::pdf;
#[cfg(feature = "vello-renderer")]
pub use browser_bar::{BrowserBar, BrowserBarHitResult, NavigationHistory, UrlBarState};
pub use color::{parse_color_rgba8, Rgba8};
#[cfg(feature = "vello-renderer")]
pub use color::parse_color;
#[cfg(feature = "vello-renderer")]
pub use error::RenderError;
#[cfg(feature = "vello-renderer")]
pub use headless::HeadlessRenderer;
#[cfg(feature = "vello-renderer")]
pub use scene::{
    build_scene, build_scene_without_cache, CachedClipScene, CachedTextScene, ClipSceneCache,
    SceneOptions, SpanRenderKey, TextRenderKey, TextSceneCache,
};
#[cfg(feature = "vello-renderer")]
pub use viewer::{
    run_viewer, run_viewer_with_compiled_file, run_viewer_with_document, run_viewer_with_file,
    run_viewer_with_file_and_registry, PendingScroll, ViewerApp, ViewerConfig, ViewerUserEvent,
};

