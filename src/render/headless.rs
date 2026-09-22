use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::Arc;

use image::RgbaImage;
use parley::{FontContext, LayoutContext};
use vello::peniko::Color;
use vello::wgpu;
use vello::{AaConfig, AaSupport, RenderParams, Renderer, RendererOptions, Scene};

use crate::compiler::layout::ResolvedLayout;
use crate::render::error::RenderError;
use crate::render::scene::{build_scene, SceneOptions};

pub struct HeadlessRenderer {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    renderer: Renderer,
    font_cx: FontContext,
    layout_cx: LayoutContext<()>,
}

impl HeadlessRenderer {
    /// Initialize a new headless WGPU device and Vello renderer.
    pub fn new() -> Result<Self, RenderError> {
        let instance = wgpu::Instance::default();

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .map_err(|_| RenderError::NoAdapter)?;

        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("DirectedType Headless Device"),
                ..Default::default()
            },
        ))?;

        let renderer = Renderer::new(
            &device,
            RendererOptions {
                use_cpu: false,
                antialiasing_support: AaSupport::all(),
                num_init_threads: NonZeroUsize::new(1),
                pipeline_cache: None,
            },
        )
        .map_err(|e| RenderError::VelloRenderer(e.to_string()))?;

        Ok(Self {
            device: Arc::new(device),
            queue: Arc::new(queue),
            renderer,
            font_cx: FontContext::new(),
            layout_cx: LayoutContext::new(),
        })
    }

    /// Renders a `vello::Scene` directly into an RGBA image buffer.
    pub fn render_scene(
        &mut self,
        scene: &Scene,
        width: u32,
        height: u32,
    ) -> Result<RgbaImage, RenderError> {
        if width == 0 || height == 0 {
            return Err(RenderError::ImageBuffer(
                "Width and height must be non-zero".into(),
            ));
        }

        // 1. Target texture for Vello compute shader output
        let target_texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("DirectedType Headless Render Target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });

        let target_view = target_texture.create_view(&wgpu::TextureViewDescriptor::default());

        // 2. Render Vello Scene to texture
        self.renderer
            .render_to_texture(
                &self.device,
                &self.queue,
                scene,
                &target_view,
                &RenderParams {
                    base_color: Color::TRANSPARENT,
                    width,
                    height,
                    antialiasing_method: AaConfig::Area,
                },
            )
            .map_err(|e| RenderError::VelloRender(e.to_string()))?;

        // 3. Staging buffer for CPU readback
        let unpadded_bytes_per_row = width * 4;
        let padded_bytes_per_row = (unpadded_bytes_per_row + 255) & !255;
        let buffer_size = (padded_bytes_per_row * height) as u64;

        let staging_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("DirectedType Staging Buffer"),
            size: buffer_size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Readback Encoder"),
            });

        encoder.copy_texture_to_buffer(
            target_texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &staging_buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_bytes_per_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        self.queue.submit(Some(encoder.finish()));

        // 4. Map and read buffer
        let slice = staging_buffer.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(
            wgpu::MapMode::Read,
            move |result: Result<(), wgpu::BufferAsyncError>| {
                let _ = sender.send(result);
            },
        );

        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| RenderError::VelloRender(format!("GPU polling failed: {e}")))?;

        receiver
            .recv()
            .map_err(|_| RenderError::BufferMapFailed)?
            .map_err(|_| RenderError::BufferMapFailed)?;

        let mapped = slice.get_mapped_range();
        let mut unpadded_bytes = Vec::with_capacity((width * height * 4) as usize);

        for chunk in mapped.chunks(padded_bytes_per_row as usize) {
            unpadded_bytes.extend_from_slice(&chunk[..unpadded_bytes_per_row as usize]);
        }

        drop(mapped);
        staging_buffer.unmap();

        RgbaImage::from_raw(width, height, unpadded_bytes)
            .ok_or_else(|| RenderError::ImageBuffer("Failed to create RgbaImage from buffer".into()))
    }

    /// Builds a scene from `ResolvedLayout` and renders it into an image buffer.
    pub fn render_layout(
        &mut self,
        layout: &ResolvedLayout,
        width: u32,
        height: u32,
        options: &SceneOptions,
    ) -> Result<RgbaImage, RenderError> {
        let scene = build_scene(layout, &mut self.font_cx, &mut self.layout_cx, options);
        self.render_scene(&scene, width, height)
    }

    /// Renders `ResolvedLayout` and saves directly to a PNG file.
    pub fn render_layout_to_file(
        &mut self,
        layout: &ResolvedLayout,
        width: u32,
        height: u32,
        options: &SceneOptions,
        path: impl AsRef<Path>,
    ) -> Result<(), RenderError> {
        let image = self.render_layout(layout, width, height, options)?;
        image.save(path)?;
        Ok(())
    }
}
