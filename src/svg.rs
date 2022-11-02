use std::sync::Arc;
use std::{collections::HashMap, str::FromStr};

use glow::HasContext;
use linked_hash_map::LinkedHashMap;
use piet::kurbo::{Point, Rect, Size};
use sha2::{Digest, Sha256};
use resvg::usvg;
use crate::context::WgpuImage;

#[derive(Clone)]
pub struct Svg {
    hash: Arc<Vec<u8>>,
    pub(crate) tree: Arc<usvg::Tree>,
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
            Ok(tree) => Ok(Self {
                hash: Arc::new(hash),
                tree: Arc::new(tree),
            }),
            Err(err) => Err(err.into()),
        }
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, Default)]
pub(crate) struct AtlasInfo {
    hash: Vec<u8>,
    width: u32,
    height: u32,
}

#[derive(Default, Clone)]
pub(crate) struct AtlasPosInfo {
    pub(crate) rect: Rect,
    pub(crate) cache_rect: Rect,
}

struct AtlasRow {
    y: u32,
    height: u32,
    width: u32,
    maps: Vec<AtlasPosInfo>,
}

pub struct AtlasCache {
    pub gl_texture: glow::Texture,
    width: u32,
    height: u32,

    rows: LinkedHashMap<usize, AtlasRow>,
    maps: HashMap<AtlasInfo, (usize, usize)>,
    pub(crate) scale: f64,
}

impl AtlasCache {
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
            maps: HashMap::new(),
            scale: 1.0,
        }
    }

    fn update(
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

    fn update_atlas(
        &mut self,
        gl: &glow::Context,
        info: &AtlasInfo,
        data: &[u8],
    ) -> Result<(), piet::Error> {
        let scale = self.scale;
        let atlas_rect = Size::new(info.width as f64, info.height as f64).to_rect();
        let mut offset = [0, 0];
        let mut inserted = false;
        for (row_number, row) in self.rows.iter_mut().rev() {
            if row.height == info.height && self.width - row.width > info.width {
                let origin = Point::new(row.width as f64, row.y as f64);
                let glyph_pos =
                    atlas_rect_to_pos(atlas_rect, origin, scale, [self.width, self.height]);

                row.maps.push(glyph_pos);
                offset[0] = row.width;
                offset[1] = row.y;
                row.width += info.width;
                self.maps
                    .insert(info.clone(), (*row_number, row.maps.len() - 1));
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
            if self.height < y + info.height {
                return Err(piet::Error::MissingFont);
            }

            let origin = Point::new(0.0, y as f64);
            let atlas_pos = atlas_rect_to_pos(atlas_rect, origin, scale, [self.width, self.height]);

            offset[0] = 0;
            offset[1] = y;
            let new_row = self.rows.len();
            let maps = vec![atlas_pos];
            let row = AtlasRow {
                y,
                height: info.height,
                width: info.width,
                maps,
            };

            self.rows.insert(new_row, row);
            self.maps.insert(info.clone(), (new_row, 0));
        }

        self.update(gl, offset, data, info.width, info.height);

        Ok(())
    }

    pub(crate) fn get_img(
        &mut self,
        gl: &glow::Context,
        img: &WgpuImage,
    ) -> Result<&AtlasPosInfo, piet::Error> {
        let (width, height) = img.img.dimensions();
        let info = AtlasInfo {
            hash: img.hash.clone(),
            width,
            height,
        };

        if let Some((row, index)) = self.maps.get(&info) {
            let row = self.rows.get(row).unwrap();
            return Ok(&row.maps[*index]);
        }

        self.update_atlas(gl, &info, img.img.as_raw().as_slice())?;

        let (row, index) = self.maps.get(&info).unwrap();
        let row = self.rows.get(row).unwrap();
        Ok(&row.maps[*index])
    }

    pub(crate) fn get_svg(
        &mut self,
        gl: &glow::Context,
        svg: &Svg,
        [width, height]: [f32; 2],
    ) -> Result<&AtlasPosInfo, piet::Error> {
        let (width, height) = (width.ceil() as u32, height.ceil() as u32);
        let info = AtlasInfo {
            hash: (*svg.hash).clone(),
            width,
            height,
        };

        if let Some((row, index)) = self.maps.get(&info) {
            let row = self.rows.get(row).unwrap();
            return Ok(&row.maps[*index]);
        }

        let transform = tiny_skia::Transform::identity();
        let mut img = tiny_skia::Pixmap::new(width, height).ok_or(piet::Error::InvalidInput)?;

        resvg::render(
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

        let data = img.take();
        self.update_atlas(gl, &info, &data)?;

        let (row, index) = self.maps.get(&info).unwrap();
        let row = self.rows.get(row).unwrap();
        Ok(&row.maps[*index])
    }
}

fn atlas_rect_to_pos(atlas_rect: Rect, origin: Point, scale: f64, size: [u32; 2]) -> AtlasPosInfo {
    let mut cache_rect = atlas_rect.with_origin(origin);
    cache_rect.x0 /= size[0] as f64;
    cache_rect.x1 /= size[0] as f64;
    cache_rect.y0 /= size[1] as f64;
    cache_rect.y1 /= size[1] as f64;

    AtlasPosInfo {
        rect: atlas_rect.with_size(Size::new(
            atlas_rect.size().width / scale,
            atlas_rect.size().height / scale,
        )),
        cache_rect,
    }
}
