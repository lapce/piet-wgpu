use std::borrow::Cow;

use crate::{
    pipeline::GpuVertex,
    quad::Quad,
    text::{WgpuText, WgpuTextLayout},
    text_pipeline::Instance,
    transformation::Transformation,
    WgpuRenderer,
};
use font_kit::{
    canvas::{Canvas, Format, RasterizationOptions},
    family_name::FamilyName,
    hinting::HintingOptions,
    source::SystemSource,
};
use futures::task::SpawnExt;
use lyon::lyon_tessellation::{
    BuffersBuilder, FillOptions, FillTessellator, FillVertex, StrokeOptions, StrokeTessellator,
    StrokeVertex, VertexBuffers,
};
use lyon::tessellation;
use pathfinder_geometry::{
    transform2d::Transform2F,
    vector::{Vector2F, Vector2I},
};
use piet::{
    kurbo::{Affine, Point, Rect, Shape, Size, Vec2},
    Color, FontFamily, Image, IntoBrush, RenderContext,
};

pub struct WgpuRenderContext<'a> {
    pub(crate) renderer: &'a mut WgpuRenderer,
    view: wgpu::TextureView,
    frame: wgpu::SurfaceFrame,
    pub(crate) encoder: Option<wgpu::CommandEncoder>,
    msaa: wgpu::TextureView,
    pub(crate) fill_tess: FillTessellator,
    pub(crate) stroke_tess: StrokeTessellator,
    pub(crate) geometry: VertexBuffers<GpuVertex, u32>,
    elements: Vec<Element>,
    inner_text: WgpuText,
    pub(crate) cur_transform: Affine,
    pub(crate) cur_depth: f32,
    depth_step: f32,
    pending_text: bool,
    state_stack: Vec<State>,
    clip_stack: Vec<Rect>,
}

#[derive(Default)]
struct State {
    /// The transform relative to the parent state.
    rel_transform: Affine,
    /// The transform at the parent state.
    ///
    /// This invariant should hold: transform * rel_transform = cur_transform
    transform: Affine,
    n_clip: usize,
}

enum Element {
    Fill(Rect, Color, Affine, Rect),
}

impl<'a> WgpuRenderContext<'a> {
    pub fn new(renderer: &'a mut WgpuRenderer) -> Self {
        let text = renderer.text();
        let geometry: VertexBuffers<GpuVertex, u32> = VertexBuffers::new();
        let frame = renderer.surface.get_current_frame().unwrap();
        let mut encoder = renderer
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("render"),
            });
        let view = frame
            .output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let texture = renderer.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Multisampled frame descriptor"),
            size: wgpu::Extent3d {
                width: renderer.pipeline.size.width as u32,
                height: renderer.pipeline.size.height as u32,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 4,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Bgra8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        });
        let msaa = texture.create_view(&wgpu::TextureViewDescriptor::default());

        {
            let _ = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                        store: true,
                    },
                }],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &renderer.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(-1.0),
                        store: true,
                    }),
                    stencil_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0),
                        store: true,
                    }),
                }),
            });
        }

        Self {
            renderer,
            view,
            frame,
            encoder: Some(encoder),
            msaa,
            fill_tess: FillTessellator::new(),
            stroke_tess: StrokeTessellator::new(),
            geometry,
            elements: Vec::new(),
            inner_text: text,
            cur_transform: Affine::default(),
            state_stack: Vec::new(),
            clip_stack: Vec::new(),
            cur_depth: 0.0,
            depth_step: 0.0001,
            pending_text: false,
        }
    }

    pub fn pop_clip(&mut self) {
        self.clip_stack.pop();
    }

    pub(crate) fn current_clip(&self) -> Option<&Rect> {
        self.clip_stack.last()
    }

    pub(crate) fn render(&mut self) {
        // self.render_text();
        // self.render_shapes();
    }

    pub(crate) fn render_shapes(&mut self) {
        if self.geometry.vertices.len() > 0 && self.geometry.indices.len() > 0 {
            self.renderer.pipeline.draw(
                &self.renderer.device,
                &mut self.renderer.staging_belt,
                &mut self.encoder.as_mut().unwrap(),
                &self.view,
                &self.msaa,
                wgpu::RenderPassDepthStencilAttachment {
                    view: &self.renderer.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: true,
                    }),
                    stencil_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: true,
                    }),
                },
                &self.geometry,
            );
            self.geometry.vertices.clear();
            self.geometry.indices.clear();
        }
    }

    pub(crate) fn render_text(&mut self) {
        //let affine = self.cur_transform.as_coeffs();
        //let translate = [affine[4] as f32, affine[5] as f32];
        self.renderer.text_pipeline.draw(
            &self.renderer.device,
            &mut self.renderer.staging_belt,
            &mut self.encoder.as_mut().unwrap(),
            &self.view,
            wgpu::RenderPassDepthStencilAttachment {
                view: &self.renderer.depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: true,
                }),
                stencil_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: true,
                }),
            },
            [0.0, 0.0],
        );
    }
}

#[derive(Clone)]
pub enum Brush {
    Solid(Color),
}

pub struct WgpuImage {}

impl<'a> RenderContext for WgpuRenderContext<'a> {
    type Brush = Brush;
    type Text = WgpuText;
    type TextLayout = WgpuTextLayout;
    type Image = WgpuImage;

    fn status(&mut self) -> Result<(), piet::Error> {
        todo!()
    }

    fn solid_brush(&mut self, color: Color) -> Self::Brush {
        Brush::Solid(color)
    }

    fn gradient(
        &mut self,
        gradient: impl Into<piet::FixedGradient>,
    ) -> Result<Self::Brush, piet::Error> {
        todo!()
    }

    fn clear(&mut self, region: impl Into<Option<Rect>>, color: Color) {}

    fn stroke(&mut self, shape: impl Shape, brush: &impl piet::IntoBrush<Self>, width: f64) {
        let brush = brush.make_brush(self, || shape.bounding_box()).into_owned();
        let Brush::Solid(color) = brush;
        let color = color.as_rgba();
        let color = [
            color.0 as f32,
            color.1 as f32,
            color.2 as f32,
            color.3 as f32,
        ];
        let affine = self.cur_transform.as_coeffs();
        let translate = [affine[4] as f32, affine[5] as f32];
        let z = self.cur_depth;

        if let Some(rect) = shape.as_rect() {
            self.stroke_tess.tessellate_rectangle(
                &lyon::geom::Rect::new(
                    lyon::geom::Point::new(rect.x0 as f32, rect.y0 as f32),
                    lyon::geom::Size::new(rect.width() as f32, rect.height() as f32),
                ),
                &StrokeOptions::tolerance(0.02).with_line_width(width as f32),
                &mut BuffersBuilder::new(&mut self.geometry, |vertex: StrokeVertex| GpuVertex {
                    pos: vertex.position().to_array(),
                    z,
                    translate,
                    color,
                    normal: vertex.normal().to_array(),
                    width: width as f32,
                    ..Default::default()
                }),
            );
        } else if let Some(line) = shape.as_line() {
            let mut builder = lyon::path::Path::builder();
            builder.begin(lyon::geom::point(line.p0.x as f32, line.p0.y as f32));
            builder.line_to(lyon::geom::point(line.p1.x as f32, line.p1.y as f32));
            builder.close();
            let path = builder.build();
            self.stroke_tess.tessellate_path(
                &path,
                &StrokeOptions::tolerance(0.02),
                &mut BuffersBuilder::new(&mut self.geometry, |vertex: StrokeVertex| GpuVertex {
                    pos: vertex.position_on_path().to_array(),
                    translate,
                    z,
                    color,
                    normal: vertex.normal().to_array(),
                    width: width as f32,
                    ..Default::default()
                }),
            );
        }
    }

    fn stroke_styled(
        &mut self,
        shape: impl piet::kurbo::Shape,
        brush: &impl piet::IntoBrush<Self>,
        width: f64,
        style: &piet::StrokeStyle,
    ) {
    }

    fn fill(&mut self, shape: impl piet::kurbo::Shape, brush: &impl piet::IntoBrush<Self>) {
        if let Some(rect) = shape.as_rect() {
            let brush = brush.make_brush(self, || shape.bounding_box()).into_owned();
            let Brush::Solid(color) = brush;
            let color = color.as_rgba();
            let color = [
                color.0 as f32,
                color.1 as f32,
                color.2 as f32,
                color.3 as f32,
            ];
            let affine = self.cur_transform.as_coeffs();
            let translate = [affine[4] as f32, affine[5] as f32];
            let z = self.cur_depth;
            self.fill_tess.tessellate_rectangle(
                &lyon::geom::Rect::new(
                    lyon::geom::Point::new(rect.x0 as f32, rect.y0 as f32),
                    lyon::geom::Size::new(rect.width() as f32, rect.height() as f32),
                ),
                &FillOptions::tolerance(0.02).with_fill_rule(tessellation::FillRule::NonZero),
                &mut BuffersBuilder::new(&mut self.geometry, |vertex: FillVertex| GpuVertex {
                    pos: vertex.position().to_array(),
                    z,
                    translate,
                    color,
                    ..Default::default()
                }),
            );
        }
    }

    fn fill_even_odd(
        &mut self,
        shape: impl piet::kurbo::Shape,
        brush: &impl piet::IntoBrush<Self>,
    ) {
    }

    fn clip(&mut self, shape: impl Shape) {
        if let Some(rect) = shape.as_rect() {
            let affine = self.cur_transform.as_coeffs();
            let rect = rect + Vec2::new(affine[4], affine[5]);
            self.clip_stack.push(rect);
            if let Some(state) = self.state_stack.last_mut() {
                state.n_clip += 1;
            }
        }
    }

    fn text(&mut self) -> &mut Self::Text {
        &mut self.inner_text
    }

    fn draw_text(&mut self, layout: &Self::TextLayout, pos: impl Into<piet::kurbo::Point>) {
        let affine = self.cur_transform.as_coeffs();
        let point: Point = pos.into();
        let translate = [(affine[4] + point.x) as f32, (affine[5] + point.y) as f32];
        layout.draw_text(self, translate);
    }

    fn save(&mut self) -> Result<(), piet::Error> {
        self.cur_depth += self.depth_step;
        self.state_stack.push(State {
            rel_transform: Affine::default(),
            transform: self.cur_transform,
            n_clip: 0,
        });
        Ok(())
    }

    fn restore(&mut self) -> Result<(), piet::Error> {
        // self.render();
        if let Some(state) = self.state_stack.pop() {
            self.cur_transform = state.transform;
            for _ in 0..state.n_clip {
                self.pop_clip();
            }
            Ok(())
        } else {
            Err(piet::Error::StackUnbalance)
        }
    }

    fn finish(&mut self) -> Result<(), piet::Error> {
        self.renderer.pipeline.draw(
            &self.renderer.device,
            &mut self.renderer.staging_belt,
            &mut self.encoder.as_mut().unwrap(),
            &self.view,
            &self.msaa,
            wgpu::RenderPassDepthStencilAttachment {
                view: &self.renderer.depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: true,
                }),
                stencil_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: true,
                }),
            },
            &self.geometry,
        );

        self.renderer.staging_belt.finish();
        let commond_buffer = self.encoder.take().unwrap().finish();
        self.renderer.queue.submit(Some(commond_buffer));

        self.renderer
            .local_pool
            .spawner()
            .spawn(self.renderer.staging_belt.recall())
            .expect("Recall staging belt");
        self.renderer.local_pool.run_until_stalled();

        Ok(())
    }

    fn transform(&mut self, transform: Affine) {
        if let Some(state) = self.state_stack.last_mut() {
            state.rel_transform *= transform;
        }
        self.cur_transform *= transform;
    }

    fn make_image(
        &mut self,
        width: usize,
        height: usize,
        buf: &[u8],
        format: piet::ImageFormat,
    ) -> Result<Self::Image, piet::Error> {
        todo!()
    }

    fn draw_image(
        &mut self,
        image: &Self::Image,
        dst_rect: impl Into<piet::kurbo::Rect>,
        interp: piet::InterpolationMode,
    ) {
        todo!()
    }

    fn draw_image_area(
        &mut self,
        image: &Self::Image,
        src_rect: impl Into<piet::kurbo::Rect>,
        dst_rect: impl Into<piet::kurbo::Rect>,
        interp: piet::InterpolationMode,
    ) {
        todo!()
    }

    fn capture_image_area(
        &mut self,
        src_rect: impl Into<piet::kurbo::Rect>,
    ) -> Result<Self::Image, piet::Error> {
        todo!()
    }

    fn blurred_rect(
        &mut self,
        rect: piet::kurbo::Rect,
        blur_radius: f64,
        brush: &impl piet::IntoBrush<Self>,
    ) {
        let rect = rect.inflate(3.0 * blur_radius, 3.0 * blur_radius);
        let blur_rect = rect.inflate(-3.0 * blur_radius, -3.0 * blur_radius);
        let brush = brush.make_brush(self, || rect).into_owned();
        let Brush::Solid(color) = brush;
        let color = color.as_rgba();
        let color = [
            color.0 as f32,
            color.1 as f32,
            color.2 as f32,
            color.3 as f32,
        ];
        let affine = self.cur_transform.as_coeffs();
        let translate = [affine[4] as f32, affine[5] as f32];
        let z = self.cur_depth;
        self.fill_tess.tessellate_rectangle(
            &lyon::geom::Rect::new(
                lyon::geom::Point::new(rect.x0 as f32, rect.y0 as f32),
                lyon::geom::Size::new(rect.width() as f32, rect.height() as f32),
            ),
            &FillOptions::tolerance(0.02).with_fill_rule(tessellation::FillRule::NonZero),
            &mut BuffersBuilder::new(&mut self.geometry, |vertex: FillVertex| GpuVertex {
                pos: vertex.position().to_array(),
                z,
                translate,
                color,
                blur_radius: blur_radius as f32,
                blur_rect: [
                    blur_rect.x0 as f32,
                    blur_rect.y0 as f32,
                    blur_rect.x1 as f32,
                    blur_rect.y1 as f32,
                ],
                ..Default::default()
            }),
        );
    }

    fn current_transform(&self) -> piet::kurbo::Affine {
        self.cur_transform
    }

    fn with_save(
        &mut self,
        f: impl FnOnce(&mut Self) -> Result<(), piet::Error>,
    ) -> Result<(), piet::Error> {
        self.save()?;
        // Always try to restore the stack, even if `f` errored.
        f(self).and(self.restore())
    }
}

impl<'a> IntoBrush<WgpuRenderContext<'a>> for Brush {
    fn make_brush<'b>(
        &'b self,
        piet: &mut WgpuRenderContext,
        bbox: impl FnOnce() -> piet::kurbo::Rect,
    ) -> std::borrow::Cow<'b, Brush> {
        Cow::Borrowed(self)
    }
}

impl Image for WgpuImage {
    fn size(&self) -> piet::kurbo::Size {
        todo!()
    }
}
