use std::num::NonZeroUsize;
use std::sync::Arc;

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

use crate::compiler::layout::ResolvedLayout;
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

/// Interactive window viewer application using Winit and Vello.
pub struct ViewerApp {
    config: ViewerConfig,
    layout: ResolvedLayout,
    render_cx: RenderContext,
    surface: Option<RenderSurface<'static>>,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    font_cx: FontContext,
    layout_cx: LayoutContext<()>,
    frame_count: u64,
}

impl ViewerApp {
    pub fn new(layout: ResolvedLayout, config: ViewerConfig) -> Self {
        Self {
            config,
            layout,
            render_cx: RenderContext::new(),
            surface: None,
            window: None,
            renderer: None,
            font_cx: FontContext::new(),
            layout_cx: LayoutContext::new(),
            frame_count: 0,
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

        let scene = build_scene(
            &self.layout,
            &mut self.font_cx,
            &mut self.layout_cx,
            &scene_opts,
        );

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
        self.frame_count += 1;
    }
}

impl ApplicationHandler for ViewerApp {
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
                        self.render_frame();
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                self.render_frame();
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
                _ => {}
            },
            _ => {}
        }
    }
}

/// Launches an interactive window viewer displaying the given `ResolvedLayout`.
pub fn run_viewer(layout: ResolvedLayout, config: ViewerConfig) -> Result<(), Box<dyn std::error::Error>> {
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = ViewerApp::new(layout, config);
    event_loop.run_app(&mut app)?;
    Ok(())
}
