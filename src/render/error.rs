use thiserror::Error;
use vello::wgpu;

#[derive(Debug, Error)]
pub enum RenderError {
    #[error("Failed to find suitable GPU adapter")]
    NoAdapter,

    #[error("Failed to request WGPU device: {0}")]
    DeviceRequest(#[from] wgpu::RequestDeviceError),

    #[error("Failed to create Vello renderer: {0}")]
    VelloRenderer(String),

    #[error("Vello rendering failure: {0}")]
    VelloRender(String),

    #[error("Buffer mapping failed")]
    BufferMapFailed,

    #[error("Image buffer error: {0}")]
    ImageBuffer(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Image error: {0}")]
    Image(#[from] image::ImageError),
}
