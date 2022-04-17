use std::{collections::HashMap, str::FromStr};

use glow::HasContext;
use linked_hash_map::LinkedHashMap;
use piet::kurbo::{Point, Rect, Size};
use sha2::{Digest, Sha256};

#[derive(Clone)]
pub struct Svg {
    hash: Vec<u8>,
    pub(crate) tree: usvg::Tree,
}

unsafe impl Send for Svg {}
unsafe impl Sync for Svg {}

impl FromStr for Svg {
    type Err = Box<dyn std::error::Error>;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut hasher = Sha256::new();
        hasher.update(s);
        let hash = hasher.finalize().to_vec();

        match usvg::Tree::from_str(s, &usvg::Options::default().to_ref()) {
            Ok(tree) => Ok(Self { hash, tree }),
            Err(err) => Err(err.into()),
        }
    }
}

pub(crate) struct SvgStore {
    pub(crate) cache: SvgCache,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, Default)]
pub(crate) struct SvgInfo {
    hash: Vec<u8>,
    width: u32,
    height: u32,
}

#[derive(Default, Clone)]
pub(crate) struct SvgPosInfo {
    pub(crate) rect: Rect,
    pub(crate) cache_rect: Rect,
}

struct SvgRow {
    y: u32,
    height: u32,
    width: u32,
    svgs: Vec<SvgPosInfo>,
}

pub struct SvgCache {
    pub gl_texture: glow::Texture,
    width: u32,
    height: u32,

    rows: LinkedHashMap<usize, SvgRow>,
    svgs: HashMap<SvgInfo, (usize, usize)>,
    pub(crate) scale: f64,
}

impl SvgCache {
    pub fn new(gl: &glow::Context) -> Self {
        let width = 2000;
        let height = 2000;
        let gl_texture = unsafe {
            let handle = gl.create_texture().expect("Create glyph cache texture");

            gl.bind_texture(glow::TEXTURE_2D, Some(handle));

            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_S,
                glow::CLAMP_TO_EDGE as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_T,
                glow::CLAMP_TO_EDGE as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MIN_FILTER,
                glow::NEAREST as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MAG_FILTER,
                glow::NEAREST as i32,
            );

            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA as i32,
                width as i32,
                height as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                None,
            );
            gl.bind_texture(glow::TEXTURE_2D, None);

            handle
        };

        Self {
            gl_texture,
            width,
            height,
            rows: LinkedHashMap::new(),
            svgs: HashMap::new(),
            scale: 1.0,
        }
    }

    pub fn update(
        &mut self,
        gl: &glow::Context,
        offset: [u32; 2],
        data: &[u8],
        width: u32,
        height: u32,
    ) {
        unsafe {
            gl.bind_texture(glow::TEXTURE_2D, Some(self.gl_texture));

            gl.tex_sub_image_2d(
                glow::TEXTURE_2D,
                0,
                offset[0] as i32,
                offset[1] as i32,
                width as i32,
                height as i32,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(data),
            );

            gl.bind_texture(glow::TEXTURE_2D, None);
        }
    }

    pub(crate) fn get_svg(
        &mut self,
        gl: &glow::Context,
        svg: &Svg,
        [width, height]: [f32; 2],
    ) -> Result<&SvgPosInfo, piet::Error> {
        let (width, height) = (width.ceil() as u32, height.ceil() as u32);
        let svg_info = SvgInfo {
            hash: svg.hash.clone(),
            width,
            height,
        };

        if let Some((row, index)) = self.svgs.get(&svg_info) {
            let row = self.rows.get(row).unwrap();
            return Ok(&row.svgs[*index]);
        }

        let transform = tiny_skia::Transform::identity();
        let mut img = tiny_skia::Pixmap::new(width, height).ok_or(piet::Error::InvalidInput)?;

        let _ = resvg::render(
            &svg.tree,
            if width > height {
                usvg::FitTo::Width(width)
            } else {
                usvg::FitTo::Height(height)
            },
            transform,
            img.as_mut(),
        )
        .ok_or(piet::Error::InvalidInput)?;

        let scale = self.scale;
        let glyph_rect = Size::new(width as f64, height as f64).to_rect();
        let mut offset = [0, 0];
        let mut inserted = false;
        for (row_number, row) in self.rows.iter_mut().rev() {
            if row.height == height && self.width - row.width > width {
                let origin = Point::new(row.width as f64, row.y as f64);
                let glyph_pos =
                    svg_rect_to_pos(glyph_rect, origin, scale, [self.width, self.height]);

                row.svgs.push(glyph_pos);
                offset[0] = row.width;
                offset[1] = row.y;
                row.width += width;
                self.svgs
                    .insert(svg_info.clone(), (*row_number, row.svgs.len() - 1));
                inserted = true;
                break;
            }
        }

        if !inserted {
            let mut y = 0;
            if !self.rows.is_empty() {
                let last_row = self.rows.get(&(self.rows.len() - 1)).unwrap();
                y = last_row.y + last_row.height;
            }
            if self.height < y + height {
                return Err(piet::Error::MissingFont);
            }

            let origin = Point::new(0.0, y as f64);
            let svg_pos = svg_rect_to_pos(glyph_rect, origin, scale, [self.width, self.height]);

            offset[0] = 0;
            offset[1] = y;
            let new_row = self.rows.len();
            let svgs = vec![svg_pos];
            let row = SvgRow {
                y,
                height,
                width,
                svgs,
            };

            self.rows.insert(new_row, row);
            self.svgs.insert(svg_info.clone(), (new_row, 0));
        }

        let data = img.take();
        self.update(gl, offset, &data, width, height);

        let (row, index) = self.svgs.get(&svg_info).unwrap();
        let row = self.rows.get(row).unwrap();
        Ok(&row.svgs[*index])
    }
}

impl SvgStore {
    pub(crate) fn new(gl: &glow::Context) -> Self {
        Self {
            cache: SvgCache::new(gl),
        }
    }
}

pub const FALLBACK_COLOR: usvg::Color = usvg::Color {
    red: 0,
    green: 0,
    blue: 0,
};

fn svg_rect_to_pos(glyph_rect: Rect, origin: Point, scale: f64, size: [u32; 2]) -> SvgPosInfo {
    let mut cache_rect = glyph_rect.with_origin(origin);
    cache_rect.x0 /= size[0] as f64;
    cache_rect.x1 /= size[0] as f64;
    cache_rect.y0 /= size[1] as f64;
    cache_rect.y1 /= size[1] as f64;

    SvgPosInfo {
        rect: glyph_rect.with_size(Size::new(
            glyph_rect.size().width / scale,
            glyph_rect.size().height / scale,
        )),
        cache_rect,
    }
}
