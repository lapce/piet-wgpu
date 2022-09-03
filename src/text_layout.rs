use std::{cell::RefCell, rc::Rc};

use parley::context::{RangedBuilder, RcLayoutContext, TextSource};
use parley::layout::Cursor;
use parley::style::{self, Brush, StyleProperty};
use parley::{layout, FontContext, Layout};
use piet::kurbo::Rect;
use piet::{
    kurbo::{Point, Size},
    FontFamily, HitTestPoint, HitTestPosition, LineMetric, Text, TextAttribute, TextLayout,
    TextLayoutBuilder, TextStorage,
};
use piet::{Color, FontFamilyInner, TextAlignment};
use swash::scale::image::Content;

use crate::context::{Tex, WgpuRenderContext};
use crate::text::Cache;

const DEFAULT_FONT: &[u8] = include_bytes!("../fonts/CascadiaCode.ttf");

impl Brush for ParleyBrush {}

#[derive(Clone, PartialEq, Debug)]
pub struct ParleyBrush(pub Color);

impl Default for ParleyBrush {
    fn default() -> Self {
        Self(Color::grey(0.0))
    }
}

#[derive(Clone)]
pub struct ParleyTextStorage(pub Rc<dyn TextStorage>);

impl TextSource for ParleyTextStorage {
    fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

#[derive(Clone)]
pub struct WgpuText {
    fcx: Rc<RefCell<FontContext>>,
    lcx: RcLayoutContext<ParleyBrush>,

    pub(crate) cache: Rc<RefCell<Cache>>,
}

impl WgpuText {
    pub(crate) fn new(gl: &glow::Context) -> Self {
        let mut t = Self {
            cache: Rc::new(RefCell::new(Cache::new(gl, 2000, 2000))),
            fcx: Rc::new(RefCell::new(FontContext::new())),
            lcx: RcLayoutContext::new(),
        };
        t.load_font(DEFAULT_FONT);
        t
    }
}

#[derive(Clone)]
pub struct WgpuTextLayout {
    pub text: ParleyTextStorage,
    pub layout: Layout<ParleyBrush>,
}

impl WgpuTextLayout {
    pub(crate) fn draw_text(&self, ctx: &mut WgpuRenderContext, translate: [f32; 2]) {
        let mut instances = Vec::new();
        let mut color_instances = Vec::new();
        let mut cache = ctx.renderer.text.cache.borrow_mut();
        let scale = cache.scale as f32;
        let depth = ctx.depth as f32;
        let affine = ctx.cur_transform.as_coeffs();
        let clip = ctx.current_clip();
        let clip = [
            clip[0] * scale,
            clip[1] * scale,
            clip[2] * scale,
            clip[3] * scale,
        ];
        let translate = [
            (translate[0] + affine[4] as f32),
            (translate[1] + affine[5] as f32).round(),
        ];

        for line in self.layout.lines() {
            for run in line.glyph_runs() {
                let font = run.run().font();
                let font_size = run.run().font_size();
                for glyph in run.positioned_glyphs() {
                    if glyph.id == 0 {
                        continue;
                    }
                    let x = glyph.x + translate[0];
                    if let Ok(pos) = cache.get_glyph(&glyph, x, font, font_size, &ctx.renderer.gl) {
                        let color = &self.layout.styles()[glyph.style_index()].brush.0.as_rgba();
                        let x = (x * scale + 0.125).floor();
                        let y = ((glyph.y + translate[1]) * scale - pos.rect.y0 as f32).round();
                        let instance = Tex {
                            rect: [
                                pos.rect.x0 as f32 + x,
                                y,
                                pos.rect.x1 as f32 + x,
                                y + pos.rect.height() as f32,
                            ],
                            tex_rect: [
                                pos.cache_rect.x0 as f32,
                                pos.cache_rect.y0 as f32,
                                pos.cache_rect.x1 as f32,
                                pos.cache_rect.y1 as f32,
                            ],
                            color: if let Content::Color = pos.content {
                                [0.0, 0.0, 0.0, 0.0]
                            } else {
                                [
                                    color.0 as f32,
                                    color.1 as f32,
                                    color.2 as f32,
                                    color.3 as f32,
                                ]
                            },
                            depth,
                            clip,
                        };
                        if let Content::Color = pos.content {
                            color_instances.push(instance);
                        } else {
                            instances.push(instance);
                        }
                    }
                }
            }
        }

        ctx.layer.add_text(instances, ctx.alpha_depth);
        ctx.layer.add_color_text(color_instances, ctx.alpha_depth);
    }

    pub fn cap_center(&self) -> f64 {
        if let Some(line) = self.layout.get(0) {
            let metrics = line.metrics();
            metrics.cap_height as f64 / 2.0 + (metrics.ascent - metrics.cap_height) as f64
        } else {
            0.0
        }
    }

    pub fn y_offset(&self, height: f64) -> f64 {
        if let Some(line) = self.layout.get(0) {
            let metrics = line.metrics();
            (height - metrics.cap_height as f64) / 2.0
                - (metrics.ascent - metrics.cap_height) as f64
        } else {
            0.0
        }
    }
}

pub struct WgpuTextLayoutBuilder {
    text: ParleyTextStorage,
    builder: RangedBuilder<'static, ParleyBrush, ParleyTextStorage>,
    max_width: f64,
    alignment: layout::Alignment,
}

impl WgpuTextLayoutBuilder {
    pub fn set_tab_width(mut self, tab_width: f64) -> Self {
        self.builder
            .push_default(&style::StyleProperty::TabWidth(tab_width as f32));
        self
    }

    pub fn set_line_height(mut self, line_height: f64) -> Self {
        self.builder
            .push_default(&style::StyleProperty::LineHeight(line_height as f32));
        self
    }
}

impl Text for WgpuText {
    type TextLayoutBuilder = WgpuTextLayoutBuilder;
    type TextLayout = WgpuTextLayout;

    fn font_family(&mut self, family_name: &str) -> Option<FontFamily> {
        if self.fcx.borrow().has_family(family_name) {
            Some(FontFamily::new_unchecked(family_name))
        } else {
            None
        }
    }

    fn load_font(&mut self, data: &[u8]) -> Result<piet::FontFamily, piet::Error> {
        if let Some(family_name) = self.fcx.borrow_mut().register_fonts(data.into()) {
            Ok(FontFamily::new_unchecked(family_name))
        } else {
            Err(piet::Error::FontLoadingFailed)
        }
    }

    fn new_text_layout(&mut self, text: impl piet::TextStorage) -> Self::TextLayoutBuilder {
        let text = ParleyTextStorage(Rc::new(text));
        let builder = self.lcx.ranged_builder(self.fcx.clone(), text.clone(), 1.0);
        let builder = WgpuTextLayoutBuilder {
            builder,
            text,
            max_width: f64::INFINITY,
            alignment: layout::Alignment::Start,
        };
        let defaults = piet::util::LayoutDefaults::default();
        builder
            .default_attribute(TextAttribute::FontFamily(defaults.font))
            .default_attribute(TextAttribute::FontSize(defaults.font_size))
            .default_attribute(TextAttribute::TextColor(defaults.fg_color))
    }
}

impl TextLayoutBuilder for WgpuTextLayoutBuilder {
    type Out = WgpuTextLayout;

    fn max_width(mut self, width: f64) -> Self {
        self.max_width = width;
        self
    }

    fn alignment(mut self, alignment: TextAlignment) -> Self {
        use layout::Alignment;
        self.alignment = match alignment {
            TextAlignment::Start => Alignment::Start,
            TextAlignment::Center => Alignment::Middle,
            TextAlignment::End => Alignment::End,
            TextAlignment::Justified => Alignment::Justified,
        };
        self
    }

    fn default_attribute(mut self, attribute: impl Into<piet::TextAttribute>) -> Self {
        self.builder.push_default(&convert_attr(&attribute.into()));
        self
    }

    fn range_attribute(
        mut self,
        range: impl std::ops::RangeBounds<usize>,
        attribute: impl Into<piet::TextAttribute>,
    ) -> Self {
        self.builder.push(&convert_attr(&attribute.into()), range);
        self
    }

    fn build(mut self) -> Result<Self::Out, piet::Error> {
        let mut layout = self.builder.build();
        layout.break_all_lines(Some(self.max_width as f32), self.alignment);
        Ok(WgpuTextLayout {
            text: self.text,
            layout,
        })
    }
}

impl TextLayout for WgpuTextLayout {
    fn size(&self) -> Size {
        self.image_bounds().size()
    }

    fn image_bounds(&self) -> Rect {
        Rect::new(0., 0., self.layout.width() as _, self.layout.height() as _)
    }

    fn trailing_whitespace_width(&self) -> f64 {
        0.0
    }

    fn text(&self) -> &str {
        self.text.0.as_str()
    }

    fn line_text(&self, line_number: usize) -> Option<&str> {
        let range = self.layout.get(line_number)?.text_range();
        self.text().get(range)
    }

    fn line_metric(&self, line_number: usize) -> Option<LineMetric> {
        let line = self.layout.get(line_number)?;
        let range = line.text_range();
        let metrics = line.metrics();
        let y_offset = metrics.cap_height as f64;
        let baseline = metrics.ascent as f64;
        let trailing_whitespace = metrics.trailing_whitespace as usize;
        Some(LineMetric {
            start_offset: range.start,
            end_offset: range.end,
            trailing_whitespace,
            baseline,
            height: metrics.size() as f64,
            y_offset,
        })
    }

    fn line_count(&self) -> usize {
        self.layout.len()
    }

    fn hit_test_point(&self, point: Point) -> HitTestPoint {
        let cursor = Cursor::from_point(&self.layout, point.x as f32, point.y as f32);
        let mut result = HitTestPoint::default();
        let range = cursor.text_range();
        // FIXME: this is horribly broken for BiDi text
        if cursor.is_trailing() {
            result.idx = range.end;
        } else {
            result.idx = range.start;
        }
        result.is_inside = cursor.is_inside();
        result
    }

    fn hit_test_text_position(&self, idx: usize) -> HitTestPosition {
        let cursor = Cursor::from_position(&self.layout, idx, true);
        let mut result = HitTestPosition::default();
        result.point = Point::new(cursor.offset() as f64, cursor.baseline() as f64);
        result.line = cursor.path().line_index;
        result
    }
}

fn convert_attr(attr: &TextAttribute) -> style::StyleProperty<ParleyBrush> {
    use style::FontStyle as Style;
    use style::FontWeight as Weight;
    use style::GenericFamily;
    use style::StyleProperty::*;
    match attr {
        TextAttribute::FontFamily(family) => {
            use style::FontFamily::*;
            FontStack(style::FontStack::Single(match family.inner() {
                FontFamilyInner::Named(name) => Named(&*name),
                FontFamilyInner::SansSerif => Generic(GenericFamily::SansSerif),
                FontFamilyInner::Serif => Generic(GenericFamily::Serif),
                FontFamilyInner::SystemUi => Generic(GenericFamily::SystemUi),
                FontFamilyInner::Monospace => Generic(GenericFamily::Monospace),
                _ => Named(""),
            }))
        }
        TextAttribute::FontSize(size) => FontSize(*size as f32),
        TextAttribute::Weight(weight) => FontWeight(Weight(weight.to_raw())),
        TextAttribute::Style(style) => FontStyle(match style {
            piet::FontStyle::Regular => Style::Normal,
            piet::FontStyle::Italic => Style::Italic,
        }),
        TextAttribute::TextColor(color) => Brush(ParleyBrush(color.clone())),
        TextAttribute::Underline(enable) => Underline(*enable),
        TextAttribute::Strikethrough(enable) => Strikethrough(*enable),
    }
}
