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
use unicode_width::UnicodeWidthChar;

use crate::context::{self, format_color, WgpuRenderContext};
use crate::text::{Cache, GlyphPosInfo};

#[derive(Clone)]
pub struct WgpuText {
    gl: Rc<glow::Context>,
    pub(crate) cache: Rc<RefCell<Cache>>,
}

impl WgpuText {
    pub(crate) fn new(gl: Rc<glow::Context>) -> Self {
        Self {
            cache: Rc::new(RefCell::new(Cache::new(&gl, 2000, 2000))),
            gl,
        }
    }

    pub(crate) fn get_glyph_pos(
        &self,
        c: char,
        font_family: FontFamily,
        font_size: f32,
        font_weight: FontWeight,
        x: f32,
    ) -> Result<GlyphPosInfo, piet::Error> {
        let mut cache = self.cache.borrow_mut();
        cache
            .get_glyph_pos(c, font_family, font_size, font_weight, x, &self.gl)
            .map(|p| p.clone())
    }
}

#[derive(Clone)]
pub struct WgpuTextLayout {
    state: WgpuText,
    text: Rc<String>,
    width: f64,
    attrs: Rc<Attributes>,
    ref_glyph: Rc<RefCell<GlyphPosInfo>>,
    glyphs: Rc<RefCell<Vec<GlyphPosInfo>>>,
    instances: Rc<RefCell<Vec<context::Tex>>>,
}

impl WgpuTextLayout {
    pub fn new(text: String, state: WgpuText) -> Self {
        Self {
            state,
            text: Rc::new(text),
            width: f64::MAX,
            attrs: Rc::new(Attributes::default()),
            glyphs: Rc::new(RefCell::new(Vec::new())),
            ref_glyph: Rc::new(RefCell::new(GlyphPosInfo::default())),
            instances: Rc::new(RefCell::new(Vec::new())),
        }
    }

    fn set_width(&mut self, width: f64) {
        self.width = width;
    }

    fn set_attrs(&mut self, attrs: Attributes) {
        self.attrs = Rc::new(attrs);
    }

    pub fn set_color(&self, color: &Color) {
        let color = format_color(color);
        for v in self.instances.borrow_mut().iter_mut() {
            v.color = color;
        }
    }

    pub(crate) fn rebuild(&self, is_mono: bool, tab_width: usize, bounds: Option<[f64; 2]>) {
        let font_family = self.attrs.defaults.font.clone();
        let font_size = self.attrs.defaults.font_size;
        let font_weight = self.attrs.defaults.weight;
        if let Ok(glyph_pos) =
            self.state
                .get_glyph_pos('W', font_family, font_size as f32, font_weight, 0.0)
        {
            *self.ref_glyph.borrow_mut() = glyph_pos;
        }

        let mono_width = self.ref_glyph.borrow().rect.width();

        let len = self.text.chars().count();

        let mut glyphs = self.glyphs.borrow_mut();
        glyphs.clear();
        glyphs.reserve(len);
        let mut instances = self.instances.borrow_mut();
        instances.clear();
        instances.reserve(len);

        let scale = self.state.cache.borrow().scale as f32;
        let mut x = 0.0;
        let mut y = 0.0;
        let mut max_height = 0.0;
        let mut index = 0;
        let mut mono_char_widths = 0;
        for c in self.text.chars() {
            let font_family = self.attrs.font(index);
            let font_size = self.attrs.size(index) as f32;
            let font_weight = self.attrs.font_weight(index);
            let color = self.attrs.color(index);
            index += c.len_utf8();

            let default_width = if is_mono {
                let char_width = if c == '\t' {
                    tab_width - mono_char_widths % tab_width
                } else {
                    UnicodeWidthChar::width(c).unwrap_or(1)
                };
                mono_char_widths += char_width;
                char_width as f32 * mono_width as f32
            } else {
                let char_width = if c == '\t' {
                    tab_width
                } else {
                    UnicodeWidthChar::width(c).unwrap_or(1)
                };
                char_width as f32 * mono_width as f32
            };

            let mut glyph_pos = self
                .state
                .get_glyph_pos(c, font_family, font_size, font_weight, x)
                .unwrap_or_else(|_| GlyphPosInfo::empty(default_width as f64));
            let width = if is_mono {
                glyph_pos.width = default_width as f64;
                default_width
            } else {
                glyph_pos.rect.width() as f32
            };
            if (x + width) as f64 > self.width {
                x = 0.0;
                y += max_height;
            }

            let corrected_x = (((x * scale + 0.125).floor()) / scale) as f64;
            glyph_pos.rect = glyph_pos.rect.with_origin((corrected_x, y as f64));

            let new_x = x + width;

            let height = glyph_pos.rect.height() as f32;
            if height > max_height {
                max_height = height;
            }

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

            if c == ' ' || c == '\n' || c == '\t' {
                x = new_x;
                glyphs.push(glyph_pos);
                continue;
            }

            let rect = &glyph_pos.rect;
            let cache_rect = &glyph_pos.cache_rect;

            let color = format_color(color);
            instances.push(context::Tex {
                rect: [
                    rect.x0 as f32,
                    rect.y0 as f32,
                    rect.x1 as f32,
                    rect.y1 as f32,
                ],
                tex_rect: [
                    cache_rect.x0 as f32,
                    cache_rect.y0 as f32,
                    cache_rect.x1 as f32,
                    cache_rect.y1 as f32,
                ],
                color,
                depth: 0.0,
                clip: [0.0, 0.0, 0.0, 0.0],
            });

            x = new_x;
            glyphs.push(glyph_pos);
        }
    }

    pub(crate) fn draw_text(&self, ctx: &mut WgpuRenderContext, translate: [f32; 2]) {
        let instances = self.instances.borrow();
        if instances.is_empty() {
            return;
        }
        let depth = ctx.depth as f32;
        let affine = ctx.cur_transform.as_coeffs();
        let clip = ctx.current_clip();
        let translate = [
            (translate[0] + affine[4] as f32).round(),
            (translate[1] + affine[5] as f32).round(),
        ];

        let instances = instances
            .iter()
            .map(|i| context::Tex {
                rect: [
                    i.rect[0] + translate[0],
                    i.rect[1] + translate[1],
                    i.rect[2] + translate[0],
                    i.rect[3] + translate[1],
                ],
                tex_rect: i.tex_rect,
                color: i.color,
                depth,
                clip,
            })
            .collect();
        ctx.layer.add_text(instances);
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
    width: f64,
    state: WgpuText,
    text: String,
    attrs: Attributes,
}

impl WgpuTextLayoutBuilder {
    pub(crate) fn new(text: impl TextStorage, state: WgpuText) -> Self {
        Self {
            width: f64::MAX,
            text: text.as_str().to_string(),
            attrs: Default::default(),
            state,
        }
    }

    fn add(&mut self, attr: TextAttribute, range: Range<usize>) {
        self.attrs.add(range, attr);
    }

    pub fn build_with_info(
        self,
        is_mono: bool,
        tab_width: usize,
        bounds: Option<[f64; 2]>,
    ) -> WgpuTextLayout {
        let state = self.state.clone();
        let mut text_layout = WgpuTextLayout::new(self.text, state);
        text_layout.set_attrs(self.attrs);
        text_layout.set_width(self.width);
        text_layout.rebuild(is_mono, tab_width, bounds);
        text_layout
    }

    pub fn build_with_bounds(self, bounds: [f64; 2]) -> WgpuTextLayout {
        let state = self.state.clone();
        let mut text_layout = WgpuTextLayout::new(self.text, state);
        text_layout.set_attrs(self.attrs);
        text_layout.set_width(self.width);
        text_layout.rebuild(false, 8, Some(bounds));
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

    fn max_width(mut self, width: f64) -> Self {
        self.width = width;
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
        text_layout.set_width(self.width);
        text_layout.rebuild(false, 8, None);
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
            let width = last_glyph.rect.x0 + last_glyph.width;
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
        let mut hit = HitTestPoint::default();
        if self.glyphs.borrow().len() == 0 {
            return hit;
        }

        let glyphs = self.glyphs.borrow();
        let mut index = None;
        for (i, glyph) in glyphs.iter().enumerate() {
            if point.x < glyph.rect.x0 + glyph.rect.width() / 2.0 {
                index = Some(i);
                break;
            }
            if point.x < glyph.rect.x1 {
                index = Some(i + 1);
                break;
            }
        }
        hit.idx = index.unwrap_or(glyphs.len());
        hit.is_inside = index.is_some();
        hit
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
