use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::Arc;

use notify::{Event as NotifyEvent, RecommendedWatcher, RecursiveMode, Watcher};
use parley::{FontContext, LayoutContext};
use vello::peniko::Color;
use vello::util::{RenderContext, RenderSurface};
use vello::wgpu;
use vello::{AaConfig, AaSupport, RenderParams, Renderer, RendererOptions};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

use crate::ast::Document;
use crate::compiler::evaluate_document_with_window;
use crate::compiler::expanded::NodeId;
use crate::compiler::layout::ResolvedLayout;
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

    fn dispatch_event_with_bubble(&mut self, mut event: Event, bubble_path: &[NodeId]) {
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

        let mut scene = build_scene(
            &self.layout,
            &mut self.font_cx,
            &mut self.layout_cx,
            &scene_opts,
        );

        if self.inspect_mode {
            use vello::kurbo::{Affine, Rect as KRect, Stroke};
            use vello::peniko::{Brush, Color as PColor};

            let transform = Affine::scale(window.scale_factor());

            // 1. Highlight hovered node (cyan outline)
            if let Some(hovered_id) = self.hovered_node {
                if let Some(node) = self.layout.get_node(hovered_id) {
                    let rect = KRect::new(
                        node.rect.x,
                        node.rect.y,
                        node.rect.x + node.rect.width,
                        node.rect.y + node.rect.height,
                    );
                    scene.stroke(
                        &Stroke::new(2.0),
                        transform,
                        Brush::Solid(PColor::from_rgba8(0, 200, 255, 220)),
                        None,
                        &rect,
                    );
                }
            }

            // 2. Highlight selected node (amber outline)
            if let Some(selected_id) = self.selected_node {
                if let Some(node) = self.layout.get_node(selected_id) {
                    let rect = KRect::new(
                        node.rect.x,
                        node.rect.y,
                        node.rect.x + node.rect.width,
                        node.rect.y + node.rect.height,
                    );
                    scene.stroke(
                        &Stroke::new(3.0),
                        transform,
                        Brush::Solid(PColor::from_rgba8(255, 170, 0, 255)),
                        None,
                        &rect,
                    );
                }
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
        if let Some(doc) = &self.doc {
            if let Ok(new_layout) = evaluate_document_with_window(doc, logical_w, logical_h) {
                self.layout = new_layout;
            }
        }

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
                    if let (Some(doc), Some(window)) = (&self.doc, &self.window) {
                        let scale = window.scale_factor();
                        let logical_w = size.width as f64 / scale;
                        let logical_h = size.height as f64 / scale;
                        if let Ok(new_layout) = evaluate_document_with_window(doc, logical_w, logical_h) {
                            self.layout = new_layout;
                        }
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
                        if let Some(doc) = &self.doc {
                            let scale = window.scale_factor();
                            let logical_w = size.width as f64 / scale;
                            let logical_h = size.height as f64 / scale;
                            if let Ok(new_layout) = evaluate_document_with_window(doc, logical_w, logical_h) {
                                self.layout = new_layout;
                            }
                        }
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

                let hit = self.layout.hit_test(point);
                let new_hovered = hit.as_ref().map(|h| h.target);

                if new_hovered != self.hovered_node {
                    if let Some(old_id) = self.hovered_node {
                        let mut leave_event = Event::new(
                            EventKind::PointerLeave,
                            point,
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
                            point,
                            hit_res.local_point,
                            self.modifiers,
                            hit_res.target,
                        );
                        self.dispatch_event_with_bubble(enter_event, &hit_res.bubble_path);
                    }

                    self.hovered_node = new_hovered;

                    if self.inspect_mode {
                        if let Some(w) = &self.window {
                            w.request_redraw();
                        }
                    }
                }

                if let Some(ref hit_res) = hit {
                    let move_event = Event::new(
                        EventKind::PointerMove,
                        point,
                        hit_res.local_point,
                        self.modifiers,
                        hit_res.target,
                    );
                    self.dispatch_event_with_bubble(move_event, &hit_res.bubble_path);
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
                    let hit = self.layout.hit_test(point);
                    match state {
                        ElementState::Pressed => {
                            if let Some(ref hit_res) = hit {
                                self.pressed_node = Some((hit_res.target, btn));
                                if self.inspect_mode {
                                    self.selected_node = Some(hit_res.target);
                                    if let Some(node) = self.layout.get_node(hit_res.target) {
                                        println!(
                                            "[Inspector] Click at ({:.1}, {:.1}) (local: ({:.1}, {:.1}))",
                                            point.x, point.y, hit_res.local_point.x, hit_res.local_point.y
                                        );
                                        println!(
                                            "    Target: {} (id: {:?}) bounds: [x: {:.1}, y: {:.1}, w: {:.1}, h: {:.1}] z: {}",
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

                                        // If the target itself doesn't have text, report any context text from children or siblings
                                        if node.text_content.is_none() {
                                            let mut context_texts = Vec::new();
                                            for child_id in &node.children {
                                                if let Some(child) = self.layout.get_node(*child_id) {
                                                    if let Some(child_text) = &child.text_content {
                                                        context_texts.push(format!("child {}: {:?}", child.name, child_text.trim()));
                                                    }
                                                }
                                            }
                                            if let Some(parent_id) = node.parent {
                                                if let Some(parent_node) = self.layout.get_node(parent_id) {
                                                    for sibling_id in &parent_node.children {
                                                        if *sibling_id != node.id {
                                                            if let Some(sibling) = self.layout.get_node(*sibling_id) {
                                                                if let Some(stext) = &sibling.text_content {
                                                                    context_texts.push(format!("sibling {}: {:?}", sibling.name, stext.trim()));
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                            for info in context_texts {
                                                println!("    Context {}", info);
                                            }
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
                                    if let Some(w) = &self.window {
                                        w.request_redraw();
                                    }
                                }
                                let down_event = Event::new(
                                    EventKind::PointerDown { button: btn },
                                    point,
                                    hit_res.local_point,
                                    self.modifiers,
                                    hit_res.target,
                                );
                                self.dispatch_event_with_bubble(down_event, &hit_res.bubble_path);
                            } else {
                                self.pressed_node = None;
                                if self.inspect_mode {
                                    self.selected_node = None;
                                    println!(
                                        "[Inspector] Click at ({:.1}, {:.1}): no node hit",
                                        point.x, point.y
                                    );
                                    if let Some(w) = &self.window {
                                        w.request_redraw();
                                    }
                                }
                            }
                        }
                        ElementState::Released => {
                            if let Some(ref hit_res) = hit {
                                let up_event = Event::new(
                                    EventKind::PointerUp { button: btn },
                                    point,
                                    hit_res.local_point,
                                    self.modifiers,
                                    hit_res.target,
                                );
                                self.dispatch_event_with_bubble(up_event, &hit_res.bubble_path);

                                if let Some((pressed_id, pressed_btn)) = self.pressed_node {
                                    if pressed_btn == btn && hit_res.bubble_path.contains(&pressed_id) {
                                        let click_event = Event::new(
                                            EventKind::Click { button: btn },
                                            point,
                                            hit_res.local_point,
                                            self.modifiers,
                                            hit_res.target,
                                        );
                                        self.dispatch_event_with_bubble(click_event, &hit_res.bubble_path);
                                    }
                                }
                            }
                            self.pressed_node = None;
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
                    if let Some(ref hit_res) = self.layout.hit_test(point) {
                        let scroll_event = Event::new(
                            EventKind::Scroll { delta_x, delta_y },
                            point,
                            hit_res.local_point,
                            self.modifiers,
                            hit_res.target,
                        );
                        self.dispatch_event_with_bubble(scroll_event, &hit_res.bubble_path);
                    }
                }
            }
            WindowEvent::CursorLeft { .. } => {
                if let Some(old_id) = self.hovered_node.take() {
                    let pt = self.cursor_pos.unwrap_or_default();
                    let mut leave_event = Event::new(
                        EventKind::PointerLeave,
                        pt,
                        Point::new(0.0, 0.0),
                        self.modifiers,
                        old_id,
                    );
                    if let Some(handler) = &mut self.event_handler {
                        handler(&mut leave_event, &self.layout);
                    }
                    if self.inspect_mode {
                        if let Some(w) = &self.window {
                            w.request_redraw();
                        }
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
                    event_loop.exit();
                }
                Key::Character(c) if c.eq_ignore_ascii_case("d") => {
                    self.layout.print_dom();
                }
                Key::Character(c) if c.eq_ignore_ascii_case("i") => {
                    self.inspect_mode = !self.inspect_mode;
                    println!(
                        "[Inspector] Mode: {}",
                        if self.inspect_mode { "ON (Hover/click elements to inspect)" } else { "OFF" }
                    );
                    if let Some(w) = &self.window {
                        w.request_redraw();
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

/// Launches an interactive window viewer watching a source file on disk, hot-reloading on changes.
pub fn run_viewer_with_file(
    file_path: PathBuf,
    config: ViewerConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let source = std::fs::read_to_string(&file_path)
        .map_err(|e| format!("Failed to read file '{}': {e}", file_path.display()))?;
    let doc = parse_document(&source)
        .map_err(|e| format!("Parse error in '{}': {e}", file_path.display()))?;
    let initial_layout = evaluate_document_with_window(&doc, config.width as f64, config.height as f64)?;
    let mut app = ViewerApp::new(initial_layout, config)
        .with_document(doc)
        .with_watch_path(file_path.clone());
    run_viewer_app(&mut app, Some(&file_path))
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
