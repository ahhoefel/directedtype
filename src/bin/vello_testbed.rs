use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Instant;

use parley::layout::PositionedLayoutItem;
use parley::style::{FontWeight, StyleProperty};
use parley::{Alignment, FontContext, LayoutContext};
use vello::kurbo::{Affine, Line, RoundedRect, Stroke};
use vello::peniko::{Brush, Color, Fill};
use vello::util::{RenderContext, RenderSurface};
use vello::wgpu;
use vello::{AaConfig, AaSupport, RenderParams, Renderer, RendererOptions, Scene};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

#[cfg(target_os = "macos")]
fn configure_metal_layer(window: &Window, gravity_top_left: bool, presents_with_tx: bool) {
    use objc2::runtime::AnyObject;
    use objc2::msg_send;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    if let Ok(handle) = window.window_handle() {
        if let RawWindowHandle::AppKit(appkit_handle) = handle.as_raw() {
            unsafe {
                let view = appkit_handle.ns_view.as_ptr() as *mut AnyObject;

                // 1. Configure NSView:
                // NSViewLayerContentsRedrawDuringViewResize = 2
                // NSViewLayerContentsPlacementTopLeft = 11, NSViewLayerContentsPlacementScaleAxesIndependently = 0
                let redraw_policy = 2isize;
                let placement = if gravity_top_left { 11isize } else { 0isize };
                let _: () = msg_send![view, setLayerContentsRedrawPolicy: redraw_policy];
                let _: () = msg_send![view, setLayerContentsPlacement: placement];

                let root_layer: *mut AnyObject = msg_send![view, layer];
                if !root_layer.is_null() {
                    let scale = window.scale_factor();
                    apply_layer_config(root_layer, scale, gravity_top_left, presents_with_tx);
                }
            }
        }
    }
}

#[cfg(target_os = "macos")]
unsafe fn apply_layer_config(
    layer: *mut objc2::runtime::AnyObject,
    scale: f64,
    gravity_top_left: bool,
    presents_with_tx: bool,
) {
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send};
    use std::ffi::CStr;

    let s: *const CStr = if gravity_top_left { c"topLeft" } else { c"resize" };
    let ns_gravity: *const AnyObject =
        msg_send![class!(NSString), stringWithUTF8String: s.cast::<std::ffi::c_char>()];

    let _: () = msg_send![layer, setContentsGravity: ns_gravity];
    let _: () = msg_send![layer, setContentsScale: scale];


    let pwt_sel = objc2::sel!(setPresentsWithTransaction:);
    if msg_send![layer, respondsToSelector: pwt_sel] {
        let _: () = msg_send![layer, setPresentsWithTransaction: presents_with_tx];
    }

    let sublayers: *mut AnyObject = msg_send![layer, sublayers];
    if !sublayers.is_null() {
        let count: usize = msg_send![sublayers, count];
        for i in 0..count {
            let sublayer: *mut AnyObject = msg_send![sublayers, objectAtIndex: i];
            apply_layer_config(sublayer, scale, gravity_top_left, presents_with_tx);
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn configure_metal_layer(_window: &Window, _gravity_top_left: bool, _presents_with_tx: bool) {}

/// Standalone minimal testbed isolating Winit + Vello rendering from any layout engine logic.
struct VelloTestbedApp {
    render_cx: RenderContext,
    surface: Option<RenderSurface<'static>>,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    font_cx: FontContext,
    layout_cx: LayoutContext<()>,

    // Testbed options & toggles
    apply_dpi_scaling: bool,
    sync_resize_render: bool,
    presents_with_tx: bool,
    contents_gravity_top_left: bool,
    present_mode: wgpu::PresentMode,
    frame_count: u64,
    start_time: Instant,
}

impl VelloTestbedApp {
    fn new() -> Self {
        Self {
            render_cx: RenderContext::new(),
            surface: None,
            window: None,
            renderer: None,
            font_cx: FontContext::new(),
            layout_cx: LayoutContext::new(),
            apply_dpi_scaling: true,
            sync_resize_render: true,
            presents_with_tx: true,
            contents_gravity_top_left: true,
            present_mode: wgpu::PresentMode::AutoNoVsync,
            frame_count: 0,
            start_time: Instant::now(),
        }
    }

    fn render_frame(&mut self) {
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

        let frame_start = Instant::now();

        // 1. Acquire current swapchain texture
        let surface_texture = match surface.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(st)
            | wgpu::CurrentSurfaceTexture::Suboptimal(st) => st,
            wgpu::CurrentSurfaceTexture::Occluded => {
                // Window is not visible yet (e.g. initial mapping on macOS) or minimized.
                // Request redraw so as soon as Cocoa marks it visible, the first frame presents.
                println!("[SURFACE] get_current_texture -> Occluded. Scheduled redraw.");
                window.request_redraw();
                return;
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                println!("[SURFACE] get_current_texture -> Outdated. Reconfiguring surface...");
                self.render_cx.configure_surface(surface);
                match surface.surface.get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Success(st)
                    | wgpu::CurrentSurfaceTexture::Suboptimal(st) => st,
                    other => {
                        println!("[SURFACE] Retry after Outdated returned: {:?}", other);
                        window.request_redraw();
                        return;
                    }
                }
            }
            wgpu::CurrentSurfaceTexture::Timeout => {
                println!("[SURFACE] get_current_texture -> Timeout (skipping transient frame)");
                window.request_redraw();
                return;
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                println!("[SURFACE] get_current_texture -> Lost. Reconfiguring surface...");
                self.render_cx.configure_surface(surface);
                window.request_redraw();
                return;
            }
            other => {
                println!("[SURFACE] get_current_texture returned unexpected status: {:?}", other);
                return;
            }
        };

        let scale_factor = window.scale_factor();
        let effective_scale = if self.apply_dpi_scaling {
            scale_factor
        } else {
            1.0
        };

        // 2. Build test scene
        let mut scene = Scene::new();

        // Background card
        let card = RoundedRect::new(30.0, 30.0, 780.0, 545.0, 16.0);
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            Brush::Solid(Color::from_rgba8(255, 255, 255, 255)),
            None,
            &card,
        );
        scene.stroke(
            &Stroke::new(2.0),
            Affine::IDENTITY,
            Brush::Solid(Color::from_rgba8(226, 232, 240, 255)),
            None,
            &card,
        );

        // Color chips / sample rectangles
        let chip1 = RoundedRect::new(60.0, 60.0, 180.0, 140.0, 8.0);
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            Brush::Solid(Color::from_rgba8(59, 130, 246, 255)),
            None,
            &chip1,
        );

        let chip2 = RoundedRect::new(200.0, 60.0, 320.0, 140.0, 8.0);
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            Brush::Solid(Color::from_rgba8(16, 185, 129, 255)),
            None,
            &chip2,
        );

        let chip3 = RoundedRect::new(340.0, 60.0, 460.0, 140.0, 8.0);
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            Brush::Solid(Color::from_rgba8(239, 68, 68, 255)),
            None,
            &chip3,
        );

        // Coordinate ruler line (width 600px from x=60 to x=660)
        let ruler = Line::new((60.0, 165.0), (660.0, 165.0));
        scene.stroke(
            &Stroke::new(3.0),
            Affine::IDENTITY,
            Brush::Solid(Color::from_rgba8(148, 163, 184, 255)),
            None,
            &ruler,
        );

        // Render helper texts
        let mut draw_text = |text: &str, x: f64, y: f64, size: f32, weight: f32, color: Color| {
            let mut builder = self.layout_cx.ranged_builder(&mut self.font_cx, text, 1.0, true);
            builder.push_default(StyleProperty::FontSize(size));
            if (weight - 400.0).abs() > 1.0 {
                builder.push_default(StyleProperty::FontWeight(FontWeight::new(weight)));
            }
            let mut layout = builder.build(text);
            layout.align(Alignment::Start, Default::default());

            for line in layout.lines() {
                for item in line.items() {
                    if let PositionedLayoutItem::GlyphRun(glyph_run) = item {
                        let run = glyph_run.run();
                        let font = run.font();
                        let glyphs = glyph_run.positioned_glyphs().map(|g| vello::Glyph {
                            id: g.id,
                            x: g.x,
                            y: g.y,
                        });
                        scene
                            .draw_glyphs(font)
                            .font_size(run.font_size())
                            .transform(Affine::translate((x, y)))
                            .brush(Brush::Solid(color))
                            .draw(Fill::NonZero, glyphs);
                    }
                }
            }
        };

        // Title
        draw_text(
            "Vello Standalone Isolation Testbed",
            60.0,
            200.0,
            26.0,
            700.0,
            Color::from_rgba8(15, 23, 42, 255),
        );

        // Live stats
        let stats_line_1 = format!(
            "Window Inner: {}x{} physical px | Scale Factor: {:.2}",
            width, height, scale_factor
        );
        draw_text(
            &stats_line_1,
            60.0,
            238.0,
            15.0,
            500.0,
            Color::from_rgba8(51, 65, 85, 255),
        );

        let stats_line_2 = format!(
            "[S] Retina DPI Scaling: {} (scale: {:.2})",
            if self.apply_dpi_scaling { "ACTIVE" } else { "DISABLED (1:1 px)" },
            effective_scale
        );
        draw_text(
            &stats_line_2,
            60.0,
            263.0,
            14.0,
            600.0,
            if self.apply_dpi_scaling {
                Color::from_rgba8(2, 132, 199, 255)
            } else {
                Color::from_rgba8(217, 119, 6, 255)
            },
        );

        let stats_line_3 = format!(
            "[M] Live Resize Presentation: {}",
            if self.sync_resize_render {
                "SYNCHRONOUS PRESENTATION"
            } else {
                "DEFERRED (request_redraw)"
            }
        );
        draw_text(
            &stats_line_3,
            60.0,
            288.0,
            14.0,
            600.0,
            if self.sync_resize_render {
                Color::from_rgba8(22, 163, 74, 255)
            } else {
                Color::from_rgba8(220, 38, 38, 255)
            },
        );

        let stats_line_4 = format!(
            "[T] presentsWithTransaction: {}",
            if self.presents_with_tx {
                "ENABLED (atomic CoreAnimation transaction)"
            } else {
                "DISABLED (uncoordinated GPU presentation)"
            }
        );
        draw_text(
            &stats_line_4,
            60.0,
            313.0,
            14.0,
            600.0,
            if self.presents_with_tx {
                Color::from_rgba8(22, 163, 74, 255)
            } else {
                Color::from_rgba8(220, 38, 38, 255)
            },
        );

        let stats_line_5 = format!(
            "[G] contentsGravity: {}",
            if self.contents_gravity_top_left {
                "TOP-LEFT (1:1 pixel pinning, no stretch)"
            } else {
                "RESIZE (compositor scales stale frames)"
            }
        );
        draw_text(
            &stats_line_5,
            60.0,
            338.0,
            14.0,
            600.0,
            if self.contents_gravity_top_left {
                Color::from_rgba8(22, 163, 74, 255)
            } else {
                Color::from_rgba8(220, 38, 38, 255)
            },
        );

        let stats_line_6 = format!(
            "[P] Swapchain PresentMode: {:?} ({})",
            self.present_mode,
            if matches!(self.present_mode, wgpu::PresentMode::AutoNoVsync | wgpu::PresentMode::Immediate) {
                "No Vsync lag / Immediate presentation"
            } else {
                "Vsync FIFO queue (may buffer stale frames)"
            }
        );
        draw_text(
            &stats_line_6,
            60.0,
            363.0,
            14.0,
            600.0,
            if matches!(self.present_mode, wgpu::PresentMode::AutoNoVsync | wgpu::PresentMode::Immediate) {
                Color::from_rgba8(22, 163, 74, 255)
            } else {
                Color::from_rgba8(217, 119, 6, 255)
            },
        );

        // Benchmark font sizes
        draw_text(
            "Size 28px Heading Sample",
            60.0,
            400.0,
            28.0,
            700.0,
            Color::from_rgba8(15, 23, 42, 255),
        );

        draw_text(
            "Directed acyclic graphs eliminate CSS layout thrashing and DOM reflow.",
            60.0,
            440.0,
            16.0,
            400.0,
            Color::from_rgba8(71, 85, 105, 255),
        );

        draw_text(
            "Keys: [T] Transaction Sync | [G] Gravity | [P] PresentMode | [M] Sync/Async | [S] DPI | [Q] Quit",
            60.0,
            475.0,
            13.0,
            500.0,
            Color::from_rgba8(100, 116, 139, 255),
        );

        let footer = format!(
            "Frame #{} | Uptime: {:.1}s",
            self.frame_count,
            self.start_time.elapsed().as_secs_f64()
        );
        draw_text(
            &footer,
            60.0,
            505.0,
            13.0,
            400.0,
            Color::from_rgba8(148, 163, 184, 255),
        );

        // 3. Apply uniform DPI scaling to the scene if enabled
        let final_scene = if (effective_scale - 1.0).abs() > 0.001 {
            let mut scaled = Scene::new();
            scaled.append(&scene, Some(Affine::scale(effective_scale)));
            scaled
        } else {
            scene
        };

        let drawable_width = surface_texture.texture.width();
        let drawable_height = surface_texture.texture.height();

        if drawable_width != width || drawable_height != height {
            println!(
                "[RESIZE SYNC] Window requested {}x{}, Metal swapchain delivered {}x{}",
                width, height, drawable_width, drawable_height
            );
        }

        if surface.target_texture.width() != drawable_width
            || surface.target_texture.height() != drawable_height
        {
            let device = &self.render_cx.devices[surface.dev_id].device;
            let target_texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("DirectedType Synced Target Texture"),
                size: wgpu::Extent3d {
                    width: drawable_width,
                    height: drawable_height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
                format: wgpu::TextureFormat::Rgba8Unorm,
                view_formats: &[],
            });
            surface.target_view = target_texture.create_view(&wgpu::TextureViewDescriptor::default());
            surface.target_texture = target_texture;
        }

        // 4. Render to surface target texture using actual drawable dimensions
        let device_handle = &self.render_cx.devices[surface.dev_id];
        let render_result = renderer.render_to_texture(
            &device_handle.device,
            &device_handle.queue,
            &final_scene,
            &surface.target_view,
            &RenderParams {
                base_color: Color::from_rgba8(241, 245, 249, 255), // light slate #f1f5f9
                width: drawable_width,
                height: drawable_height,
                antialiasing_method: AaConfig::Area,
            },
        );

        if let Err(e) = render_result {
            eprintln!("[RENDER ERROR] Failed to render scene: {e}");
            return;
        }

        // 5. Blit target texture to swapchain texture view
        let surface_view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = device_handle
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Testbed Blitter Command Encoder"),
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
        let elapsed = frame_start.elapsed();
        if self.frame_count.is_multiple_of(30) || self.frame_count <= 5 {
            println!(
                "[RENDER] Frame #{} rendered in {:.2}ms (surface: {}x{}, scale: {:.2})",
                self.frame_count,
                elapsed.as_secs_f64() * 1000.0,
                width,
                height,
                effective_scale
            );
        }
    }
}

impl ApplicationHandler for VelloTestbedApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        println!("[LIFECYCLE] resumed() invoked. Creating window...");

        let window_attrs = Window::default_attributes()
            .with_title("Vello Minimal Isolation Testbed")
            .with_inner_size(LogicalSize::new(800.0, 600.0));

        let window = match event_loop.create_window(window_attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                eprintln!("[ERROR] Failed to create window: {e}");
                event_loop.exit();
                return;
            }
        };

        let scale_factor = window.scale_factor();
        let inner = window.inner_size();
        println!(
            "[LIFECYCLE] Window created: reported inner_size = {:?}, scale_factor = {:.2}",
            inner, scale_factor
        );

        let width = if inner.width > 0 {
            inner.width
        } else {
            (800.0 * scale_factor).round() as u32
        }.max(1);

        let height = if inner.height > 0 {
            inner.height
        } else {
            (600.0 * scale_factor).round() as u32
        }.max(1);

        println!("[LIFECYCLE] Creating surface at physical {}x{} with present_mode={:?}...", width, height, self.present_mode);

        let surface = match pollster::block_on(self.render_cx.create_surface(
            window.clone(),
            width,
            height,
            self.present_mode,
        )) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[ERROR] Failed to create render surface: {e}");
                event_loop.exit();
                return;
            }
        };

        println!("[LIFECYCLE] Surface created. Initializing Vello renderer...");

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
                eprintln!("[ERROR] Failed to initialize Vello renderer: {e}");
                event_loop.exit();
                return;
            }
        };

        // Configure CAMetalLayer hierarchy: contentsGravity, contentsScale, presentsWithTransaction
        configure_metal_layer(&window, self.contents_gravity_top_left, self.presents_with_tx);

        self.renderer = Some(renderer);
        self.surface = Some(surface);
        self.window = Some(window);

        println!("[LIFECYCLE] Attempting initial frame #0 in resumed()...");
        self.render_frame();
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        // Keep requesting redraw on startup until the window un-occludes and renders its first frame
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
                println!("[LIFECYCLE] Window close requested. Exiting...");
                event_loop.exit();
            }

            WindowEvent::Occluded(is_occluded) => {
                println!("[EVENT] WindowEvent::Occluded({is_occluded})");
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

                    if self.sync_resize_render {
                        // Synchronous presentation during live AppKit resize drag
                        self.render_frame();
                    } else if let Some(window) = &self.window {
                        // Deferred redraw request
                        window.request_redraw();
                    }
                }
            }

            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                println!("[EVENT] ScaleFactorChanged -> {:.2}", scale_factor);
                if let Some(window) = &self.window {
                    configure_metal_layer(window, self.contents_gravity_top_left, self.presents_with_tx);
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
                Key::Character(c) if c.eq_ignore_ascii_case("s") => {
                    self.apply_dpi_scaling = !self.apply_dpi_scaling;
                    println!("[KEY] Toggled apply_dpi_scaling -> {}", self.apply_dpi_scaling);
                    self.render_frame();
                }
                Key::Character(c) if c.eq_ignore_ascii_case("m") => {
                    self.sync_resize_render = !self.sync_resize_render;
                    println!("[KEY] Toggled sync_resize_render -> {}", self.sync_resize_render);
                    self.render_frame();
                }
                Key::Character(c) if c.eq_ignore_ascii_case("t") => {
                    self.presents_with_tx = !self.presents_with_tx;
                    println!("[KEY] Toggled presents_with_tx -> {}", self.presents_with_tx);
                    if let Some(window) = &self.window {
                        configure_metal_layer(window, self.contents_gravity_top_left, self.presents_with_tx);
                    }
                    self.render_frame();
                }
                Key::Character(c) if c.eq_ignore_ascii_case("g") => {
                    self.contents_gravity_top_left = !self.contents_gravity_top_left;
                    println!("[KEY] Toggled contents_gravity_top_left -> {}", self.contents_gravity_top_left);
                    if let Some(window) = &self.window {
                        configure_metal_layer(window, self.contents_gravity_top_left, self.presents_with_tx);
                    }
                    self.render_frame();
                }
                Key::Character(c) if c.eq_ignore_ascii_case("p") => {
                    self.present_mode = match self.present_mode {
                        wgpu::PresentMode::AutoNoVsync => wgpu::PresentMode::AutoVsync,
                        _ => wgpu::PresentMode::AutoNoVsync,
                    };
                    println!("[KEY] Toggled present_mode -> {:?}", self.present_mode);
                    if let Some(surface) = &mut self.surface {
                        self.render_cx.set_present_mode(surface, self.present_mode);
                        if let Some(window) = &self.window {
                            configure_metal_layer(window, self.contents_gravity_top_left, self.presents_with_tx);
                        }
                    }
                    self.render_frame();
                }
                Key::Character(c) if c.eq_ignore_ascii_case("q") => {
                    println!("[KEY] Quit requested. Exiting...");
                    event_loop.exit();
                }
                Key::Named(NamedKey::Escape) => {
                    println!("[KEY] Escape pressed. Exiting...");
                    event_loop.exit();
                }
                _ => {}
            },

            _ => {}
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== DirectedType: Vello Minimal Isolation Testbed ===");
    println!("Interactive Diagnostic Controls:");
    println!("  [T] Toggle presentsWithTransaction (Atomic CATransaction sync)");
    println!("  [G] Toggle contentsGravity (TopLeft vs Resize)");
    println!("  [P] Toggle PresentMode (AutoNoVsync/Immediate vs AutoVsync/Fifo)");
    println!("  [M] Toggle Live Resize Render (Synchronous vs Deferred)");
    println!("  [S] Toggle Retina/DPI scaling (1.0 vs scale_factor)");
    println!("  [Q] or [Escape] Exit");
    println!("=====================================================");

    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = VelloTestbedApp::new();
    event_loop.run_app(&mut app)?;
    Ok(())
}
