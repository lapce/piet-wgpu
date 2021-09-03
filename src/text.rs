use piet::{
    kurbo::{Point, Size},
    HitTestPoint, HitTestPosition, LineMetric, Text, TextLayout, TextLayoutBuilder,
};

use crate::context::WgpuRenderContext;

#[derive(Clone)]
pub struct WgpuText {}

impl WgpuText {
    pub fn new() -> Self {
        Self {}
    }
}

#[derive(Clone)]
pub struct WgpuTextLayout {}

impl WgpuTextLayout {
    pub fn new() -> Self {
        Self {}
    }

    pub fn draw_text(&self, ctx: &mut WgpuRenderContext, pos: Point) {}
}

#[derive(Clone)]
pub struct WgpuTextLayoutBuilder {}

impl WgpuTextLayoutBuilder {
    pub fn new() -> Self {
        Self {}
    }
}

impl Text for WgpuText {
    type TextLayoutBuilder = WgpuTextLayoutBuilder;
    type TextLayout = WgpuTextLayout;

    fn font_family(&mut self, family_name: &str) -> Option<piet::FontFamily> {
        todo!()
    }

    fn load_font(&mut self, data: &[u8]) -> Result<piet::FontFamily, piet::Error> {
        todo!()
    }

    fn new_text_layout(&mut self, text: impl piet::TextStorage) -> Self::TextLayoutBuilder {
        Self::TextLayoutBuilder::new()
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

    fn default_attribute(self, attribute: impl Into<piet::TextAttribute>) -> Self {
        self
    }

    fn range_attribute(
        self,
        range: impl std::ops::RangeBounds<usize>,
        attribute: impl Into<piet::TextAttribute>,
    ) -> Self {
        self
    }

    fn build(self) -> Result<Self::Out, piet::Error> {
        Ok(WgpuTextLayout::new())
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
        Some(LineMetric::default())
    }

    fn line_count(&self) -> usize {
        0
    }

    fn hit_test_point(&self, point: piet::kurbo::Point) -> HitTestPoint {
        HitTestPoint::default()
    }

    fn hit_test_text_position(&self, idx: usize) -> HitTestPosition {
        HitTestPosition::default()
    }
}
