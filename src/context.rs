use std::borrow::Cow;

use crate::{
    pipeline::{GpuVertex, Primitive},
    svg::Svg,
    text_layout::{WgpuText, WgpuTextLayout},
    transformation::Transformation,
    WgpuRenderer,
};
use bytemuck::{Pod, Zeroable};
use glow::HasContext;
use lyon::lyon_tessellation::{
    BuffersBuilder, FillTessellator, StrokeOptions, StrokeTessellator, StrokeVertex, VertexBuffers,
};
use lyon::tessellation;
use piet::{
    kurbo::{Affine, Point, Rect, Shape, Vec2},
    Color, Image, IntoBrush, RenderContext,
};

pub struct WgpuRenderContext<'a> {
    pub(crate) renderer: &'a mut WgpuRenderer,
    pub(crate) fill_tess: FillTessellator,
    pub(crate) stroke_tess: StrokeTessellator,
    pub(crate) geometry: VertexBuffers<GpuVertex, u32>,
    pub(crate) layer: Layer,
    inner_text: WgpuText,
    pub(crate) cur_transform: Affine,
    state_stack: Vec<State>,
    clip_stack: Vec<[f32; 4]>,
    pub(crate) primitives: Vec<Primitive>,
    pub(crate) depth: u32,
}

#[derive(Debug, Clone)]
pub(crate) struct Layer {
    pub quads: Vec<Quad>,
    pub transparent_quads: Vec<Quad>,
    pub blurred_quads: Vec<BlurQuad>,
    pub triangles: VertexBuffers<Vertex, u32>,
    pub transparent_triangles: VertexBuffers<Vertex, u32>,
    pub texts: Vec<Tex>,
    pub color_texts: Vec<Tex>,
    pub svgs: Vec<Tex>,
}

#[derive(Debug, Clone, Copy, Zeroable, Pod)]
#[repr(C)]
pub struct Quad {
    pub rect: [f32; 4],
    pub color: [f32; 4],
    pub depth: f32,
    pub clip: [f32; 4],
}

#[derive(Debug, Clone, Copy, Zeroable, Pod)]
#[repr(C)]
pub struct BlurQuad {
    pub rect: [f32; 4],
    pub blur_rect: [f32; 4],
    pub blur_radius: f32,
    pub color: [f32; 4],
    pub depth: f32,
    pub clip: [f32; 4],
}

#[derive(Debug, Clone, Copy, Zeroable, Pod)]
#[repr(C)]
pub struct Tex {
    pub rect: [f32; 4],
    pub tex_rect: [f32; 4],
    pub color: [f32; 4],
    pub depth: f32,
    pub clip: [f32; 4],
}

#[derive(Clone, Debug, Copy, Zeroable, Pod)]
#[repr(C)]
pub struct Vertex {
    pub(crate) pos: [f32; 2],
    pub(crate) color: [f32; 4],
    pub(crate) depth: f32,
    pub clip: [f32; 4],
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

impl Layer {
    fn new() -> Self {
        Self {
            quads: Vec::new(),
            blurred_quads: Vec::new(),
            triangles: VertexBuffers::new(),
            transparent_quads: Vec::new(),
            transparent_triangles: VertexBuffers::new(),
            texts: Vec::new(),
            color_texts: Vec::new(),
            svgs: Vec::new(),
        }
    }

    fn add_quad(&mut self, rect: [f32; 4], color: [f32; 4], depth: f32, clip: [f32; 4]) {
        let quad = Quad {
            rect,
            color,
            depth,
            clip,
        };
        if color[3] < 1.0 {
            self.transparent_quads.push(quad);
        } else {
            self.quads.push(quad);
        }
    }

    fn add_blurred_quad(
        &mut self,
        rect: [f32; 4],
        blur_rect: [f32; 4],
        blur_radius: f32,
        color: [f32; 4],
        depth: f32,
        clip: [f32; 4],
    ) {
        let quad = BlurQuad {
            rect,
            blur_rect,
            blur_radius,
            color,
            depth,
            clip,
        };
        self.blurred_quads.push(quad);
    }

    pub fn add_text(&mut self, mut text: Vec<Tex>) {
        self.texts.append(&mut text);
    }

    pub fn add_color_text(&mut self, mut text: Vec<Tex>) {
        self.color_texts.append(&mut text);
    }

    pub fn add_svg(&mut self, svg: Tex) {
        self.svgs.push(svg);
    }

    fn draw(&self, renderer: &mut WgpuRenderer, max_depth: u32) {
        unsafe {
            renderer.gl.disable(glow::BLEND);
            renderer.gl.blend_equation(glow::FUNC_ADD);
            renderer
                .gl
                .blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
        }

        let view_proj = create_view_proj(renderer.size.width as f32, renderer.size.height as f32);
        let scale = renderer.scale;
        renderer
            .quad_pipeline
            .draw(&renderer.gl, &self.quads, scale, &view_proj, max_depth);
        renderer.triangle_pipeline.draw(
            &renderer.gl,
            &self.triangles,
            scale,
            &view_proj,
            max_depth,
        );

        unsafe {
            renderer.gl.depth_mask(false);
            renderer.gl.enable(glow::BLEND);
            renderer.gl.blend_equation(glow::FUNC_ADD);
            renderer
                .gl
                .blend_func(glow::SRC1_COLOR, glow::ONE_MINUS_SRC1_COLOR);
        }
        renderer.tex_pipeline.draw(
            &renderer.gl,
            &self.texts,
            1.0,
            &view_proj,
            max_depth,
            renderer.text.cache.borrow().gl_texture,
            true,
        );

        unsafe {
            renderer.gl.enable(glow::BLEND);
            renderer.gl.blend_equation(glow::FUNC_ADD);
            renderer
                .gl
                .blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
        }
        renderer.tex_pipeline.draw(
            &renderer.gl,
            &self.svgs,
            scale,
            &view_proj,
            max_depth,
            renderer.svg_store.cache.gl_texture,
            false,
        );
        renderer.tex_pipeline.draw(
            &renderer.gl,
            &self.color_texts,
            scale,
            &view_proj,
            max_depth,
            renderer.text.cache.borrow().gl_texture,
            false,
        );

        renderer.quad_pipeline.draw(
            &renderer.gl,
            &self.transparent_quads,
            scale,
            &view_proj,
            max_depth,
        );
        renderer.blur_quad_pipeline.draw(
            &renderer.gl,
            &self.blurred_quads,
            scale,
            &view_proj,
            max_depth,
        );
        renderer.triangle_pipeline.draw(
            &renderer.gl,
            &self.transparent_triangles,
            scale,
            &view_proj,
            max_depth,
        );

        unsafe {
            renderer.gl.disable(glow::BLEND);
            renderer.gl.depth_mask(true);
        }
    }

    fn reset(&mut self) {
        self.quads.clear();
        self.blurred_quads.clear();
        self.triangles.vertices.clear();
        self.triangles.indices.clear();
        self.transparent_quads.clear();
        self.transparent_triangles.vertices.clear();
        self.transparent_triangles.indices.clear();
        self.texts.clear();
    }
}

impl<'a> WgpuRenderContext<'a> {
    pub fn new(renderer: &'a mut WgpuRenderer) -> Self {
        let text = renderer.text();
        let geometry: VertexBuffers<GpuVertex, u32> = VertexBuffers::new();

        let mut context = Self {
            layer: Layer::new(),
            renderer,
            fill_tess: FillTessellator::new(),
            stroke_tess: StrokeTessellator::new(),
            geometry,
            inner_text: text,
            cur_transform: Affine::default(),
            state_stack: Vec::new(),
            clip_stack: Vec::new(),
            primitives: Vec::new(),
            depth: 0,
        };
        context.add_primitive();
        context
    }

    fn pop_clip(&mut self) {
        self.clip_stack.pop();
    }

    pub(crate) fn current_clip(&self) -> [f32; 4] {
        self.get_current_clip().unwrap_or([0., 0., 0., 0.])
    }

    pub fn get_current_clip(&self) -> Option<[f32; 4]> {
        self.clip_stack.last().cloned()
    }

    fn add_primitive(&mut self) {
        let affine = self.cur_transform.as_coeffs();
        let translate = [affine[4] as f32, affine[5] as f32];
        let clip = 1.0;
        let clip_rect = self.current_clip();
        self.primitives.push(Primitive {
            translate,
            clip,
            clip_rect,
            ..Default::default()
        });
    }

    pub fn draw_svg(&mut self, svg: &Svg, rect: Rect, override_color: Option<&Color>) {
        let depth = self.depth as f32;
        let affine = self.cur_transform.as_coeffs();
        let clip = self.current_clip();
        let scale = self.renderer.scale;
        if let Ok(svg) = self.renderer.svg_store.cache.get_svg(
            &self.renderer.gl,
            svg,
            [rect.width() as f32 * scale, rect.height() as f32 * scale],
        ) {
            let color = override_color
                .map(format_color)
                .unwrap_or([0.0, 0.0, 0.0, 0.0]);
            let tex = Tex {
                rect: [
                    (rect.x0 + affine[4]).round() as f32,
                    (rect.y0 + affine[5]).round() as f32,
                    (rect.x1 + affine[4]).round() as f32,
                    (rect.y1 + affine[5]).round() as f32,
                ],
                tex_rect: [
                    svg.cache_rect.x0 as f32,
                    svg.cache_rect.y0 as f32,
                    svg.cache_rect.x1 as f32,
                    svg.cache_rect.y1 as f32,
                ],
                color,
                depth,
                clip,
            };
            self.layer.add_svg(tex);
        }
    }

    fn add_clip_rect(&mut self, rect: Rect) {
        self.clip_stack.push([
            rect.x0 as f32,
            rect.y0 as f32,
            rect.x1 as f32,
            rect.y1 as f32,
        ]);
        if let Some(state) = self.state_stack.last_mut() {
            state.n_clip += 1;
        }
        self.add_primitive();
    }

    pub fn clip_override(&mut self, shape: impl Shape) {
        if let Some(rect) = shape.as_rect() {
            let affine = self.cur_transform.as_coeffs();
            let rect = rect + Vec2::new(affine[4], affine[5]);

            self.add_clip_rect(rect);
        }
    }

    pub fn clip_nested(&mut self, shape: impl Shape) {
        if let Some([x0, y0, x1, y1]) = self.get_current_clip() {
            let current = Rect::new(x0 as f64, y0 as f64, x1 as f64, y1 as f64);

            if let Some(rect) = shape.as_rect() {
                let affine = self.cur_transform.as_coeffs();
                let rect = rect + Vec2::new(affine[4], affine[5]);
                let rect = rect.intersect(current);

                self.add_clip_rect(rect);
            }
        } else {
            self.clip_override(shape);
        }
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
        _gradient: impl Into<piet::FixedGradient>,
    ) -> Result<Self::Brush, piet::Error> {
        todo!()
    }

    fn clear(&mut self, _region: impl Into<Option<Rect>>, _color: Color) {}

    fn stroke(&mut self, shape: impl Shape, brush: &impl piet::IntoBrush<Self>, width: f64) {
        let affine = self.cur_transform.as_coeffs();
        self.depth += 1;
        let brush = brush.make_brush(self, || shape.bounding_box()).into_owned();
        let Brush::Solid(color) = brush;
        let color = format_color(&color);
        let depth = self.depth as f32;
        let clip = self.current_clip();

        let triangles = if color[3] < 1.0 {
            &mut self.layer.transparent_triangles
        } else {
            &mut self.layer.triangles
        };
        let mut stroke_builder = BuffersBuilder::new(triangles, |vertex: StrokeVertex| {
            let mut pos = vertex.position_on_path().to_array();
            let normal = vertex.normal().to_array();
            pos[0] += normal[0] * width as f32 / 2.0 + affine[4] as f32;
            pos[1] += normal[1] * width as f32 / 2.0 + affine[5] as f32;
            Vertex {
                pos,
                color,
                depth,
                clip,
            }
        });
        if let Some(rect) = shape.as_rect() {
            let _ = self.stroke_tess.tessellate_rectangle(
                &lyon::geom::Rect::new(
                    lyon::geom::Point::new(rect.x0 as f32, rect.y0 as f32),
                    lyon::geom::Size::new(rect.width() as f32, rect.height() as f32),
                ),
                &StrokeOptions::tolerance(0.02)
                    .with_line_width(width as f32)
                    .with_line_cap(tessellation::LineCap::Round)
                    .with_line_join(tessellation::LineJoin::Round),
                &mut stroke_builder,
            );
        } else if let Some(line) = shape.as_line() {
            let mut builder = lyon::path::Path::builder();
            builder.begin(lyon::geom::point(line.p0.x as f32, line.p0.y as f32));
            builder.line_to(lyon::geom::point(line.p1.x as f32, line.p1.y as f32));
            builder.close();
            let path = builder.build();
            let _ = self.stroke_tess.tessellate_path(
                &path,
                &StrokeOptions::tolerance(0.02)
                    .with_line_width(width as f32)
                    .with_line_cap(tessellation::LineCap::Round)
                    .with_line_join(tessellation::LineJoin::Round),
                &mut stroke_builder,
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
            let _ = self.stroke_tess.tessellate_path(
                &path,
                &StrokeOptions::tolerance(0.02)
                    .with_line_width(width as f32)
                    .with_line_cap(tessellation::LineCap::Round)
                    .with_line_join(tessellation::LineJoin::Round),
                &mut stroke_builder,
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
        let affine = self.cur_transform.as_coeffs();
        let clip = self.current_clip();

        self.depth += 1;
        let depth = self.depth as f32;
        if let Some(rect) = shape.as_rect() {
            let brush = brush.make_brush(self, || shape.bounding_box()).into_owned();
            let Brush::Solid(color) = brush;
            let color = format_color(&color);
            let rect = rect + Vec2::new(affine[4], affine[5]);

            self.layer.add_quad(
                [
                    rect.x0 as f32,
                    rect.y0 as f32,
                    rect.x1 as f32,
                    rect.y1 as f32,
                ],
                color,
                depth,
                clip,
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
        self.clip_nested(shape);
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
        let gl = &self.renderer.gl;
        unsafe {
            gl.clear_color(1.0, 1.0, 1.0, 1.0);
            gl.clear_depth_f32(1.0);
            gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
            gl.enable(glow::DEPTH_TEST);
            gl.depth_func(glow::LEQUAL);
            gl.depth_mask(true);
        }

        self.layer.draw(self.renderer, self.depth);

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
        self.depth += 1;
        let depth = self.depth as f32;

        let affine = self.cur_transform.as_coeffs();
        let clip_rect = self.current_clip();
        let rect = rect + Vec2::new(affine[4], affine[5]);
        let rect = rect.inflate(3.0 * blur_radius, 3.0 * blur_radius);
        let blur_rect = rect.inflate(-3.0 * blur_radius, -3.0 * blur_radius);
        let brush = brush.make_brush(self, || rect).into_owned();
        let Brush::Solid(color) = brush;
        let color = format_color(&color);
        self.layer.add_blurred_quad(
            [
                rect.x0 as f32,
                rect.y0 as f32,
                rect.x1 as f32,
                rect.y1 as f32,
            ],
            [
                blur_rect.x0 as f32,
                blur_rect.y0 as f32,
                blur_rect.x1 as f32,
                blur_rect.y1 as f32,
            ],
            blur_radius as f32,
            color,
            depth,
            clip_rect,
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

pub fn format_color(color: &Color) -> [f32; 4] {
    let color = color.as_rgba();
    [
        color.0 as f32,
        color.1 as f32,
        color.2 as f32,
        color.3 as f32,
    ]
}

fn create_view_proj(width: f32, height: f32) -> [f32; 16] {
    Transformation::orthographic(width, height).into()
}
