mod context;
mod layer;
mod pipeline;
mod quad;
mod text;
mod transformation;

pub use piet::kurbo;
use piet::kurbo::Size;
pub use piet::*;

use std::marker::PhantomData;

use context::{WgpuImage, WgpuRenderContext};
use text::{WgpuText, WgpuTextLayout, WgpuTextLayoutBuilder};

pub type Piet<'a> = WgpuRenderContext<'a>;

pub type Brush = context::Brush;

pub type PietText = WgpuText;

pub type PietTextLayout = WgpuTextLayout;

pub type PietTextLayoutBuilder = WgpuTextLayoutBuilder;

pub type PietImage = WgpuImage;

pub struct WgpuRenderer {
    instance: wgpu::Instance,
    device: wgpu::Device,
    surface: wgpu::Surface,
    queue: wgpu::Queue,
    format: wgpu::TextureFormat,
    staging_belt: wgpu::util::StagingBelt,
    local_pool: futures::executor::LocalPool,

    quad_pipeline: quad::Pipeline,
    pipeline: pipeline::Pipeline,
}

impl WgpuRenderer {
    pub fn new<W: raw_window_handle::HasRawWindowHandle>(window: &W) -> Result<Self, piet::Error> {
        let instance = wgpu::Instance::new(wgpu::Backends::PRIMARY);
        let surface = unsafe { instance.create_surface(window) };
        let adapter =
            futures::executor::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                compatible_surface: Some(&surface),
            }))
            .ok_or(piet::Error::NotSupported)?;
        let (device, queue) = futures::executor::block_on(
            adapter.request_device(&wgpu::DeviceDescriptor::default(), None),
        )
        .map_err(|e| piet::Error::BackendError(Box::new(e)))?;

        let format = surface
            .get_preferred_format(&adapter)
            .ok_or(piet::Error::NotSupported)?;

        let staging_belt = wgpu::util::StagingBelt::new(10 * 1024);
        let local_pool = futures::executor::LocalPool::new();

        let quad_pipeline = quad::Pipeline::new(&device);
        let pipeline = pipeline::Pipeline::new(&device);

        Ok(Self {
            instance,
            device,
            queue,
            surface,
            format,
            staging_belt,
            local_pool,
            quad_pipeline,
            pipeline,
        })
    }

    pub fn set_size(&mut self, size: Size) {
        let sc_desc = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: wgpu::TextureFormat::Bgra8Unorm,
            width: size.width as u32,
            height: size.height as u32,
            present_mode: wgpu::PresentMode::Mailbox,
        };
        self.surface.configure(&self.device, &sc_desc);
        self.pipeline.size = size;
    }

    pub fn set_scale(&mut self, scale: f64) {
        self.pipeline.scale = scale;
    }
}

pub struct Device {
    // Since not all backends can support `Device: Sync`, make it non-Sync here to, for fewer
    // portability surprises.
    marker: std::marker::PhantomData<*const ()>,
}

unsafe impl Send for Device {}

/// A struct provides a `RenderContext` and then can have its bitmap extracted.
pub struct BitmapTarget<'a> {
    phantom: PhantomData<&'a ()>,
}
