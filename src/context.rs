use std::borrow::Cow;

use crate::{
    pipeline::{GpuVertex, Primitive},
    svg::Svg,
    text::{WgpuText, WgpuTextLayout},
    WgpuRenderer,
};
use futures::task::SpawnExt;
use lyon::lyon_tessellation::{
    BuffersBuilder, FillOptions, FillTessellator, FillVertex, StrokeOptions, StrokeTessellator,
    StrokeVertex, VertexBuffers,
};
use lyon::tessellation;
use piet::{
    kurbo::{Affine, Point, Rect, Shape, Size, Vec2},
    Color, FontFamily, Image, IntoBrush, RenderContext,
};

pub struct WgpuRenderContext<'a> {
    pub(crate) renderer: &'a mut WgpuRenderer,
    pub(crate) fill_tess: FillTessellator,
    pub(crate) stroke_tess: StrokeTessellator,
    pub(crate) geometry: VertexBuffers<GpuVertex, u32>,
    inner_text: WgpuText,
    pub(crate) cur_transform: Affine,
    state_stack: Vec<State>,
    clip_stack: Vec<Rect>,
    pub(crate) primitives: Vec<Primitive>,
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

impl<'a> WgpuRenderContext<'a> {
    pub fn new(renderer: &'a mut WgpuRenderer) -> Self {
        let text = renderer.text();
        let geometry: VertexBuffers<GpuVertex, u32> = VertexBuffers::new();

        let mut context = Self {
            renderer,
            fill_tess: FillTessellator::new(),
            stroke_tess: StrokeTessellator::new(),
            geometry,
            inner_text: text,
            cur_transform: Affine::default(),
            state_stack: Vec::new(),
            clip_stack: Vec::new(),
            primitives: Vec::new(),
        };
        context.add_primitive();
        context
    }

    fn pop_clip(&mut self) {
        self.clip_stack.pop();
    }

    pub(crate) fn current_clip(&self) -> Option<&Rect> {
        self.clip_stack.last()
    }

    fn add_primitive(&mut self) {
        let affine = self.cur_transform.as_coeffs();
        let translate = [affine[4] as f32, affine[5] as f32];
        let (clip, clip_rect) = self
            .current_clip()
            .map(|r| (1.0, [r.x0 as f32, r.y0 as f32, r.x1 as f32, r.y1 as f32]))
            .unwrap_or((0.0, [0.0, 0.0, 0.0, 0.0]));
        self.primitives.push(Primitive {
            translate,
            clip,
            clip_rect,
            ..Default::default()
        });
    }

    pub fn draw_svg(&mut self, svg: &Svg, rect: Rect, override_color: Option<&Color>) {
        let view_box = svg.tree.svg_node().view_box;
        let view_rect = view_box.rect;
        let scale =
            (rect.width() / view_rect.width()).min(rect.height() / view_rect.height()) as f32;

        let translate = [rect.x0 as f32, rect.y0 as f32];
        let override_color = override_color.map(|c| format_color(c));
        let svg_data = self.renderer.svg_store.get_svg_data(svg);
        let transforms = svg_data.transforms.clone();
        let offset = self.geometry.vertices.len() as u32;

        let primitive_id = self.primitives.len() as u32;
        for transform in transforms {
            self.add_primitive();
            let primitive = self.primitives.last_mut().unwrap();
            primitive.transform_1[0] *= scale * transform[0];
            primitive.transform_1[3] *= scale * transform[3];
            primitive.transform_2[0] += scale * transform[4];
            primitive.transform_2[1] += scale * transform[5];
        }
        self.add_primitive();

        let svg_data = self.renderer.svg_store.get_svg_data(svg);
        let mut vertices = svg_data
            .geometry
            .vertices
            .iter()
            .map(|v| {
                let mut v = v.clone();
                v.translate = translate;
                v.primitive_id = primitive_id + v.primitive_id;
                if let Some(c) = override_color.clone() {
                    v.color = c;
                }
                v
            })
            .collect();
        let mut indices = svg_data
            .geometry
            .indices
            .iter()
            .map(|i| *i + offset)
            .collect();
        self.geometry.vertices.append(&mut vertices);
        self.geometry.indices.append(&mut indices);
    }
}

#[derive(Clone)]
pub enum Brush {
    Solid(Color),
}

#[derive(Clone)]
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
        let color = format_color(&color);
        // let affine = self.cur_transform.as_coeffs();
        // let translate = [affine[4] as f32, affine[5] as f32];
        let primitive_id = self.primitives.len() as u32 - 1;

        if let Some(rect) = shape.as_rect() {
            self.stroke_tess.tessellate_rectangle(
                &lyon::geom::Rect::new(
                    lyon::geom::Point::new(rect.x0 as f32, rect.y0 as f32),
                    lyon::geom::Size::new(rect.width() as f32, rect.height() as f32),
                ),
                &StrokeOptions::tolerance(0.02)
                    .with_line_width(width as f32)
                    .with_line_cap(tessellation::LineCap::Round)
                    .with_line_join(tessellation::LineJoin::Round),
                &mut BuffersBuilder::new(&mut self.geometry, |vertex: StrokeVertex| {
                    let mut pos = vertex.position_on_path().to_array();
                    let normal = vertex.normal().to_array();
                    pos[0] += normal[0] * width as f32 / 2.0;
                    pos[1] += normal[1] * width as f32 / 2.0;
                    GpuVertex {
                        pos,
                        color,
                        primitive_id,
                        ..Default::default()
                    }
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
                &StrokeOptions::tolerance(0.02)
                    .with_line_width(width as f32)
                    .with_line_cap(tessellation::LineCap::Round)
                    .with_line_join(tessellation::LineJoin::Round),
                &mut BuffersBuilder::new(&mut self.geometry, |vertex: StrokeVertex| {
                    let mut pos = vertex.position_on_path().to_array();
                    let normal = vertex.normal().to_array();
                    pos[0] += normal[0] * width as f32 / 2.0;
                    pos[1] += normal[1] * width as f32 / 2.0;
                    GpuVertex {
                        pos,
                        color,
                        primitive_id,
                        ..Default::default()
                    }
                }),
            );
        } else {
            let mut builder = lyon::path::Path::builder();
            let mut in_subpath = false;
            for el in shape.path_elements(0.01) {
                match el {
                    piet::kurbo::PathEl::MoveTo(p) => {
                        builder.begin(lyon::geom::point(p.x as f32, p.y as f32));
                        in_subpath = true;
                    }
                    piet::kurbo::PathEl::LineTo(p) => {
                        builder.line_to(lyon::geom::point(p.x as f32, p.y as f32));
                    }
                    piet::kurbo::PathEl::QuadTo(ctrl, to) => {
                        builder.quadratic_bezier_to(
                            lyon::geom::point(ctrl.x as f32, ctrl.y as f32),
                            lyon::geom::point(to.x as f32, to.y as f32),
                        );
                    }
                    piet::kurbo::PathEl::CurveTo(c1, c2, p) => {
                        builder.cubic_bezier_to(
                            lyon::geom::point(c1.x as f32, c1.y as f32),
                            lyon::geom::point(c2.x as f32, c2.y as f32),
                            lyon::geom::point(p.x as f32, p.y as f32),
                        );
                    }
                    piet::kurbo::PathEl::ClosePath => {
                        in_subpath = false;
                        builder.close();
                    }
                }
            }
            if in_subpath {
                builder.end(false);
            }
            let path = builder.build();
            self.stroke_tess.tessellate_path(
                &path,
                &StrokeOptions::tolerance(0.02)
                    .with_line_width(width as f32)
                    .with_line_cap(tessellation::LineCap::Round)
                    .with_line_join(tessellation::LineJoin::Round),
                &mut BuffersBuilder::new(&mut self.geometry, |vertex: StrokeVertex| {
                    let mut pos = vertex.position_on_path().to_array();
                    let normal = vertex.normal().to_array();
                    pos[0] += normal[0] * width as f32 / 2.0;
                    pos[1] += normal[1] * width as f32 / 2.0;
                    GpuVertex {
                        pos,
                        color,
                        primitive_id,
                        ..Default::default()
                    }
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
            let color = format_color(&color);
            let primitive_id = self.primitives.len() as u32 - 1;
            self.fill_tess.tessellate_rectangle(
                &lyon::geom::Rect::new(
                    lyon::geom::Point::new(rect.x0 as f32, rect.y0 as f32),
                    lyon::geom::Size::new(rect.width() as f32, rect.height() as f32),
                ),
                &FillOptions::tolerance(0.02).with_fill_rule(tessellation::FillRule::NonZero),
                &mut BuffersBuilder::new(&mut self.geometry, |vertex: FillVertex| GpuVertex {
                    pos: vertex.position().to_array(),
                    color,
                    primitive_id,
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
            self.add_primitive();
        }
    }

    fn text(&mut self) -> &mut Self::Text {
        &mut self.inner_text
    }

    fn draw_text(&mut self, layout: &Self::TextLayout, pos: impl Into<piet::kurbo::Point>) {
        let point: Point = pos.into();
        let translate = [point.x as f32, point.y as f32];
        layout.draw_text(self, translate);
    }

    fn save(&mut self) -> Result<(), piet::Error> {
        self.state_stack.push(State {
            rel_transform: Affine::default(),
            transform: self.cur_transform,
            n_clip: 0,
        });
        Ok(())
    }

    fn restore(&mut self) -> Result<(), piet::Error> {
        if let Some(state) = self.state_stack.pop() {
            self.cur_transform = state.transform;
            for _ in 0..state.n_clip {
                self.pop_clip();
            }
            self.add_primitive();
            Ok(())
        } else {
            Err(piet::Error::StackUnbalance)
        }
    }

    fn finish(&mut self) -> Result<(), piet::Error> {
        self.renderer.ensure_encoder();
        let mut encoder = self.renderer.take_encoder();

        self.renderer.pipeline.upload_data(
            &self.renderer.device,
            &mut self.renderer.staging_belt.borrow_mut(),
            &mut encoder,
            &self.geometry,
            &self.primitives,
        );

        let texture = self
            .renderer
            .surface
            .get_current_texture()
            .map_err(|e| piet::Error::NotSupported)?;
        let view = texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        self.renderer.pipeline.draw(
            &self.renderer.device,
            &mut encoder,
            &view,
            &self.renderer.msaa,
            &self.geometry,
        );

        self.renderer.staging_belt.borrow_mut().finish();
        self.renderer.queue.submit(Some(encoder.finish()));
        texture.present();

        self.renderer
            .local_pool
            .spawner()
            .spawn(self.renderer.staging_belt.borrow_mut().recall())
            .expect("Recall staging belt");
        self.renderer.local_pool.run_until_stalled();

        Ok(())
    }

    fn transform(&mut self, transform: Affine) {
        if let Some(state) = self.state_stack.last_mut() {
            state.rel_transform *= transform;
        }
        self.cur_transform *= transform;
        self.add_primitive();
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
        let color = format_color(&color);

        self.add_primitive();
        let primitive = self.primitives.last_mut().unwrap();
        primitive.blur_radius = blur_radius as f32;
        primitive.blur_rect = [
            blur_rect.x0 as f32,
            blur_rect.y0 as f32,
            blur_rect.x1 as f32,
            blur_rect.y1 as f32,
        ];

        let primitive_id = self.primitives.len() as u32 - 1;
        self.fill_tess.tessellate_rectangle(
            &lyon::geom::Rect::new(
                lyon::geom::Point::new(rect.x0 as f32, rect.y0 as f32),
                lyon::geom::Size::new(rect.width() as f32, rect.height() as f32),
            ),
            &FillOptions::tolerance(0.02).with_fill_rule(tessellation::FillRule::NonZero),
            &mut BuffersBuilder::new(&mut self.geometry, |vertex: FillVertex| GpuVertex {
                pos: vertex.position().to_array(),
                color,
                primitive_id,
                ..Default::default()
            }),
        );
        self.add_primitive();
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

pub fn from_linear(x: f32) -> f32 {
    if x <= 0.04045 {
        x * (1.0 / 12.92)
    } else {
        ((x + 0.055) * (1.0 / 1.055)).powf(2.4)
    }
}

pub fn format_color(color: &Color) -> [f32; 4] {
    let color = color.as_rgba();
    [
        from_linear(color.0 as f32),
        from_linear(color.1 as f32),
        from_linear(color.2 as f32),
        color.3 as f32,
    ]
}
