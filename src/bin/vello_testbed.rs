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
fn configure_metal_layer(window: &Window) {
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use std::ffi::CStr;

    if let Ok(handle) = window.window_handle() {
        if let RawWindowHandle::AppKit(appkit_handle) = handle.as_raw() {
            unsafe {
                let view = appkit_handle.ns_view.as_ptr() as *mut AnyObject;
                let layer: *mut AnyObject = msg_send![view, layer];
                if !layer.is_null() {
                    // 1. Set contentsGravity to @"topLeft" to prevent compositor from stretching stale frames
                    let s: *const CStr = c"topLeft";
                    let ns_string: *const AnyObject =
                        msg_send![class!(NSString), stringWithUTF8String: s.cast::<std::ffi::c_char>()];
                    let _: () = msg_send![layer, setContentsGravity: ns_string];

                    // 2. Set contentsScale to backingScaleFactor for 1:1 Retina mapping
                    let scale = window.scale_factor();
                    let _: () = msg_send![layer, setContentsScale: scale];
                    println!("[METAL] CAMetalLayer configured: contentsGravity=topLeft, contentsScale={scale:.2}");
                }
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn configure_metal_layer(_window: &Window) {}

/// Standalone minimal testbed isolating Winit + Vello rendering from any layout engine logic.
struct VelloTestbedApp {
    render_cx: RenderContext,
    surface: Option<RenderSurface<'static>>,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    font_cx: FontContext,
    layout_cx: LayoutContext<()>,

    // Testbed options
    apply_dpi_scaling: bool,
    sync_resize_render: bool,
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
        let card = RoundedRect::new(30.0, 30.0, 750.0, 520.0, 16.0);
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
            28.0,
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
            245.0,
            16.0,
            500.0,
            Color::from_rgba8(51, 65, 85, 255),
        );

        let stats_line_2 = format!(
            "DPI Scaling Mode: {} (effective scale: {:.2}) — [Press 'S' to toggle]",
            if self.apply_dpi_scaling { "ACTIVE" } else { "DISABLED (1:1 px)" },
            effective_scale
        );
        draw_text(
            &stats_line_2,
            60.0,
            275.0,
            16.0,
            600.0,
            if self.apply_dpi_scaling {
                Color::from_rgba8(2, 132, 199, 255)
            } else {
                Color::from_rgba8(217, 119, 6, 255)
            },
        );

        let stats_line_3 = format!(
            "Live Resize Mode: {} — [Press 'M' to toggle]",
            if self.sync_resize_render {
                "SYNCHRONOUS PRESENTATION"
            } else {
                "DEFERRED (request_redraw)"
            }
        );
        draw_text(
            &stats_line_3,
            60.0,
            305.0,
            16.0,
            600.0,
            if self.sync_resize_render {
                Color::from_rgba8(22, 163, 74, 255)
            } else {
                Color::from_rgba8(220, 38, 38, 255)
            },
        );

        // Benchmark font sizes
        draw_text(
            "Size 36px Heading Sample",
            60.0,
            350.0,
            36.0,
            700.0,
            Color::from_rgba8(15, 23, 42, 255),
        );

        draw_text(
            "Size 18px Subheading Sample: Directed acyclic graphs eliminate CSS layout thrashing.",
            60.0,
            405.0,
            18.0,
            400.0,
            Color::from_rgba8(71, 85, 105, 255),
        );

        draw_text(
            "Size 14px Body Sample: Controls -> [S] Toggle DPI Scaling | [M] Toggle Sync Resize | [Q/Esc] Quit",
            60.0,
            445.0,
            14.0,
            400.0,
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
            485.0,
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

        // 4. Render to surface target texture
        let device_handle = &self.render_cx.devices[surface.dev_id];
        let render_result = renderer.render_to_texture(
            &device_handle.device,
            &device_handle.queue,
            &final_scene,
            &surface.target_view,
            &RenderParams {
                base_color: Color::from_rgba8(241, 245, 249, 255), // light slate #f1f5f9
                width,
                height,
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

        println!("[LIFECYCLE] Creating surface at physical {}x{}...", width, height);

        let surface = match pollster::block_on(self.render_cx.create_surface(
            window.clone(),
            width,
            height,
            wgpu::PresentMode::AutoVsync,
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

        // Configure CAMetalLayer: contentsGravity = @"topLeft" and contentsScale = scale_factor
        configure_metal_layer(&window);

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
                Key::Character(c) if c.eq_ignore_ascii_case("s") => {
                    self.apply_dpi_scaling = !self.apply_dpi_scaling;
                    println!(
                        "[KEY] Toggled apply_dpi_scaling -> {}",
                        self.apply_dpi_scaling
                    );
                    self.render_frame();
                }
                Key::Character(c) if c.eq_ignore_ascii_case("m") => {
                    self.sync_resize_render = !self.sync_resize_render;
                    println!(
                        "[KEY] Toggled sync_resize_render -> {}",
                        self.sync_resize_render
                    );
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
    println!("Interactive Controls:");
    println!("  [S] Toggle Retina/DPI scaling (1.0 vs scale_factor)");
    println!("  [M] Toggle Sync Resize Render vs Deferred Redraw");
    println!("  [Q] or [Escape] Exit");
    println!("=====================================================");

    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = VelloTestbedApp::new();
    event_loop.run_app(&mut app)?;
    Ok(())
}
