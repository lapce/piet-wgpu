use std::{cell::RefCell, collections::HashMap, ops::Range, rc::Rc};

use ab_glyph::{Font, FontVec, PxScale, ScaleFont};
use font_kit::source::SystemSource;
use lyon::lyon_tessellation::{
    BuffersBuilder, FillOptions, FillVertex, StrokeOptions, StrokeVertex,
};
use lyon::tessellation;
use piet::Color;
use piet::{
    kurbo::{Point, Size},
    FontFamily, FontStyle, FontWeight, HitTestPoint, HitTestPosition, LineMetric, Text,
    TextAttribute, TextLayout, TextLayoutBuilder, TextStorage,
};

use crate::pipeline::GpuVertex;
use crate::{context::WgpuRenderContext, font::FontSource};

#[derive(Clone)]
pub struct WgpuText {
    source: Rc<RefCell<SystemSource>>,
    fonts: Rc<RefCell<HashMap<FontFamily, Rc<ab_glyph::FontVec>>>>,
}

impl WgpuText {
    pub fn new() -> Self {
        Self {
            source: Rc::new(RefCell::new(SystemSource::new())),
            fonts: Rc::new(RefCell::new(HashMap::new())),
        }
    }

    fn get_font(&self, family: &FontFamily) -> Result<Rc<FontVec>, piet::Error> {
        if !self.fonts.borrow().contains_key(family) {
            let font = self.get_new_font(family)?;
            self.fonts
                .borrow_mut()
                .insert(family.clone(), Rc::new(font));
        }
        Ok(self.fonts.borrow().get(family).unwrap().clone())
    }

    fn get_new_font(&self, family: &FontFamily) -> Result<ab_glyph::FontVec, piet::Error> {
        let handle = self
            .source
            .borrow()
            .select_best_match(
                &[font_kit::family_name::FamilyName::Title(
                    family.name().to_string(),
                )],
                &font_kit::properties::Properties::new(),
            )
            .map_err(|e| piet::Error::NotSupported)?;
        let font = match handle {
            font_kit::handle::Handle::Path { path, font_index } => {
                let content =
                    std::fs::read_to_string(path).map_err(|e| piet::Error::NotSupported)?;
                let font = FontVec::try_from_vec(content.into_bytes())
                    .map_err(|e| piet::Error::NotSupported)?;
                font
            }
            font_kit::handle::Handle::Memory { bytes, font_index } => {
                let font =
                    FontVec::try_from_vec(bytes.to_vec()).map_err(|e| piet::Error::NotSupported)?;
                font
            }
        };
        Ok(font)
    }
}

#[derive(Clone)]
pub struct WgpuTextLayout {
    state: WgpuText,
    text: String,
    attrs: Rc<Attributes>,
}

impl WgpuTextLayout {
    pub fn new(text: String, state: WgpuText) -> Self {
        Self {
            text,
            state,
            attrs: Rc::new(Attributes::default()),
        }
    }

    fn set_attrs(&mut self, attrs: Attributes) {
        self.attrs = Rc::new(attrs);
    }

    pub(crate) fn draw_text(&self, ctx: &mut WgpuRenderContext, pos: Point) {
        let font = self
            .state
            .get_font(&FontFamily::new_unchecked("Cascadia Code".to_string()))
            .unwrap();
        let units_per_em = font.units_per_em().unwrap();
        let font_size = 13.0;
        let font_scale = font_size * (font.height_unscaled() / units_per_em);
        let scaled_font = font.as_scaled(font_size * (font.height_unscaled() / units_per_em));
        let scale = font_scale / font.height_unscaled();
        let scale = [scale, -scale];
        let affine = ctx.cur_transform.as_coeffs();
        let translate = [affine[4] as f32, affine[5] as f32];
        let mut x = 0.0;
        for (index, c) in self.text.chars().enumerate() {
            let id = font.glyph_id(c);
            let glyph = scaled_font.scaled_glyph(c);
            if let Some(outline) = font.outline(id) {
                let mut builder = lyon::path::Path::builder();
                let mut last = None;
                for curve in &outline.curves {
                    let start = match curve {
                        ab_glyph::OutlineCurve::Line(p1, _) => p1,
                        ab_glyph::OutlineCurve::Quad(p1, _, _) => p1,
                        ab_glyph::OutlineCurve::Cubic(p1, _, _, _) => p1,
                    };
                    if let Some(p) = last.as_ref() {
                        if p != start {
                            println!("differnt start {:?} {:?}", p, start);
                            builder.end(false);
                            builder.begin(lyon::geom::point(start.x, start.y));
                        }
                    } else {
                        builder.begin(lyon::geom::point(start.x, start.y));
                    }

                    let end = match curve {
                        ab_glyph::OutlineCurve::Line(p1, p2) => {
                            builder.line_to(lyon::geom::point(p2.x, p2.y));
                            p2
                        }
                        ab_glyph::OutlineCurve::Quad(p1, p2, p3) => {
                            builder.quadratic_bezier_to(
                                lyon::geom::point(p2.x, p2.y),
                                lyon::geom::point(p3.x, p3.y),
                            );
                            p3
                        }
                        ab_glyph::OutlineCurve::Cubic(p1, p2, p3, p4) => {
                            builder.cubic_bezier_to(
                                lyon::geom::point(p2.x, p2.y),
                                lyon::geom::point(p3.x, p3.y),
                                lyon::geom::point(p4.x, p4.y),
                            );
                            p4
                        }
                    };
                    last = Some(end.clone());
                }
                builder.close();
                let path = builder.build();
                let translate = [
                    translate[0] + pos.x as f32 + x,
                    translate[1] + pos.y as f32 + font_size,
                ];
                let color = self.attrs.color(index);
                let color = color.as_rgba();
                let color = [
                    color.0 as f32,
                    color.1 as f32,
                    color.2 as f32,
                    color.3 as f32,
                ];
                ctx.fill_tess.tessellate_path(
                    &path,
                    &FillOptions::tolerance(1.0).with_fill_rule(tessellation::FillRule::NonZero),
                    &mut BuffersBuilder::new(&mut ctx.geometry, |vertex: FillVertex| GpuVertex {
                        pos: vertex.position().to_array(),
                        translate,
                        scale,
                        color,
                        ..Default::default()
                    }),
                );
            }
            x += scaled_font.h_advance(id);
        }
    }
}

pub struct WgpuTextLayoutBuilder {
    text: String,
    state: WgpuText,
    attrs: Attributes,
}

impl WgpuTextLayoutBuilder {
    pub(crate) fn new(text: impl TextStorage, state: WgpuText) -> Self {
        Self {
            text: text.as_str().to_string(),
            state,
            attrs: Default::default(),
        }
    }

    fn add(&mut self, attr: TextAttribute, range: Range<usize>) {
        self.attrs.add(range, attr);
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
        Self::TextLayoutBuilder::new(text, self.clone())
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
        let mut text_layout = WgpuTextLayout::new(self.text, self.state);
        text_layout.set_attrs(self.attrs);
        Ok(text_layout)
    }
}

impl TextLayout for WgpuTextLayout {
    fn size(&self) -> Size {
        Size::ZERO
    }

    fn trailing_whitespace_width(&self) -> f64 {
        0.0
    }

    fn image_bounds(&self) -> piet::kurbo::Rect {
        Size::ZERO.to_rect()
    }

    fn text(&self) -> &str {
        ""
    }

    fn line_text(&self, line_number: usize) -> Option<&str> {
        Some("")
    }

    fn line_metric(&self, line_number: usize) -> Option<LineMetric> {
        None
    }

    fn line_count(&self) -> usize {
        0
    }

    fn hit_test_point(&self, point: Point) -> HitTestPoint {
        HitTestPoint::default()
    }

    fn hit_test_text_position(&self, idx: usize) -> HitTestPosition {
        HitTestPosition::default()
    }
}

#[derive(Default)]
struct Attributes {
    defaults: piet::util::LayoutDefaults,
    color: Vec<Span<Color>>,
    font: Option<Span<FontFamily>>,
    size: Option<Span<f64>>,
    weight: Option<Span<FontWeight>>,
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

    fn size(&self) -> f64 {
        self.size
            .as_ref()
            .map(|s| s.payload)
            .unwrap_or(self.defaults.font_size)
    }

    fn weight(&self) -> FontWeight {
        self.weight
            .as_ref()
            .map(|w| w.payload)
            .unwrap_or(self.defaults.weight)
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

    fn font(&self) -> &FontFamily {
        self.font
            .as_ref()
            .map(|t| &t.payload)
            .unwrap_or_else(|| &self.defaults.font)
    }

    fn next_span_end(&self, max: usize) -> usize {
        self.font
            .as_ref()
            .map(Span::range_end)
            .unwrap_or(max)
            .min(self.size.as_ref().map(Span::range_end).unwrap_or(max))
            .min(self.weight.as_ref().map(Span::range_end).unwrap_or(max))
            .min(self.style.as_ref().map(Span::range_end).unwrap_or(max))
            .min(max)
    }

    // invariant: `last_pos` is the end of at least one span.
    fn clear_up_to(&mut self, last_pos: usize) {
        if self.font.as_ref().map(Span::range_end) == Some(last_pos) {
            self.font = None;
        }
        if self.weight.as_ref().map(Span::range_end) == Some(last_pos) {
            self.weight = None;
        }
        if self.style.as_ref().map(Span::range_end) == Some(last_pos) {
            self.style = None;
        }
        if self.size.as_ref().map(Span::range_end) == Some(last_pos) {
            self.size = None;
        }
    }
}
