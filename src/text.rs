use std::{cell::RefCell, collections::HashMap, ops::Range, rc::Rc};

use font_kit::source::SystemSource;
use lyon::lyon_tessellation::{
    BuffersBuilder, FillOptions, FillTessellator, FillVertex, StrokeOptions, StrokeTessellator,
    StrokeVertex, VertexBuffers,
};
use piet::kurbo::Line;
use piet::Color;
use piet::{
    kurbo::{Point, Size},
    FontFamily, FontStyle, FontWeight, HitTestPoint, HitTestPosition, LineMetric, Text,
    TextAttribute, TextLayout, TextLayoutBuilder, TextStorage,
};

use crate::context::WgpuRenderContext;
use crate::pipeline::{Cache, GlyphMetricInfo, GlyphPosInfo, GpuVertex};

#[derive(Clone)]
pub struct WgpuText {
    source: Rc<RefCell<SystemSource>>,
    glyphs: Rc<RefCell<HashMap<FontFamily, HashMap<char, Rc<(Vec<[f32; 2]>, Vec<u32>)>>>>>,
    pub(crate) cache: Rc<RefCell<Cache>>,
    device: Rc<wgpu::Device>,
    staging_belt: Rc<RefCell<wgpu::util::StagingBelt>>,
    encoder: Rc<RefCell<Option<wgpu::CommandEncoder>>>,
    fill_tess: Rc<RefCell<FillTessellator>>,
    stroke_tess: Rc<RefCell<StrokeTessellator>>,
}

impl WgpuText {
    pub(crate) fn new(
        device: Rc<wgpu::Device>,
        staging_belt: Rc<RefCell<wgpu::util::StagingBelt>>,
        encoder: Rc<RefCell<Option<wgpu::CommandEncoder>>>,
    ) -> Self {
        Self {
            source: Rc::new(RefCell::new(SystemSource::new())),
            glyphs: Rc::new(RefCell::new(HashMap::new())),
            cache: Rc::new(RefCell::new(Cache::new(&device, 2000, 2000))),
            device,
            staging_belt,
            encoder,
            fill_tess: Rc::new(RefCell::new(FillTessellator::new())),
            stroke_tess: Rc::new(RefCell::new(StrokeTessellator::new())),
        }
    }

    pub(crate) fn get_glyph_pos(
        &self,
        c: char,
        font_family: FontFamily,
        font_size: f32,
        font_weight: FontWeight,
    ) -> Result<GlyphPosInfo, piet::Error> {
        let mut encoder = self.encoder.borrow_mut();
        if encoder.is_none() {
            *encoder = Some(
                self.device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("render"),
                    }),
            );
        }

        let mut cache = self.cache.borrow_mut();
        cache
            .get_glyph_pos(
                c,
                font_family,
                font_size,
                font_weight,
                &self.device,
                &mut self.staging_belt.borrow_mut(),
                encoder.as_mut().unwrap(),
            )
            .map(|p| p.clone())
    }
}

#[derive(Clone)]
pub struct WgpuTextLayout {
    state: WgpuText,
    text: String,
    attrs: Rc<Attributes>,
    ref_glyph: Rc<RefCell<GlyphPosInfo>>,
    glyphs: Rc<RefCell<Vec<GlyphPosInfo>>>,
    geometry: Rc<RefCell<VertexBuffers<GpuVertex, u32>>>,
}

impl WgpuTextLayout {
    pub fn new(text: String, state: WgpuText) -> Self {
        let char_number = text.chars().count();
        let num_vertices = char_number * 4;
        let num_indices = char_number * 6;
        Self {
            state,
            text,
            attrs: Rc::new(Attributes::default()),
            glyphs: Rc::new(RefCell::new(Vec::new())),
            ref_glyph: Rc::new(RefCell::new(GlyphPosInfo::default())),
            geometry: Rc::new(RefCell::new(VertexBuffers::with_capacity(
                num_vertices,
                num_indices,
            ))),
        }
    }

    fn set_attrs(&mut self, attrs: Attributes) {
        self.attrs = Rc::new(attrs);
    }

    pub(crate) fn rebuild(&self, bounds: Option<[f64; 2]>) {
        let font_family = self.attrs.defaults.font.clone();
        let font_size = self.attrs.defaults.font_size;
        let font_weight = self.attrs.defaults.weight;
        if let Ok(glyph_pos) =
            self.state
                .get_glyph_pos('W', font_family, font_size as f32, font_weight)
        {
            *self.ref_glyph.borrow_mut() = glyph_pos.clone();
        }

        let mut glyphs = self.glyphs.borrow_mut();
        glyphs.clear();
        let mut geometry = self.geometry.borrow_mut();
        geometry.vertices.clear();
        geometry.indices.clear();

        let mut x = 0.0;
        let mut y = 0.0;
        for (index, c) in self.text.chars().enumerate() {
            let font_family = self.attrs.font(index);
            let font_size = self.attrs.size(index) as f32;
            let font_weight = self.attrs.font_weight(index);
            let color = self.attrs.color(index);
            let color = color.as_rgba();
            let color = [
                color.0 as f32,
                color.1 as f32,
                color.2 as f32,
                color.3 as f32,
            ];
            if let Ok(glyph_pos) = self
                .state
                .get_glyph_pos(c, font_family, font_size, font_weight)
            {
                let mut glyph_pos = glyph_pos.clone();
                glyph_pos.rect = glyph_pos.rect.with_origin((x as f64, y as f64));
                let new_x = x + glyph_pos.rect.width() as f32;
                if let Some(bounds) = bounds.as_ref() {
                    if x > bounds[1] as f32 {
                        return;
                    }
                    if new_x < bounds[0] as f32 {
                        x = new_x;
                        glyphs.push(glyph_pos);
                        continue;
                    }
                }

                if c == ' ' || c == '\n' {
                    x = new_x;
                    glyphs.push(glyph_pos);
                    continue;
                }

                let rect = &glyph_pos.rect;
                let cache_rect = &glyph_pos.cache_rect;
                let mut vertices = vec![
                    GpuVertex {
                        pos: [rect.x0 as f32, rect.y0 as f32],
                        tex: 1.0,
                        tex_pos: [cache_rect.x0 as f32, cache_rect.y0 as f32],
                        color,
                        ..Default::default()
                    },
                    GpuVertex {
                        pos: [rect.x0 as f32, rect.y1 as f32],
                        tex: 1.0,
                        tex_pos: [cache_rect.x0 as f32, cache_rect.y1 as f32],
                        color,
                        ..Default::default()
                    },
                    GpuVertex {
                        pos: [rect.x1 as f32, rect.y1 as f32],
                        tex: 1.0,
                        tex_pos: [cache_rect.x1 as f32, cache_rect.y1 as f32],
                        color,
                        ..Default::default()
                    },
                    GpuVertex {
                        pos: [rect.x1 as f32, rect.y0 as f32],
                        tex: 1.0,
                        tex_pos: [cache_rect.x1 as f32, cache_rect.y0 as f32],
                        color,
                        ..Default::default()
                    },
                ];
                let offset = geometry.vertices.len() as u32;
                let mut indices = vec![
                    offset + 0,
                    offset + 1,
                    offset + 2,
                    offset + 0,
                    offset + 2,
                    offset + 3,
                ];

                geometry.vertices.append(&mut vertices);
                geometry.indices.append(&mut indices);

                x = new_x;
                glyphs.push(glyph_pos);
            }
        }
    }

    pub(crate) fn draw_text(&self, ctx: &mut WgpuRenderContext, translate: [f32; 2]) {
        let offset = ctx.geometry.vertices.len() as u32;
        let geometry = self.geometry.borrow();
        let primivite_id = (ctx.primitives.len() - 1) as u32;
        let mut vertices = geometry
            .vertices
            .iter()
            .map(|v| {
                let mut v = v.clone();
                v.translate = translate;
                v.primitive_id = primivite_id;
                v
            })
            .collect();
        let mut indices = geometry.indices.iter().map(|i| *i + offset).collect();
        ctx.geometry.vertices.append(&mut vertices);
        ctx.geometry.indices.append(&mut indices);
    }

    pub fn cursor_line_for_text_position(&self, text_pos: usize) -> Line {
        let pos = self.hit_test_text_position(text_pos);
        let line_metric = self.line_metric(0).unwrap();
        let p0 = (pos.point.x, line_metric.y_offset);
        let p1 = (pos.point.x, line_metric.y_offset + line_metric.height);
        Line::new(p0, p1)
    }
}

pub struct WgpuTextLayoutBuilder {
    state: WgpuText,
    text: String,
    attrs: Attributes,
}

impl WgpuTextLayoutBuilder {
    pub(crate) fn new(text: impl TextStorage, state: WgpuText) -> Self {
        Self {
            text: text.as_str().to_string(),
            attrs: Default::default(),
            state,
        }
    }

    fn add(&mut self, attr: TextAttribute, range: Range<usize>) {
        self.attrs.add(range, attr);
    }

    pub fn build_with_bounds(self, bounds: [f64; 2]) -> WgpuTextLayout {
        let state = self.state.clone();
        let mut text_layout = WgpuTextLayout::new(self.text, state);
        text_layout.set_attrs(self.attrs);
        text_layout.rebuild(Some(bounds));
        text_layout
    }
}

impl Text for WgpuText {
    type TextLayoutBuilder = WgpuTextLayoutBuilder;
    type TextLayout = WgpuTextLayout;

    fn font_family(&mut self, family_name: &str) -> Option<FontFamily> {
        todo!()
    }

    fn load_font(&mut self, data: &[u8]) -> Result<piet::FontFamily, piet::Error> {
        todo!()
    }

    fn new_text_layout(&mut self, text: impl piet::TextStorage) -> Self::TextLayoutBuilder {
        let state = self.clone();
        Self::TextLayoutBuilder::new(text, state)
    }
}

impl TextLayoutBuilder for WgpuTextLayoutBuilder {
    type Out = WgpuTextLayout;

    fn max_width(self, width: f64) -> Self {
        self
    }

    fn alignment(self, alignment: piet::TextAlignment) -> Self {
        self
    }

    fn default_attribute(mut self, attribute: impl Into<piet::TextAttribute>) -> Self {
        let attribute = attribute.into();
        self.attrs.defaults.set(attribute);
        self
    }

    fn range_attribute(
        mut self,
        range: impl std::ops::RangeBounds<usize>,
        attribute: impl Into<piet::TextAttribute>,
    ) -> Self {
        let range = piet::util::resolve_range(range, self.text.len());
        let attribute = attribute.into();
        self.add(attribute, range);
        self
    }

    fn build(self) -> Result<Self::Out, piet::Error> {
        let state = self.state.clone();
        let mut text_layout = WgpuTextLayout::new(self.text, state);
        text_layout.set_attrs(self.attrs);
        text_layout.rebuild(None);
        Ok(text_layout)
    }
}

impl TextLayout for WgpuTextLayout {
    fn size(&self) -> Size {
        if self.glyphs.borrow().len() == 0 {
            let ref_glyph = self.ref_glyph.borrow();
            Size::new(0.0, ref_glyph.rect.height())
        } else {
            let glyphs = self.glyphs.borrow();

            let last_glyph = &glyphs[glyphs.len() - 1];
            let width = last_glyph.rect.x1;
            let height = last_glyph.rect.y1;
            Size::new(width as f64, height as f64)
        }
    }

    fn trailing_whitespace_width(&self) -> f64 {
        0.0
    }

    fn image_bounds(&self) -> piet::kurbo::Rect {
        Size::ZERO.to_rect()
    }

    fn text(&self) -> &str {
        &self.text
    }

    fn line_text(&self, line_number: usize) -> Option<&str> {
        Some(&self.text)
    }

    fn line_metric(&self, line_number: usize) -> Option<LineMetric> {
        let mut metric = LineMetric {
            start_offset: 0,
            end_offset: self.text.len(),
            trailing_whitespace: 0,
            baseline: 0.0,
            height: 0.0,
            y_offset: 0.0,
        };
        let glyph = &self.ref_glyph.borrow();
        metric.baseline = glyph.metric.ascent;
        metric.height = glyph.metric.ascent - glyph.metric.descent + glyph.metric.line_gap;
        Some(metric)
    }

    fn line_count(&self) -> usize {
        0
    }

    fn hit_test_point(&self, point: Point) -> HitTestPoint {
        HitTestPoint::default()
    }

    fn hit_test_text_position(&self, idx: usize) -> HitTestPosition {
        if self.glyphs.borrow().len() == 0 {
            return HitTestPosition::default();
        }

        let glyphs = self.glyphs.borrow();

        let cur_glyph = &glyphs[idx.min(glyphs.len() - 1)];
        let mut x = cur_glyph.rect.x0;
        if idx >= glyphs.len() {
            x = cur_glyph.rect.x1;
        }

        let mut pos = HitTestPosition::default();
        pos.point = Point::new(x as f64, 0.0);
        pos
    }
}

#[derive(Default)]
struct Attributes {
    defaults: piet::util::LayoutDefaults,
    color: Vec<Span<Color>>,
    font: Vec<Span<FontFamily>>,
    size: Vec<Span<f64>>,
    weight: Vec<Span<FontWeight>>,
    style: Option<Span<FontStyle>>,
}

/// during construction, `Span`s represent font attributes that have been applied
/// to ranges of the text; these are combined into coretext font objects as the
/// layout is built.
struct Span<T> {
    payload: T,
    range: Range<usize>,
}

impl<T> Span<T> {
    fn new(payload: T, range: Range<usize>) -> Self {
        Span { payload, range }
    }

    fn range_end(&self) -> usize {
        self.range.end
    }
}

impl Attributes {
    fn add(&mut self, range: Range<usize>, attr: TextAttribute) {
        match attr {
            TextAttribute::TextColor(color) => self.color.push(Span::new(color, range)),
            TextAttribute::Weight(weight) => self.weight.push(Span::new(weight, range)),
            _ => {}
        }
    }

    fn color(&self, index: usize) -> &Color {
        for r in &self.color {
            if r.range.contains(&index) {
                return &r.payload;
            }
        }
        &self.defaults.fg_color
    }

    fn size(&self, index: usize) -> f64 {
        for r in &self.size {
            if r.range.contains(&index) {
                return r.payload;
            }
        }
        self.defaults.font_size
    }

    fn italic(&self) -> bool {
        matches!(
            self.style
                .as_ref()
                .map(|t| t.payload)
                .unwrap_or(self.defaults.style),
            FontStyle::Italic
        )
    }

    fn font(&self, index: usize) -> FontFamily {
        for r in &self.font {
            if r.range.contains(&index) {
                return r.payload.clone();
            }
        }
        self.defaults.font.clone()
    }

    fn font_weight(&self, index: usize) -> FontWeight {
        for r in &self.weight {
            if r.range.contains(&index) {
                return r.payload;
            }
        }
        self.defaults.weight
    }
}
