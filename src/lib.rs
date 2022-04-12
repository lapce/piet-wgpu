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

use std::{marker::PhantomData, rc::Rc};

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
    pub fn new(
        context: &glutin::ContextWrapper<glutin::PossiblyCurrent, glutin::window::Window>,
    ) -> Result<Self, piet::Error> {
        let gl = unsafe {
            glow::Context::from_loader_function(|s| context.get_proc_address(s) as *const _)
        };

        let gl = Rc::new(gl);
        let text = WgpuText::new(gl.clone());
        let quad_pipeline = quad::Pipeline::new(&gl);
        let blur_quad_pipeline = blur_quad::Pipeline::new(&gl);
        let triangle_pipeline = triangle::Pipeline::new(&gl);
        let tex_pipeline = tex::Pipeline::new(&gl);

        Ok(Self {
            gl,
            text,
            size: Size::ZERO,
            svg_store: SvgStore::new(),
            quad_pipeline,
            blur_quad_pipeline,
            triangle_pipeline,
            tex_pipeline,
            scale: 1.0,
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

/// A struct provides a `RenderContext` and then can have its bitmap extracted.
pub struct BitmapTarget<'a> {
    phantom: PhantomData<&'a ()>,
}
