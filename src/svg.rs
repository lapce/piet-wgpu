use std::{collections::HashMap, f64::NAN, str::FromStr};

use lyon::{
    lyon_tessellation::{
        BuffersBuilder, FillOptions, FillTessellator, FillVertex, StrokeOptions, StrokeTessellator,
        StrokeVertex, VertexBuffers,
    },
    math::{point, Point},
    path::PathEvent,
    tessellation,
};
use sha2::{Digest, Sha256};
use usvg::NodeExt;

use crate::pipeline::GpuVertex;

pub struct Svg {
    hash: Vec<u8>,
    pub(crate) tree: usvg::Tree,
}

impl FromStr for Svg {
    type Err = Box<dyn std::error::Error>;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut re_opt = usvg::Options {
            keep_named_groups: false,
            ..usvg::Options::default()
        };

        let mut hasher = Sha256::new();
        hasher.update(s);
        let hash = hasher.finalize().to_vec();

        re_opt.fontdb.load_system_fonts();

        match usvg::Tree::from_str(s, &re_opt) {
            Ok(tree) => Ok(Self { hash, tree }),
            Err(err) => Err(err.into()),
        }
    }
}

pub(crate) struct SvgStore {
    svgs: HashMap<Vec<u8>, VertexBuffers<GpuVertex, u32>>,
    fill_tess: FillTessellator,
    stroke_tess: StrokeTessellator,
}

impl SvgStore {
    pub(crate) fn new() -> Self {
        Self {
            svgs: HashMap::new(),
            fill_tess: FillTessellator::new(),
            stroke_tess: StrokeTessellator::new(),
        }
    }

    pub(crate) fn get_geometry(&mut self, svg: &Svg) -> &VertexBuffers<GpuVertex, u32> {
        if !self.svgs.contains_key(&svg.hash) {
            let geometry = self.new_geometry(svg);
            self.svgs.insert(svg.hash.clone(), geometry);
        }
        self.svgs.get(&svg.hash).unwrap()
    }

    fn new_geometry(&mut self, svg: &Svg) -> VertexBuffers<GpuVertex, u32> {
        let mut geometry: VertexBuffers<GpuVertex, u32> = VertexBuffers::new();
        for node in svg.tree.root().descendants() {
            if let usvg::NodeKind::Path(ref p) = *node.borrow() {
                let t = node.transform();
                let transform_1 = [t.a as f32, t.b as f32, t.c as f32, t.d as f32];
                let transform_2 = [t.e as f32, t.f as f32];
                if let Some(ref fill) = p.fill {
                    let color = match fill.paint {
                        usvg::Paint::Color(c) => c,
                        _ => FALLBACK_COLOR,
                    };
                    let color = [
                        color.red as f32 / 255.0,
                        color.green as f32 / 255.0,
                        color.blue as f32 / 255.0,
                        fill.opacity.value() as f32,
                    ];
                    self.fill_tess.tessellate(
                        convert_path(p),
                        &FillOptions::tolerance(0.01),
                        &mut BuffersBuilder::new(&mut geometry, |vertex: FillVertex| GpuVertex {
                            pos: vertex.position().to_array(),
                            color,
                            transform_1,
                            transform_2,
                            ..Default::default()
                        }),
                    );
                }

                if let Some(ref stroke) = p.stroke {
                    let (stroke_color, stroke_opacity, stroke_opts) = convert_stroke(stroke);
                    let color = [
                        stroke_color.red as f32 / 255.0,
                        stroke_color.green as f32 / 255.0,
                        stroke_color.blue as f32 / 255.0,
                        stroke_opacity.value() as f32,
                    ];
                    let _ = self.stroke_tess.tessellate(
                        convert_path(p),
                        &stroke_opts.with_tolerance(0.01),
                        &mut BuffersBuilder::new(&mut geometry, |vertex: StrokeVertex| GpuVertex {
                            pos: vertex.position().to_array(),
                            color,
                            transform_1,
                            transform_2,
                            ..Default::default()
                        }),
                    );
                }
            }
        }
        geometry
    }
}

pub const FALLBACK_COLOR: usvg::Color = usvg::Color {
    red: 0,
    green: 0,
    blue: 0,
};

pub struct PathConvIter<'a> {
    iter: std::slice::Iter<'a, usvg::PathSegment>,
    prev: Point,
    first: Point,
    needs_end: bool,
    deferred: Option<PathEvent>,
}

impl<'l> Iterator for PathConvIter<'l> {
    type Item = PathEvent;
    fn next(&mut self) -> Option<PathEvent> {
        if self.deferred.is_some() {
            return self.deferred.take();
        }

        let next = self.iter.next();
        match next {
            Some(usvg::PathSegment::MoveTo { x, y }) => {
                if self.needs_end {
                    let last = self.prev;
                    let first = self.first;
                    self.needs_end = false;
                    self.prev = point(*x as f32, *y as f32);
                    self.deferred = Some(PathEvent::Begin { at: self.prev });
                    self.first = self.prev;
                    Some(PathEvent::End {
                        last,
                        first,
                        close: false,
                    })
                } else {
                    self.first = point(*x as f32, *y as f32);
                    self.needs_end = true;
                    Some(PathEvent::Begin { at: self.first })
                }
            }
            Some(usvg::PathSegment::LineTo { x, y }) => {
                self.needs_end = true;
                let from = self.prev;
                self.prev = point(*x as f32, *y as f32);
                Some(PathEvent::Line {
                    from,
                    to: self.prev,
                })
            }
            Some(usvg::PathSegment::CurveTo {
                x1,
                y1,
                x2,
                y2,
                x,
                y,
            }) => {
                self.needs_end = true;
                let from = self.prev;
                self.prev = point(*x as f32, *y as f32);
                Some(PathEvent::Cubic {
                    from,
                    ctrl1: point(*x1 as f32, *y1 as f32),
                    ctrl2: point(*x2 as f32, *y2 as f32),
                    to: self.prev,
                })
            }
            Some(usvg::PathSegment::ClosePath) => {
                self.needs_end = false;
                self.prev = self.first;
                Some(PathEvent::End {
                    last: self.prev,
                    first: self.first,
                    close: true,
                })
            }
            None => {
                if self.needs_end {
                    self.needs_end = false;
                    let last = self.prev;
                    let first = self.first;
                    Some(PathEvent::End {
                        last,
                        first,
                        close: false,
                    })
                } else {
                    None
                }
            }
        }
    }
}

pub fn convert_path(p: &usvg::Path) -> PathConvIter {
    PathConvIter {
        iter: p.data.iter(),
        first: Point::new(0.0, 0.0),
        prev: Point::new(0.0, 0.0),
        deferred: None,
        needs_end: false,
    }
}

pub fn convert_stroke(s: &usvg::Stroke) -> (usvg::Color, usvg::Opacity, StrokeOptions) {
    let color = match s.paint {
        usvg::Paint::Color(c) => c,
        _ => FALLBACK_COLOR,
    };
    let linecap = match s.linecap {
        usvg::LineCap::Butt => tessellation::LineCap::Butt,
        usvg::LineCap::Square => tessellation::LineCap::Square,
        usvg::LineCap::Round => tessellation::LineCap::Round,
    };
    let linejoin = match s.linejoin {
        usvg::LineJoin::Miter => tessellation::LineJoin::Miter,
        usvg::LineJoin::Bevel => tessellation::LineJoin::Bevel,
        usvg::LineJoin::Round => tessellation::LineJoin::Round,
    };

    let opt = StrokeOptions::tolerance(0.01)
        .with_line_width(s.width.value() as f32)
        .with_line_cap(linecap)
        .with_line_join(linejoin);

    (color, s.opacity, opt)
}
