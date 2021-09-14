mod context;
mod font;
mod layer;
mod pipeline;
mod svg;
mod text;
mod transformation;

use lyon::lyon_tessellation::{FillTessellator, StrokeTessellator};
pub use piet::kurbo;
use piet::kurbo::Size;
pub use piet::*;
use pipeline::{Cache, GlyphPosInfo};
pub use svg::Svg;
use svg::SvgStore;

use std::{cell::RefCell, marker::PhantomData, rc::Rc};

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
    device: Rc<wgpu::Device>,
    surface: wgpu::Surface,
    queue: wgpu::Queue,
    format: wgpu::TextureFormat,
    staging_belt: Rc<RefCell<wgpu::util::StagingBelt>>,
    local_pool: futures::executor::LocalPool,
    msaa: wgpu::TextureView,
    depth_view: wgpu::TextureView,
    size: Size,
    svg_store: SvgStore,

    text: WgpuText,

    pipeline: pipeline::Pipeline,
    pub(crate) encoder: Rc<RefCell<Option<wgpu::CommandEncoder>>>,
    pub(crate) fill_tess: FillTessellator,
    pub(crate) stroke_tess: StrokeTessellator,
}

impl WgpuRenderer {
    pub fn new<W: raw_window_handle::HasRawWindowHandle>(window: &W) -> Result<Self, piet::Error> {
        let instance = wgpu::Instance::new(wgpu::Backends::PRIMARY);
        let surface = unsafe { instance.create_surface(window) };
        let adapter =
            futures::executor::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
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

        let staging_belt = wgpu::util::StagingBelt::new(1024);
        let local_pool = futures::executor::LocalPool::new();

        let depth_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Depth buffer"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        });
        let depth_view = depth_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let msaa_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Multisampled frame descriptor"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 4,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Bgra8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        });
        let msaa = msaa_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let staging_belt = Rc::new(RefCell::new(staging_belt));
        let encoder = Rc::new(RefCell::new(None));
        let device = Rc::new(device);
        let text = WgpuText::new(device.clone(), staging_belt.clone(), encoder.clone());
        let pipeline = pipeline::Pipeline::new(&device, &text.cache.borrow());

        Ok(Self {
            instance,
            device,
            queue,
            surface,
            text,
            size: Size::ZERO,
            format,
            staging_belt,
            local_pool,
            msaa,
            depth_view,
            pipeline,
            svg_store: SvgStore::new(),
            encoder,
            fill_tess: FillTessellator::new(),
            stroke_tess: StrokeTessellator::new(),
        })
    }

    pub fn set_size(&mut self, size: Size) {
        self.size = size;
        let sc_desc = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: wgpu::TextureFormat::Bgra8Unorm,
            width: size.width as u32,
            height: size.height as u32,
            present_mode: wgpu::PresentMode::Mailbox,
        };
        self.surface.configure(&self.device, &sc_desc);
        let msaa_texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Multisampled frame descriptor"),
            size: wgpu::Extent3d {
                width: size.width as u32,
                height: size.height as u32,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 4,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Bgra8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        });
        self.msaa = msaa_texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.pipeline.size = size;

        let depth_texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Depth buffer"),
            size: wgpu::Extent3d {
                width: size.width as u32,
                height: size.height as u32,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        });
        self.depth_view = depth_texture.create_view(&wgpu::TextureViewDescriptor::default());
    }

    pub fn set_scale(&mut self, scale: f64) {
        self.pipeline.scale = scale;
        self.text.cache.borrow_mut().scale = scale;
    }

    pub fn text(&self) -> WgpuText {
        self.text.clone()
    }

    pub(crate) fn ensure_encoder(&mut self) {
        let mut encoder = self.encoder.borrow_mut();
        if encoder.is_none() {
            *encoder = Some(
                self.device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("render"),
                    }),
            );
        }
    }

    pub(crate) fn take_encoder(&mut self) -> wgpu::CommandEncoder {
        self.encoder.take().unwrap()
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
