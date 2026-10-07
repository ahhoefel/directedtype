use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::Arc;

use notify::{Event as NotifyEvent, RecommendedWatcher, RecursiveMode, Watcher};
use parley::{FontContext, LayoutContext};
use vello::peniko::Color;
use vello::util::{RenderContext, RenderSurface};
use vello::wgpu;
use vello::{AaConfig, AaSupport, RenderParams, Renderer, RendererOptions, Scene};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

use crate::ast::Document;
use crate::component::ComponentRegistry;
use crate::compiler::evaluate_document_with_window;
use crate::compiler::expanded::NodeId;
use crate::compiler::layout::ResolvedLayout;
use crate::compiler::module::FsResolver;
use crate::compiler::CompiledDocument;
use crate::interaction::{Event, EventKind, Modifiers, MouseButton, Point};
use crate::parser::parse_document;
use crate::render::scene::{build_scene, SceneOptions};

#[cfg(target_os = "macos")]
fn configure_metal_layer(window: &Window) {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    if let Ok(handle) = window.window_handle() {
        if let RawWindowHandle::AppKit(appkit_handle) = handle.as_raw() {
            unsafe {
                let view = appkit_handle.ns_view.as_ptr() as *mut AnyObject;

                // 1. Configure NSView:
                // NSViewLayerContentsRedrawDuringViewResize = 2
                // NSViewLayerContentsPlacementTopLeft = 11
                let _: () = msg_send![view, setLayerContentsRedrawPolicy: 2isize];
                let _: () = msg_send![view, setLayerContentsPlacement: 11isize];

                let root_layer: *mut AnyObject = msg_send![view, layer];
                if !root_layer.is_null() {
                    let scale = window.scale_factor();
                    apply_layer_config(root_layer, scale);
                }
            }
        }
    }
}

#[cfg(target_os = "macos")]
unsafe fn apply_layer_config(layer: *mut objc2::runtime::AnyObject, scale: f64) {
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send};
    use std::ffi::CStr;

    let s: *const CStr = c"topLeft";
    let ns_gravity: *const AnyObject =
        msg_send![class!(NSString), stringWithUTF8String: s.cast::<std::ffi::c_char>()];

    let _: () = msg_send![layer, setContentsGravity: ns_gravity];
    let _: () = msg_send![layer, setContentsScale: scale];

    let pwt_sel = objc2::sel!(setPresentsWithTransaction:);
    if msg_send![layer, respondsToSelector: pwt_sel] {
        let _: () = msg_send![layer, setPresentsWithTransaction: true];
    }

    let sublayers: *mut AnyObject = msg_send![layer, sublayers];
    if !sublayers.is_null() {
        let count: usize = msg_send![sublayers, count];
        for i in 0..count {
            let sublayer: *mut AnyObject = msg_send![sublayers, objectAtIndex: i];
            apply_layer_config(sublayer, scale);
        }
    }
}

#[cfg(target_os = "macos")]
fn flush_metal_transaction() {
    unsafe {
        use objc2::{class, msg_send};
        let _: () = msg_send![class!(CATransaction), flush];
    }
}

#[cfg(not(target_os = "macos"))]
fn flush_metal_transaction() {}

#[cfg(not(target_os = "macos"))]
fn configure_metal_layer(_window: &Window) {}

/// Configuration options for the interactive viewer.
#[derive(Debug, Clone)]
pub struct ViewerConfig {
    pub title: String,
    pub width: u32,
    pub height: u32,
    pub scene_options: SceneOptions,
}

impl Default for ViewerConfig {
    fn default() -> Self {
        Self {
            title: "DirectedType Viewer".into(),
            width: 1024,
            height: 768,
            scene_options: SceneOptions {
                background: Some(Color::WHITE),
                scale_factor: 1.0,
                scroll_offset: (0.0, 0.0),
            },
        }
    }
}

/// Custom event sent to the Winit event loop from background watcher threads.
#[derive(Debug, Clone)]
pub enum ViewerUserEvent {
    FileModified,
}

/// Interactive window viewer application using Winit and Vello.
pub struct ViewerApp {
    config: ViewerConfig,
    watch_path: Option<PathBuf>,
    _watcher: Option<RecommendedWatcher>,
    doc: Option<Document>,
    compiled: Option<CompiledDocument>,
    component_registry: ComponentRegistry,
    layout: ResolvedLayout,
    render_cx: RenderContext,
    surface: Option<RenderSurface<'static>>,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    font_cx: FontContext,
    layout_cx: LayoutContext<()>,
    frame_count: u64,

    // Interaction state
    cursor_pos: Option<Point>,
    hovered_node: Option<NodeId>,
    pressed_node: Option<(NodeId, MouseButton)>,
    modifiers: Modifiers,
    inspect_mode: bool,
    selected_node: Option<NodeId>,
    event_handler: Option<EventHandler>,
    inspector_state: crate::inspector::InspectorState,
    panel_component: crate::inspector::InspectPanelComponent,

    // Scrolling state
    scroll_x: f64,
    scroll_y: f64,
    is_dragging_scrollbar: bool,
    scrollbar_drag_start_y: f64,
    scrollbar_start_scroll_y: f64,
}

/// Type alias for event callbacks dispatched by `ViewerApp`.
pub type EventHandler = Box<dyn FnMut(&mut Event, &ResolvedLayout)>;

impl ViewerApp {
    pub fn new(layout: ResolvedLayout, config: ViewerConfig) -> Self {
        Self {
            config,
            watch_path: None,
            _watcher: None,
            doc: None,
            compiled: None,
            component_registry: ComponentRegistry::new(),
            layout,
            render_cx: RenderContext::new(),
            surface: None,
            window: None,
            renderer: None,
            font_cx: FontContext::new(),
            layout_cx: LayoutContext::new(),
            frame_count: 0,
            cursor_pos: None,
            hovered_node: None,
            pressed_node: None,
            modifiers: Modifiers::default(),
            inspect_mode: false,
            selected_node: None,
            event_handler: None,
            inspector_state: crate::inspector::InspectorState::new(),
            panel_component: crate::inspector::InspectPanelComponent::default(),
            scroll_x: 0.0,
            scroll_y: 0.0,
            is_dragging_scrollbar: false,
            scrollbar_drag_start_y: 0.0,
            scrollbar_start_scroll_y: 0.0,
        }
    }

    /// Registers a callback to receive high-level interaction events.
    pub fn on_event<F: FnMut(&mut Event, &ResolvedLayout) + 'static>(mut self, handler: F) -> Self {
        self.event_handler = Some(Box::new(handler));
        self
    }

    /// Dynamically sets or updates the interaction event callback.
    pub fn set_event_handler<F: FnMut(&mut Event, &ResolvedLayout) + 'static>(&mut self, handler: F) {
        self.event_handler = Some(Box::new(handler));
    }

    /// Toggles the interactive visual inspector overlay.
    pub fn set_inspect_mode(&mut self, enabled: bool) {
        self.inspect_mode = enabled;
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// Returns whether inspect mode is currently active.
    pub fn inspect_mode(&self) -> bool {
        self.inspect_mode
    }

    /// Returns the currently selected node in inspector mode, if any.
    pub fn selected_node(&self) -> Option<NodeId> {
        self.selected_node
    }

    /// Returns the currently hovered visual node, if any.
    pub fn hovered_node(&self) -> Option<NodeId> {
        self.hovered_node
    }

    /// Returns whether the spatial element picker cursor is actively picking elements.
    pub fn inspect_cursor_active(&self) -> bool {
        self.inspector_state.inspect_cursor_active
    }

    /// Sets whether the spatial element picker cursor is actively picking elements.
    pub fn set_inspect_cursor_active(&mut self, active: bool) {
        self.inspector_state.inspect_cursor_active = active;
        if !active {
            self.inspector_state.set_hovered_id(None);
        }
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// Returns a reference to the internal inspector state.
    pub fn inspector_state(&self) -> &crate::inspector::InspectorState {
        &self.inspector_state
    }

    /// Returns a mutable reference to the internal inspector state.
    pub fn inspector_state_mut(&mut self) -> &mut crate::inspector::InspectorState {
        &mut self.inspector_state
    }

    /// Returns the current vertical scroll offset.
    pub fn scroll_y(&self) -> f64 {
        self.scroll_y
    }

    /// Sets the vertical scroll offset directly.
    pub fn set_scroll_y(&mut self, y: f64) {
        self.scroll_y = y.max(0.0);
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// Returns the current horizontal scroll offset.
    pub fn scroll_x(&self) -> f64 {
        self.scroll_x
    }

    /// Sets the horizontal scroll offset directly.
    pub fn set_scroll_x(&mut self, x: f64) {
        self.scroll_x = x.max(0.0);
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// Returns the current scroll offset `(scroll_x, scroll_y)`.
    pub fn scroll_offset(&self) -> (f64, f64) {
        (self.scroll_x, self.scroll_y)
    }

    /// Sets the scroll offset directly.
    pub fn set_scroll_offset(&mut self, x: f64, y: f64) {
        self.scroll_x = x.max(0.0);
        self.scroll_y = y.max(0.0);
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// Computes the total content width across all layout nodes.
    pub fn content_width(&self) -> f64 {
        let mut max_x: f64 = 0.0;
        for node in &self.layout.nodes {
            max_x = max_x.max(node.rect.x + node.rect.width);
        }
        max_x
    }

    /// Computes the total content height across all layout nodes.
    pub fn content_height(&self) -> f64 {
        let mut max_y: f64 = 0.0;
        for node in &self.layout.nodes {
            max_y = max_y.max(node.rect.y + node.rect.height);
        }
        max_y
    }

    /// Computes the maximum vertical scroll offset for a given viewport height.
    pub fn max_scroll_y(&self, viewport_h: f64) -> f64 {
        (self.content_height() - viewport_h).max(0.0)
    }

    /// Scrolls vertically to a target Y position, clamping to valid scroll range.
    pub fn scroll_to_y(&mut self, target_y: f64) {
        let scale = self.window.as_ref().map(|w| w.scale_factor()).unwrap_or(1.0);
        let win_h = if let Some(w) = &self.window {
            let size = w.inner_size();
            size.height as f64 / scale
        } else {
            self.config.height as f64
        };
        let max_scroll = self.max_scroll_y(win_h);
        self.scroll_y = target_y.clamp(0.0, max_scroll);
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// Scrolls to bring a specific node into view at the top of the viewport.
    pub fn scroll_to_node(&mut self, node_id: NodeId) -> bool {
        if let Some(node) = self.layout.get_node(node_id) {
            let target_y = if !node.fragments.is_empty() {
                node.fragments[0].y
            } else {
                node.rect.y
            };
            self.scroll_to_y(target_y);
            true
        } else {
            false
        }
    }

    /// Resolves an anchor link (e.g. `"#section"` or `"#/scope/target"`) and scrolls the window to it.
    pub fn scroll_to_anchor(&mut self, source_node: Option<NodeId>, url: &str) -> bool {
        let from_node = source_node.unwrap_or(NodeId(0));
        if let Some((target_id, _scope_id)) = self.layout.resolve_anchor(from_node, url) {
            self.scroll_to_node(target_id)
        } else {
            false
        }
    }

    fn dispatch_event_with_bubble(&mut self, mut event: Event, bubble_path: &[NodeId]) {
        event.bubble_path = bubble_path.to_vec();

        if let Some(compiled) = &mut self.compiled {
            if let Ok(changed_vars) = compiled.dispatch_event(&mut event) {
                if !changed_vars.is_empty() {
                    self.layout = compiled.layout().clone();
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                }
            }
        }

        if let Some(handler) = &mut self.event_handler {
            for &ancestor_id in bubble_path {
                event.current_target = ancestor_id;
                handler(&mut event, &self.layout);
                if event.propagation_stopped {
                    break;
                }
            }
        }
    }

    /// Attaches the source AST `Document` to enable dynamic layout re-evaluation on window resize.
    pub fn with_document(mut self, doc: Document) -> Self {
        self.doc = Some(doc);
        self
    }

    /// Attaches an active `CompiledDocument` to power reactive state mutations and event dispatching.
    pub fn with_compiled(mut self, compiled: CompiledDocument) -> Self {
        self.layout = compiled.layout().clone();
        self.compiled = Some(compiled);
        self
    }

    /// Attaches a `ComponentRegistry` for companion component lifecycle and dispatching.
    pub fn with_registry(mut self, registry: ComponentRegistry) -> Self {
        self.component_registry = registry;
        self
    }

    /// Returns a reference to the active `CompiledDocument`, if present.
    pub fn compiled(&self) -> Option<&CompiledDocument> {
        self.compiled.as_ref()
    }

    /// Returns a mutable reference to the active `CompiledDocument`, if present.
    pub fn compiled_mut(&mut self) -> Option<&mut CompiledDocument> {
        self.compiled.as_mut()
    }

    /// Returns a reference to the current resolved layout.
    pub fn layout(&self) -> &ResolvedLayout {
        &self.layout
    }

    /// Returns a reference to the current AST document, if available.
    pub fn doc(&self) -> Option<&Document> {
        self.doc.as_ref()
    }

    /// Attaches a file path to watch for live changes.
    pub fn with_watch_path(mut self, path: PathBuf) -> Self {
        self.watch_path = Some(path);
        self
    }

    fn update_layout_for_size(&mut self, logical_w: f64, logical_h: f64) {
        if let Some(doc) = &self.doc {
            if let Some(compiled) = &mut self.compiled {
                let base_dir = self
                    .watch_path
                    .as_deref()
                    .and_then(|p| p.parent())
                    .unwrap_or_else(|| std::path::Path::new("."));
                let resolver = FsResolver;
                if let Ok(mut new_compiled) = CompiledDocument::compile_with_registry(
                    doc,
                    logical_w,
                    logical_h,
                    base_dir,
                    &resolver,
                    &self.component_registry,
                ) {
                    for (var_id, val) in &compiled.state_overrides {
                        let _ = new_compiled.set_state(var_id.node, &var_id.port, val.clone());
                    }
                    self.layout = new_compiled.layout().clone();
                    *compiled = new_compiled;
                }
            } else if let Ok(new_layout) = evaluate_document_with_window(doc, logical_w, logical_h) {
                self.layout = new_layout;
            }
        }
    }

    /// Reloads the document from disk and re-renders if parsing and layout evaluation succeed.
    pub fn reload_document(&mut self) {
        let path = match &self.watch_path {
            Some(p) => p.clone(),
            None => return,
        };

        let source = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[HotReload] Error reading file '{}': {e}", path.display());
                return;
            }
        };

        let new_doc = match parse_document(&source) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("[HotReload] Parse error in '{}':\n{e}", path.display());
                return;
            }
        };

        let (logical_w, logical_h) = if let Some(window) = &self.window {
            let scale = window.scale_factor();
            let size = window.inner_size();
            if size.width > 0 && size.height > 0 {
                (size.width as f64 / scale, size.height as f64 / scale)
            } else {
                (self.config.width as f64, self.config.height as f64)
            }
        } else {
            (self.config.width as f64, self.config.height as f64)
        };

        if let Some(compiled) = &mut self.compiled {
            let base_dir = path.parent().unwrap_or_else(|| std::path::Path::new("."));
            let resolver = FsResolver;
            match CompiledDocument::compile_with_registry(
                &new_doc,
                logical_w,
                logical_h,
                base_dir,
                &resolver,
                &self.component_registry,
            ) {
                Ok(mut new_compiled) => {
                    for (var_id, val) in &compiled.state_overrides {
                        let _ = new_compiled.set_state(var_id.node, &var_id.port, val.clone());
                    }
                    println!(
                        "[HotReload] Successfully reloaded '{}' ({} resolved nodes)",
                        path.display(),
                        new_compiled.layout().nodes.len()
                    );
                    self.doc = Some(new_doc);
                    self.layout = new_compiled.layout().clone();
                    *compiled = new_compiled;
                    self.render_frame();
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                }
                Err(e) => {
                    eprintln!("[HotReload] Compile error in '{}':\n{e}", path.display());
                }
            }
        } else {
            match evaluate_document_with_window(&new_doc, logical_w, logical_h) {
                Ok(new_layout) => {
                    println!(
                        "[HotReload] Successfully reloaded '{}' ({} resolved nodes)",
                        path.display(),
                        new_layout.nodes.len()
                    );
                    self.doc = Some(new_doc);
                    self.layout = new_layout;
                    self.render_frame();
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                }
                Err(e) => {
                    eprintln!("[HotReload] Layout evaluation error in '{}':\n{e}", path.display());
                }
            }
        }
    }

    /// Synchronously renders a frame to the current swapchain texture and presents it.
    pub fn render_frame(&mut self) {
        let (surface, renderer, window) = match (
            &mut self.surface,
            &mut self.renderer,
            &self.window,
        ) {
            (Some(s), Some(r), Some(w)) => (s, r, w),
            _ => return,
        };

        let width = surface.config.width;
        let height = surface.config.height;
        if width == 0 || height == 0 {
            return;
        }

        // 1. Acquire current surface texture first, handling Occluded and Outdated gracefully
        let surface_texture = match surface.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(st)
            | wgpu::CurrentSurfaceTexture::Suboptimal(st) => st,
            wgpu::CurrentSurfaceTexture::Occluded => {
                // Window is occluded on macOS startup; schedule redraw to present as soon as Cocoa maps it
                window.request_redraw();
                return;
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.render_cx.configure_surface(surface);
                match surface.surface.get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Success(st)
                    | wgpu::CurrentSurfaceTexture::Suboptimal(st) => st,
                    _ => {
                        window.request_redraw();
                        return;
                    }
                }
            }
            wgpu::CurrentSurfaceTexture::Timeout => {
                // Transient timeout during rapid live resize drag; defer to next frame
                window.request_redraw();
                return;
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                self.render_cx.configure_surface(surface);
                window.request_redraw();
                return;
            }
            _ => return,
        };

        let device_handle = &self.render_cx.devices[surface.dev_id];

        let mut scene_opts = self.config.scene_options.clone();
        scene_opts.scale_factor = window.scale_factor();
        scene_opts.scroll_offset = (self.scroll_x, self.scroll_y);

        let mut scene = build_scene(
            &self.layout,
            &mut self.font_cx,
            &mut self.layout_cx,
            &scene_opts,
        );

        let scale = window.scale_factor();
        let size = window.inner_size();
        let win_w = if size.width > 0 {
            size.width as f64 / scale
        } else {
            self.config.width as f64
        };
        let win_h = if size.height > 0 {
            size.height as f64 / scale
        } else {
            self.config.height as f64
        };

        let canvas_w = if self.inspect_mode {
            (win_w - self.panel_component.width).max(100.0)
        } else {
            win_w
        };

        if self.inspect_mode {
            let overlay = crate::inspector::InspectOverlayComponent::default();
            let overlay_transform = vello::kurbo::Affine::translate((-self.scroll_x, -self.scroll_y));
            let panel_w = self.panel_component.width;
            let panel_x = win_w - panel_w;

            let mut overlay_scene = Scene::new();

            // Render selected node (if distinct from hovered)
            if let Some(selected_id) = self.selected_node {
                if self.inspector_state.hovered_id != Some(selected_id) {
                    if let Some(info) = crate::inspector::InspectTargetInfo::from_layout(&self.layout, selected_id) {
                        overlay.render_to_scene(
                            &mut overlay_scene,
                            overlay_transform,
                            &info,
                            true,
                            &mut self.font_cx,
                            &mut self.layout_cx,
                            canvas_w,
                            win_h,
                        );
                    }
                }
            }

            // Render hovered node
            if let Some(hovered_id) = self.inspector_state.hovered_id {
                if let Some(info) = crate::inspector::InspectTargetInfo::from_layout(&self.layout, hovered_id) {
                    let is_selected = self.selected_node == Some(hovered_id);
                    overlay.render_to_scene(
                        &mut overlay_scene,
                        overlay_transform,
                        &info,
                        is_selected,
                        &mut self.font_cx,
                        &mut self.layout_cx,
                        canvas_w,
                        win_h,
                    );
                }
            }

            // Build and render docked DOM inspector side panel
            let tree_items = crate::inspector::build_tree_items_from_layout(
                &self.layout,
                &self.inspector_state,
            );
            let mut panel_scene = Scene::new();
            self.panel_component.render_to_scene(
                &mut panel_scene,
                panel_x,
                win_h,
                &tree_items,
                &self.inspector_state,
                &self.layout,
                &mut self.font_cx,
                &mut self.layout_cx,
            );

            if (scale - 1.0).abs() > 0.001 {
                let scale_affine = Some(vello::kurbo::Affine::scale(scale));
                scene.append(&overlay_scene, scale_affine);
                scene.append(&panel_scene, scale_affine);
            } else {
                scene.append(&overlay_scene, None);
                scene.append(&panel_scene, None);
            }
        }

        // Render subtle scrollbar thumb if document height exceeds viewport
        let content_h = self
            .layout
            .nodes
            .iter()
            .fold(0.0f64, |acc, n| acc.max(n.rect.y + n.rect.height));
        if content_h > win_h {
            let max_scroll = (content_h - win_h).max(1.0);
            let track_h = win_h;
            let thumb_h = ((win_h / content_h) * track_h).max(24.0).min(track_h);
            let scroll_ratio = (self.scroll_y / max_scroll).clamp(0.0, 1.0);
            let thumb_y = scroll_ratio * (track_h - thumb_h);
            let thumb_w = 6.0;
            let thumb_x = canvas_w - thumb_w - 3.0;

            let mut scrollbar_scene = Scene::new();
            let thumb_rrect = vello::kurbo::RoundedRect::new(
                thumb_x,
                thumb_y,
                thumb_x + thumb_w,
                thumb_y + thumb_h,
                3.0,
            );
            let thumb_color = Color::from_rgba8(120, 120, 128, 140);
            scrollbar_scene.fill(
                vello::peniko::Fill::NonZero,
                vello::kurbo::Affine::IDENTITY,
                vello::peniko::Brush::Solid(thumb_color),
                None,
                &thumb_rrect,
            );

            if (scale - 1.0).abs() > 0.001 {
                scene.append(&scrollbar_scene, Some(vello::kurbo::Affine::scale(scale)));
            } else {
                scene.append(&scrollbar_scene, None);
            }
        }

        let base_color = self
            .config
            .scene_options
            .background
            .unwrap_or(Color::WHITE);

        let render_result = renderer.render_to_texture(
            &device_handle.device,
            &device_handle.queue,
            &scene,
            &surface.target_view,
            &RenderParams {
                base_color,
                width,
                height,
                antialiasing_method: AaConfig::Area,
            },
        );

        if let Err(e) = render_result {
            eprintln!("Render error: {e}");
            return;
        }

        let surface_view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = device_handle
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("DirectedType Blitter Encoder"),
            });

        surface.blitter.copy(
            &device_handle.device,
            &mut encoder,
            &surface.target_view,
            &surface_view,
        );

        device_handle.queue.submit(Some(encoder.finish()));
        surface_texture.present();
        flush_metal_transaction();
        self.frame_count += 1;
    }
}

impl ApplicationHandler<ViewerUserEvent> for ViewerApp {
    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: ViewerUserEvent) {
        match event {
            ViewerUserEvent::FileModified => {
                self.reload_document();
            }
        }
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let window_attrs = Window::default_attributes()
            .with_title(&self.config.title)
            .with_inner_size(LogicalSize::new(
                self.config.width as f64,
                self.config.height as f64,
            ));

        let window = match event_loop.create_window(window_attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                eprintln!("Failed to create window: {e}");
                event_loop.exit();
                return;
            }
        };

        // On macOS, inner_size() right after creation can return 0 before the window is mapped.
        // Fall back to config dimensions scaled by scale_factor so surface is never 1x1.
        let scale_factor = window.scale_factor();
        let inner_size = window.inner_size();
        let width = if inner_size.width > 0 {
            inner_size.width
        } else {
            (self.config.width as f64 * scale_factor).round() as u32
        }.max(1);
        let height = if inner_size.height > 0 {
            inner_size.height
        } else {
            (self.config.height as f64 * scale_factor).round() as u32
        }.max(1);

        let surface = match pollster::block_on(self.render_cx.create_surface(
            window.clone(),
            width,
            height,
            wgpu::PresentMode::AutoNoVsync,
        )) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Failed to create render surface: {e}");
                event_loop.exit();
                return;
            }
        };

        let device_handle = &self.render_cx.devices[surface.dev_id];
        let renderer = match Renderer::new(
            &device_handle.device,
            RendererOptions {
                use_cpu: false,
                antialiasing_support: AaSupport::all(),
                num_init_threads: NonZeroUsize::new(1),
                pipeline_cache: None,
            },
        ) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("Failed to initialize Vello renderer: {e}");
                event_loop.exit();
                return;
            }
        };

        // Configure CAMetalLayer: presentsWithTransaction = true, contentsGravity = topLeft, contentsScale
        configure_metal_layer(&window);

        let logical_w = width as f64 / scale_factor;
        let logical_h = height as f64 / scale_factor;
        self.update_layout_for_size(logical_w, logical_h);

        self.renderer = Some(renderer);
        self.surface = Some(surface);
        self.window = Some(window);

        // Attempt initial frame
        self.render_frame();
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        // Keep requesting redraw on startup until window un-occludes and renders initial frame
        if self.frame_count == 0 {
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
            }
            WindowEvent::Occluded(is_occluded) => {
                if !is_occluded {
                    self.render_frame();
                }
            }
            WindowEvent::Resized(size) => {
                if size.width > 0 && size.height > 0 {
                    if let Some(surface) = &mut self.surface {
                        self.render_cx
                            .resize_surface(surface, size.width, size.height);
                    }
                    if let Some(window) = &self.window {
                        let scale = window.scale_factor();
                        let logical_w = size.width as f64 / scale;
                        let logical_h = size.height as f64 / scale;
                        self.update_layout_for_size(logical_w, logical_h);
                        let max_scroll = self.max_scroll_y(logical_h);
                        self.scroll_y = self.scroll_y.clamp(0.0, max_scroll);
                    }
                    // Immediately render frame synchronously on resize!
                    // This prevents macOS CAMetalLayer from stretching the previous frame's texture.
                    self.render_frame();
                }
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                if let Some(window) = &self.window {
                    configure_metal_layer(window);
                    let size = window.inner_size();
                    if size.width > 0 && size.height > 0 {
                        if let Some(surface) = &mut self.surface {
                            self.render_cx
                                .resize_surface(surface, size.width, size.height);
                        }
                        let scale = window.scale_factor();
                        let logical_w = size.width as f64 / scale;
                        let logical_h = size.height as f64 / scale;
                        self.update_layout_for_size(logical_w, logical_h);
                        let max_scroll = self.max_scroll_y(logical_h);
                        self.scroll_y = self.scroll_y.clamp(0.0, max_scroll);
                        self.render_frame();
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                self.render_frame();
            }
            WindowEvent::CursorMoved { position, .. } => {
                let scale = self.window.as_ref().map(|w| w.scale_factor()).unwrap_or(1.0);
                let point = Point::new(position.x / scale, position.y / scale);
                self.cursor_pos = Some(point);

                if self.is_dragging_scrollbar {
                    let (_win_w, win_h) = if let Some(w) = &self.window {
                        let size = w.inner_size();
                        (size.width as f64 / scale, size.height as f64 / scale)
                    } else {
                        (self.config.width as f64, self.config.height as f64)
                    };
                    let content_h = self.content_height();
                    if content_h > win_h {
                        let track_h = win_h;
                        let thumb_h = ((win_h / content_h) * track_h).max(24.0).min(track_h);
                        let available_track = (track_h - thumb_h).max(1.0);
                        let max_scroll = (content_h - win_h).max(0.0);
                        let dy = point.y - self.scrollbar_drag_start_y;
                        let scroll_delta = (dy / available_track) * max_scroll;
                        self.scroll_y = (self.scrollbar_start_scroll_y + scroll_delta).clamp(0.0, max_scroll);
                        if let Some(w) = &self.window {
                            w.request_redraw();
                        }
                    }
                    return;
                }

                if self.inspect_mode {
                    let (win_w, win_h) = if let Some(w) = &self.window {
                        let size = w.inner_size();
                        (size.width as f64 / scale, size.height as f64 / scale)
                    } else {
                        (self.config.width as f64, self.config.height as f64)
                    };
                    let panel_x = win_w - self.panel_component.width;

                    if point.x >= panel_x {
                        let btn_hovered = self.panel_component.is_cursor_btn_hovered(point.x, point.y, panel_x);
                        let mut needs_redraw = false;

                        if self.inspector_state.inspect_cursor_hovered != btn_hovered {
                            self.inspector_state.inspect_cursor_hovered = btn_hovered;
                            needs_redraw = true;
                        }

                        let tree_items = crate::inspector::build_tree_items_from_layout(
                            &self.layout,
                            &self.inspector_state,
                        );
                        let panel_hovered = self.panel_component.handle_mouse_move(
                            point.x,
                            point.y,
                            panel_x,
                            win_h,
                            &tree_items,
                            &self.inspector_state,
                        );
                        if self.inspector_state.hovered_id != panel_hovered {
                            self.inspector_state.set_hovered_id(panel_hovered);
                            needs_redraw = true;
                        }

                        if needs_redraw {
                            if let Some(w) = &self.window {
                                w.request_redraw();
                            }
                        }

                        // Clear canvas hover state when moving into panel
                        if let Some(old_id) = self.hovered_node.take() {
                            let doc_point = Point::new(point.x + self.scroll_x, point.y + self.scroll_y);
                            let mut leave_event = Event::new(
                                EventKind::PointerLeave,
                                doc_point,
                                Point::new(0.0, 0.0),
                                self.modifiers,
                                old_id,
                            );
                            if let Some(handler) = &mut self.event_handler {
                                handler(&mut leave_event, &self.layout);
                            }
                        }
                        return;
                    } else if self.inspector_state.inspect_cursor_hovered {
                        self.inspector_state.inspect_cursor_hovered = false;
                        if let Some(w) = &self.window {
                            w.request_redraw();
                        }
                    }
                }

                // Cursor is over the canvas
                let doc_point = Point::new(point.x + self.scroll_x, point.y + self.scroll_y);
                let hit = self.layout.hit_test(doc_point);
                let new_hovered = hit.as_ref().map(|h| h.target);

                if new_hovered != self.hovered_node {
                    if let Some(old_id) = self.hovered_node {
                        let mut leave_event = Event::new(
                            EventKind::PointerLeave,
                            doc_point,
                            Point::new(0.0, 0.0),
                            self.modifiers,
                            old_id,
                        );
                        if let Some(handler) = &mut self.event_handler {
                            handler(&mut leave_event, &self.layout);
                        }
                    }

                    if let Some(ref hit_res) = hit {
                        let enter_event = Event::new(
                            EventKind::PointerEnter,
                            doc_point,
                            hit_res.local_point,
                            self.modifiers,
                            hit_res.target,
                        );
                        self.dispatch_event_with_bubble(enter_event, &hit_res.bubble_path);
                    }

                    self.hovered_node = new_hovered;
                }

                if let Some(ref hit_res) = hit {
                    let move_event = Event::new(
                        EventKind::PointerMove,
                        doc_point,
                        hit_res.local_point,
                        self.modifiers,
                        hit_res.target,
                    );
                    self.dispatch_event_with_bubble(move_event, &hit_res.bubble_path);
                }

                // Update cursor icon on hover (e.g. CursorIcon::Pointer when hovering over links or pointer elements)
                if let Some(w) = &self.window {
                    let is_pointer = hit.as_ref().map_or(false, |h| {
                        h.bubble_path.iter().any(|&nid| {
                            self.layout.get_node(nid).map_or(false, |node| {
                                node.properties.contains_key("url")
                                    || node.properties.get("cursor").and_then(|v| v.as_str()) == Some("Pointer")
                                    || node.properties.get("cursor").and_then(|v| v.as_str()) == Some("pointer")
                            })
                        })
                    });

                    if is_pointer {
                        w.set_cursor(winit::window::CursorIcon::Pointer);
                    } else {
                        w.set_cursor(winit::window::CursorIcon::Default);
                    }
                }

                if self.inspect_mode {
                    let target_hover = if self.inspector_state.inspect_cursor_active {
                        new_hovered
                    } else {
                        None
                    };
                    if self.inspector_state.hovered_id != target_hover {
                        self.inspector_state.set_hovered_id(target_hover);
                        if let Some(w) = &self.window {
                            w.request_redraw();
                        }
                    }
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let btn = match button {
                    winit::event::MouseButton::Left => MouseButton::Left,
                    winit::event::MouseButton::Right => MouseButton::Right,
                    winit::event::MouseButton::Middle => MouseButton::Middle,
                    winit::event::MouseButton::Back => MouseButton::Other(1),
                    winit::event::MouseButton::Forward => MouseButton::Other(2),
                    winit::event::MouseButton::Other(c) => MouseButton::Other(c),
                };

                if let Some(point) = self.cursor_pos {
                    let scale = self.window.as_ref().map(|w| w.scale_factor()).unwrap_or(1.0);
                    let (win_w, win_h) = if let Some(w) = &self.window {
                        let size = w.inner_size();
                        (size.width as f64 / scale, size.height as f64 / scale)
                    } else {
                        (self.config.width as f64, self.config.height as f64)
                    };

                    if self.inspect_mode {
                        let panel_x = win_w - self.panel_component.width;
                        if point.x >= panel_x {
                            if state == ElementState::Pressed {
                                let tree_items = crate::inspector::build_tree_items_from_layout(
                                    &self.layout,
                                    &self.inspector_state,
                                );
                                let action = self.panel_component.handle_click(
                                    point.x,
                                    point.y,
                                    panel_x,
                                    win_h,
                                    &tree_items,
                                    &self.inspector_state,
                                    Some(&self.layout),
                                );

                                match action {
                                    crate::inspector::PanelHitResult::ToggleExpand(node_id) => {
                                        self.inspector_state.toggle_expanded_id(node_id);
                                        if let Some(w) = &self.window {
                                            w.request_redraw();
                                        }
                                    }
                                    crate::inspector::PanelHitResult::TogglePropertyRef(node_id, prop_key) => {
                                        self.inspector_state.toggle_property_ref_expanded(node_id, &prop_key);
                                        if let Some(w) = &self.window {
                                            w.request_redraw();
                                        }
                                    }
                                    crate::inspector::PanelHitResult::SelectNode(node_id) => {
                                        self.selected_node = Some(node_id);
                                        self.inspector_state.set_selected_id(Some(node_id));
                                        self.inspector_state.expand_ancestors(node_id, &self.layout);
                                        if let Some(w) = &self.window {
                                            w.request_redraw();
                                        }
                                    }
                                    crate::inspector::PanelHitResult::ToggleInspectCursor => {
                                        let active = self.inspector_state.toggle_inspect_cursor();
                                        if !active {
                                            self.inspector_state.set_hovered_id(None);
                                        }
                                        println!(
                                            "[Inspector] Pick cursor: {}",
                                            if active {
                                                "ACTIVE (click any component on canvas to inspect)"
                                            } else {
                                                "INACTIVE (normal page interaction)"
                                            }
                                        );
                                        if let Some(w) = &self.window {
                                            w.request_redraw();
                                        }
                                    }
                                    crate::inspector::PanelHitResult::None => {}
                                }
                            }
                            // Always consume mouse events over the inspector panel
                            return;
                        }
                    }

                    let content_h = self.content_height();
                    let canvas_w = if self.inspect_mode {
                        (win_w - self.panel_component.width).max(100.0)
                    } else {
                        win_w
                    };

                    if state == ElementState::Pressed {
                        if content_h > win_h && point.x >= canvas_w - 14.0 && point.x <= canvas_w {
                            self.is_dragging_scrollbar = true;
                            self.scrollbar_drag_start_y = point.y;
                            self.scrollbar_start_scroll_y = self.scroll_y;
                            return;
                        }
                    } else if self.is_dragging_scrollbar {
                        self.is_dragging_scrollbar = false;
                        return;
                    }

                    // Mouse input is over canvas area
                    let doc_point = Point::new(point.x + self.scroll_x, point.y + self.scroll_y);
                    let hit = self.layout.hit_test(doc_point);
                    match state {
                        ElementState::Pressed => {
                            if self.inspect_mode && self.inspector_state.inspect_cursor_active {
                                // Inspect picker tool is ACTIVE: intercept click to select component!
                                if let Some(ref hit_res) = hit {
                                    self.selected_node = Some(hit_res.target);
                                    self.inspector_state.set_selected_id(Some(hit_res.target));
                                    self.inspector_state.expand_ancestors(hit_res.target, &self.layout);

                                    // Turn off inspect cursor after picking (standard DevTools behavior)
                                    self.inspector_state.inspect_cursor_active = false;
                                    self.inspector_state.set_hovered_id(None);

                                    if let Some(node) = self.layout.get_node(hit_res.target) {
                                        println!(
                                            "[Inspector] Selected element: {} (id: {:?}) bounds: [x: {:.1}, y: {:.1}, w: {:.1}, h: {:.1}] z: {}",
                                            node.name,
                                            node.id,
                                            node.rect.x,
                                            node.rect.y,
                                            node.rect.width,
                                            node.rect.height,
                                            node.z
                                        );
                                        if let Some(text) = &node.text_content {
                                            println!("    Text: {:?}", text.trim());
                                        } else if node.name == "Text" {
                                            println!("    Text: (empty)");
                                        }

                                        let path: Vec<String> = hit_res
                                            .bubble_path
                                            .iter()
                                            .filter_map(|id| {
                                                self.layout.get_node(*id).map(|n| format!("{} ({:?})", n.name, n.id))
                                            })
                                            .collect();
                                        println!("    Hierarchy: {}", path.join(" -> "));
                                    }
                                } else {
                                    // Clicked empty canvas in picking mode
                                    self.selected_node = None;
                                    self.inspector_state.set_selected_id(None);
                                    self.inspector_state.inspect_cursor_active = false;
                                    self.inspector_state.set_hovered_id(None);
                                }
                                if let Some(w) = &self.window {
                                    w.request_redraw();
                                }
                                return; // Intercepted: DO NOT dispatch to page elements
                            }

                            // Normal page interaction: inspect cursor is inactive
                            if let Some(ref hit_res) = hit {
                                self.pressed_node = Some((hit_res.target, btn));
                                let down_event = Event::new(
                                    EventKind::PointerDown { button: btn },
                                    doc_point,
                                    hit_res.local_point,
                                    self.modifiers,
                                    hit_res.target,
                                );
                                self.dispatch_event_with_bubble(down_event, &hit_res.bubble_path);
                            } else {
                                self.pressed_node = None;
                            }
                        }
                        ElementState::Released => {
                            if let Some((pressed_id, pressed_btn)) = self.pressed_node.take() {
                                if let Some(ref hit_res) = hit {
                                    let up_event = Event::new(
                                        EventKind::PointerUp { button: btn },
                                        doc_point,
                                        hit_res.local_point,
                                        self.modifiers,
                                        hit_res.target,
                                    );
                                    self.dispatch_event_with_bubble(up_event, &hit_res.bubble_path);

                                    if pressed_btn == btn && hit_res.bubble_path.contains(&pressed_id) {
                                        let click_event = Event::new(
                                            EventKind::Click { button: btn },
                                            doc_point,
                                            hit_res.local_point,
                                            self.modifiers,
                                            hit_res.target,
                                        );
                                        self.dispatch_event_with_bubble(click_event, &hit_res.bubble_path);

                                        if btn == MouseButton::Left {
                                            for &nid in &hit_res.bubble_path {
                                                let maybe_url = self.layout.get_node(nid).and_then(|node| {
                                                    node.properties.get("url").and_then(|v| v.as_str()).map(|s| s.to_string())
                                                });
                                                if let Some(url) = maybe_url {
                                                    if url.starts_with('#') {
                                                        if !self.scroll_to_anchor(Some(nid), &url) {
                                                            eprintln!("[Viewer] In-page anchor not found: {}", url);
                                                        }
                                                        break;
                                                    } else if self.scroll_to_anchor(Some(nid), &url) {
                                                        break;
                                                    } else {
                                                        println!("[Viewer] Opening link: {}", url);
                                                        #[cfg(target_os = "macos")]
                                                        let _ = std::process::Command::new("open").arg(&url).spawn();
                                                        #[cfg(target_os = "linux")]
                                                        let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
                                                        #[cfg(target_os = "windows")]
                                                        let _ = std::process::Command::new("cmd").args(["/C", "start", "", &url]).spawn();
                                                        break;
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let (delta_x, delta_y) = match delta {
                    winit::event::MouseScrollDelta::LineDelta(x, y) => (x as f64 * 20.0, y as f64 * 20.0),
                    winit::event::MouseScrollDelta::PixelDelta(pos) => (pos.x, pos.y),
                };
                if let Some(point) = self.cursor_pos {
                    let scale = self.window.as_ref().map(|w| w.scale_factor()).unwrap_or(1.0);
                    let (win_w, win_h) = if let Some(w) = &self.window {
                        let size = w.inner_size();
                        (size.width as f64 / scale, size.height as f64 / scale)
                    } else {
                        (self.config.width as f64, self.config.height as f64)
                    };
                    let panel_x = win_w - self.panel_component.width;

                    if self.inspect_mode && point.x >= panel_x {
                        let divider_y = self.panel_component.divider_y(win_h);
                        if point.y < divider_y {
                            let tree_items = crate::inspector::build_tree_items_from_layout(
                                &self.layout,
                                &self.inspector_state,
                            );
                            let max_scroll = self.panel_component.max_scroll(tree_items.len(), win_h);
                            self.inspector_state.scroll_by(-delta_y, max_scroll);
                        } else {
                            let max_detail_scroll = self.panel_component.max_detail_scroll_with_state(
                                self.selected_node,
                                &self.layout,
                                Some(&self.inspector_state),
                                win_h,
                            );
                            self.inspector_state.scroll_detail_by(-delta_y, max_detail_scroll);
                        }
                        if let Some(w) = &self.window {
                            w.request_redraw();
                        }
                        return;
                    }

                    // Window document scrolling:
                    let max_scroll = self.max_scroll_y(win_h);
                    let old_scroll = self.scroll_y;
                    self.scroll_y = (self.scroll_y - delta_y).clamp(0.0, max_scroll);
                    if (self.scroll_y - old_scroll).abs() > 0.001 {
                        if let Some(w) = &self.window {
                            w.request_redraw();
                        }
                    }

                    let doc_point = Point::new(point.x + self.scroll_x, point.y + self.scroll_y);
                    if let Some(ref hit_res) = self.layout.hit_test(doc_point) {
                        let scroll_event = Event::new(
                            EventKind::Scroll { delta_x, delta_y },
                            doc_point,
                            hit_res.local_point,
                            self.modifiers,
                            hit_res.target,
                        );
                        self.dispatch_event_with_bubble(scroll_event, &hit_res.bubble_path);
                    }
                }
            }
            WindowEvent::CursorLeft { .. } => {
                self.is_dragging_scrollbar = false;
                if let Some(old_id) = self.hovered_node.take() {
                    let pt = self.cursor_pos.unwrap_or_default();
                    let doc_pt = Point::new(pt.x + self.scroll_x, pt.y + self.scroll_y);
                    let mut leave_event = Event::new(
                        EventKind::PointerLeave,
                        doc_pt,
                        Point::new(0.0, 0.0),
                        self.modifiers,
                        old_id,
                    );
                    if let Some(handler) = &mut self.event_handler {
                        handler(&mut leave_event, &self.layout);
                    }
                }
                self.inspector_state.set_hovered_id(None);
                self.inspector_state.inspect_cursor_hovered = false;
                if self.inspect_mode {
                    if let Some(w) = &self.window {
                        w.request_redraw();
                    }
                }
                self.cursor_pos = None;
            }
            WindowEvent::ModifiersChanged(new_mods) => {
                self.modifiers.shift = new_mods.state().shift_key();
                self.modifiers.ctrl = new_mods.state().control_key();
                self.modifiers.alt = new_mods.state().alt_key();
                self.modifiers.meta = new_mods.state().super_key();
            }
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        logical_key,
                        state: ElementState::Pressed,
                        ..
                    },
                ..
            } => match logical_key {
                Key::Character(c) if c.eq_ignore_ascii_case("q") => {
                    event_loop.exit();
                }
                Key::Named(NamedKey::Escape) => {
                    if self.inspect_mode && self.inspector_state.inspect_cursor_active {
                        self.inspector_state.inspect_cursor_active = false;
                        self.inspector_state.set_hovered_id(None);
                        println!("[Inspector] Pick cursor deactivated (Escape)");
                        if let Some(w) = &self.window {
                            w.request_redraw();
                        }
                    } else {
                        event_loop.exit();
                    }
                }
                Key::Character(c) if c.eq_ignore_ascii_case("d") => {
                    self.layout.print_dom();
                }
                Key::Character(c) if c.eq_ignore_ascii_case("i") => {
                    self.inspect_mode = !self.inspect_mode;
                    if !self.inspect_mode {
                        self.hovered_node = None;
                        self.inspector_state.set_hovered_id(None);
                        self.inspector_state.inspect_cursor_active = false;
                    }
                    println!(
                        "[Inspector] Mode: {}",
                        if self.inspect_mode { "ON (Docked DOM inspector active)" } else { "OFF" }
                    );
                    if let Some(w) = &self.window {
                        w.request_redraw();
                    }
                }
                Key::Character(c) if c.eq_ignore_ascii_case("c") => {
                    if self.inspect_mode {
                        let active = self.inspector_state.toggle_inspect_cursor();
                        if !active {
                            self.inspector_state.set_hovered_id(None);
                        }
                        println!(
                            "[Inspector] Pick cursor: {}",
                            if active {
                                "ACTIVE (click any component on canvas to inspect)"
                            } else {
                                "INACTIVE (normal page interaction)"
                            }
                        );
                        if let Some(w) = &self.window {
                            w.request_redraw();
                        }
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }
}

/// Launches an interactive window viewer displaying the given `ResolvedLayout`.
pub fn run_viewer(layout: ResolvedLayout, config: ViewerConfig) -> Result<(), Box<dyn std::error::Error>> {
    let mut app = ViewerApp::new(layout, config);
    run_viewer_app(&mut app, None)
}

/// Launches an interactive window viewer displaying a live `Document`, re-evaluating the layout DAG on resize.
pub fn run_viewer_with_document(
    doc: Document,
    config: ViewerConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let initial_layout = evaluate_document_with_window(&doc, config.width as f64, config.height as f64)?;
    let mut app = ViewerApp::new(initial_layout, config).with_document(doc);
    run_viewer_app(&mut app, None)
}

/// Launches an interactive window viewer watching a source file on disk, hot-reloading on changes,
/// and wiring a `ComponentRegistry` for companion component lifecycle and event dispatching.
pub fn run_viewer_with_file_and_registry(
    file_path: PathBuf,
    config: ViewerConfig,
    registry: ComponentRegistry,
) -> Result<(), Box<dyn std::error::Error>> {
    let source = std::fs::read_to_string(&file_path)
        .map_err(|e| format!("Failed to read file '{}': {e}", file_path.display()))?;
    let doc = parse_document(&source)
        .map_err(|e| format!("Parse error in '{}': {e}", file_path.display()))?;
    let base_dir = file_path.parent().unwrap_or_else(|| std::path::Path::new("."));
    let resolver = FsResolver;
    let compiled = CompiledDocument::compile_with_registry(
        &doc,
        config.width as f64,
        config.height as f64,
        base_dir,
        &resolver,
        &registry,
    )?;
    let initial_layout = compiled.layout().clone();
    let mut app = ViewerApp::new(initial_layout, config)
        .with_document(doc)
        .with_watch_path(file_path.clone())
        .with_compiled(compiled)
        .with_registry(registry);
    run_viewer_app(&mut app, Some(&file_path))
}

/// Launches an interactive window viewer watching a source file on disk, hot-reloading on changes.
pub fn run_viewer_with_file(
    file_path: PathBuf,
    config: ViewerConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    run_viewer_with_file_and_registry(file_path, config, ComponentRegistry::standard())
}

fn run_viewer_app(
    app: &mut ViewerApp,
    watch_file: Option<&PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    let event_loop: EventLoop<ViewerUserEvent> = EventLoop::with_user_event().build()?;
    event_loop.set_control_flow(ControlFlow::Wait);

    if let Some(path) = watch_file {
        let proxy = event_loop.create_proxy();
        let target_filename = path.file_name().map(|n| n.to_os_string());
        let parent_dir = path
            .parent()
            .and_then(|p| if p.as_os_str().is_empty() { None } else { Some(p) })
            .unwrap_or_else(|| std::path::Path::new("."));

        let (tx, rx) = std::sync::mpsc::channel::<()>();

        // Debounce worker thread: coalesces rapid bursts of filesystem notifications
        // (atomic editor swapfiles, temp file renames, metadata updates) into a single reload event.
        std::thread::spawn(move || {
            while rx.recv().is_ok() {
                std::thread::sleep(std::time::Duration::from_millis(40));
                while rx.try_recv().is_ok() {}
                let _ = proxy.send_event(ViewerUserEvent::FileModified);
            }
        });

        let mut watcher = RecommendedWatcher::new(
            move |res: Result<NotifyEvent, notify::Error>| {
                if let Ok(event) = res {
                    if !event.kind.is_access() {
                        let matches = if let Some(target) = &target_filename {
                            event.paths.is_empty()
                                || event.paths.iter().any(|p| p.file_name() == Some(target.as_os_str()))
                        } else {
                            true
                        };
                        if matches {
                            let _ = tx.send(());
                        }
                    }
                }
            },
            notify::Config::default(),
        )?;

        watcher.watch(parent_dir, RecursiveMode::NonRecursive)?;
        app._watcher = Some(watcher);
        println!("[HotReload] Watching for live changes in: {}", path.display());
    }

    event_loop.run_app(app)?;
    Ok(())
}
