mod blur_quad;
mod context;
mod pipeline;
mod quad;
mod svg;
mod tex;
mod text;
mod text_layout;
mod transformation;
mod triangle;

use glow::HasContext;
pub use piet::kurbo;
use piet::kurbo::Size;
pub use piet::*;
pub use svg::Svg;
use svg::SvgStore;

use std::{marker::PhantomData, path::Path, rc::Rc};

use context::{WgpuImage, WgpuRenderContext};
use text_layout::{WgpuText, WgpuTextLayout, WgpuTextLayoutBuilder};

pub type Piet<'a> = WgpuRenderContext<'a>;

pub type Brush = context::Brush;

pub type PietText = WgpuText;

pub type PietTextLayout = WgpuTextLayout;

pub type PietTextLayoutBuilder = WgpuTextLayoutBuilder;

pub type PietImage = WgpuImage;

pub struct WgpuRenderer {
    gl: Rc<glow::Context>,
    size: Size,
    scale: f32,
    svg_store: SvgStore,

    text: WgpuText,

    quad_pipeline: quad::Pipeline,
    blur_quad_pipeline: blur_quad::Pipeline,
    triangle_pipeline: triangle::Pipeline,
    tex_pipeline: tex::Pipeline,
}

impl WgpuRenderer {
    pub fn new<F>(loader_function: F) -> Result<Self, piet::Error>
    where
        F: FnMut(&str) -> *const std::os::raw::c_void,
    {
        let gl = unsafe { glow::Context::from_loader_function(loader_function) };

        let gl = Rc::new(gl);
        let text = WgpuText::new(&gl);
        let quad_pipeline = quad::Pipeline::new(&gl);
        let blur_quad_pipeline = blur_quad::Pipeline::new(&gl);
        let triangle_pipeline = triangle::Pipeline::new(&gl);
        let tex_pipeline = tex::Pipeline::new(&gl);

        Ok(Self {
            text,
            size: Size::ZERO,
            svg_store: SvgStore::new(&gl),
            quad_pipeline,
            blur_quad_pipeline,
            triangle_pipeline,
            tex_pipeline,
            scale: 1.0,
            gl,
        })
    }

    pub fn set_size(&mut self, size: Size) {
        self.size = size;
        unsafe {
            self.gl
                .viewport(0, 0, self.size.width as i32, self.size.height as i32);
        }
    }

    pub fn set_scale(&mut self, scale: f64) {
        self.text.cache.borrow_mut().scale = scale;
        self.scale = scale as f32;
    }

    pub fn text(&self) -> WgpuText {
        self.text.clone()
    }
}

pub struct Device {
    // Since not all backends can support `Device: Sync`, make it non-Sync here to, for fewer
    // portability surprises.
    marker: std::marker::PhantomData<*const ()>,
}

unsafe impl Send for Device {}

impl Device {
    /// Create a new device.
    pub fn new() -> Result<Device, piet::Error> {
        Ok(Device {
            marker: std::marker::PhantomData,
        })
    }

    /// Create a new bitmap target.
    pub fn bitmap_target(
        &mut self,
        _width: usize,
        _height: usize,
        _pix_scale: f64,
    ) -> Result<BitmapTarget, piet::Error> {
        let phantom = Default::default();
        Ok(BitmapTarget { phantom })
    }
}

/// A struct provides a `RenderContext` and then can have its bitmap extracted.
pub struct BitmapTarget<'a> {
    phantom: PhantomData<&'a ()>,
}

impl<'a> BitmapTarget<'a> {
    pub fn to_image_buf(&mut self, _fmt: ImageFormat) -> Result<ImageBuf, piet::Error> {
        Ok(ImageBuf::empty())
    }

    pub fn save_to_file<P: AsRef<Path>>(self, _path: P) -> Result<(), piet::Error> {
        Err(piet::Error::Unimplemented)
    }
}
