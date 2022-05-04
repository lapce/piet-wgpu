use glow::HasContext;
use hashbrown::HashMap;
use linked_hash_map::LinkedHashMap;
use parley::layout::Glyph;
use piet::kurbo::{Point, Rect, Size};
use swash::{
    scale::{
        image::{Content, Image},
        Render, ScaleContext, Source, StrikeWith,
    },
    zeno::{self, Vector},
};

const IS_MACOS: bool = cfg!(target_os = "macos");
const SOURCES: &[Source] = &[
    Source::ColorBitmap(StrikeWith::BestFit),
    Source::ColorOutline(0),
    Source::Outline,
];

struct Row {
    y: u32,
    height: u32,
    width: u32,
    glyphs: Vec<GlyphPosInfo>,
}

pub struct Cache {
    pub gl_texture: glow::Texture,
    width: u32,
    height: u32,
    scx: ScaleContext,

    glyph_image: Image,

    rows: LinkedHashMap<usize, Row>,
    glyphs: HashMap<GlyphInfo, (usize, usize)>,
    pub(crate) scale: f64,
}

impl Cache {
    pub fn new(gl: &glow::Context, width: u32, height: u32) -> Cache {
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

        Cache {
            gl_texture,
            width,
            height,

            scx: ScaleContext::new(),

            glyph_image: Image::new(),

            rows: LinkedHashMap::new(),
            glyphs: HashMap::new(),
            scale: 1.0,
        }
    }

    pub(crate) fn get_glyph(
        &mut self,
        glyph: &Glyph,
        x: f32,
        font: &parley::Font,
        font_size: f32,
        gl: &glow::Context,
    ) -> Result<&GlyphPosInfo, piet::Error> {
        let scale = self.scale;
        let font_size = (font_size as f64 * scale).round() as u32;
        let subpx = [
            SubpixelOffset::quantize(x * scale as f32),
            SubpixelOffset::quantize(0.0),
        ];

        let glyph_info = GlyphInfo {
            font_id: font.as_ref().key.value() as usize,
            glyph_id: glyph.id as u32,
            font_size,
            subpx,
        };

        if let Some((row, index)) = self.glyphs.get(&glyph_info) {
            let row = self.rows.get(row).unwrap();
            return Ok(&row.glyphs[*index]);
        }

        let mut scaler = self
            .scx
            .builder(font.as_ref())
            .hint(!IS_MACOS)
            .size(font_size as f32)
            .build();

        let embolden = if IS_MACOS { 0.2 } else { 0. };

        self.glyph_image.data.clear();
        Render::new(SOURCES)
            .format(zeno::Format::CustomSubpixel([0.3, 0., -0.3]))
            .offset(Vector::new(subpx[0].to_f32(), subpx[1].to_f32()))
            .embolden(embolden)
            .render_into(&mut scaler, glyph.id, &mut self.glyph_image);

        let glyph_width = self.glyph_image.placement.width;
        let glyph_height = self.glyph_image.placement.height;
        let glyph_rect = Size::new(glyph_width as f64, glyph_height as f64)
            .to_rect()
            .with_origin(Point::new(
                self.glyph_image.placement.left as f64,
                self.glyph_image.placement.top as f64,
            ));

        let mut offset = [0, 0];
        let mut inserted = false;
        for (row_number, row) in self.rows.iter_mut().rev() {
            if row.height == glyph_height && self.width - row.width > glyph_width {
                let origin = Point::new(row.width as f64, row.y as f64);
                let glyph_pos = glyph_rect_to_pos(
                    glyph_rect,
                    origin,
                    [self.width, self.height],
                    self.glyph_image.content,
                );

                row.glyphs.push(glyph_pos);
                offset[0] = row.width;
                offset[1] = row.y;
                row.width += glyph_width;
                self.glyphs
                    .insert(glyph_info.clone(), (*row_number, row.glyphs.len() - 1));
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
            if self.height < y + glyph_height {
                return Err(piet::Error::MissingFont);
            }

            let origin = Point::new(0.0, y as f64);
            let glyph_pos = glyph_rect_to_pos(
                glyph_rect,
                origin,
                [self.width, self.height],
                self.glyph_image.content,
            );

            offset[0] = 0;
            offset[1] = y;
            let new_row = self.rows.len();
            let glyphs = vec![glyph_pos];
            let row = Row {
                y,
                height: glyph_height,
                width: glyph_width,
                glyphs,
            };

            self.rows.insert(new_row, row);
            self.glyphs.insert(glyph_info.clone(), (new_row, 0));
        }

        self.update(gl, offset);

        let (row, index) = self.glyphs.get(&glyph_info).unwrap();
        let row = self.rows.get(row).unwrap();
        Ok(&row.glyphs[*index])
    }

    pub fn update(&mut self, gl: &glow::Context, offset: [u32; 2]) {
        if self.glyph_image.data.is_empty() {
            return;
        }
        let width = self.glyph_image.placement.width;
        let height = self.glyph_image.placement.height;

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
                glow::PixelUnpackData::Slice(&self.glyph_image.data),
            );

            gl.bind_texture(glow::TEXTURE_2D, None);
        }
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, Default)]
pub(crate) struct GlyphInfo {
    font_id: usize,
    glyph_id: u32,
    font_size: u32,
    pub(crate) subpx: [SubpixelOffset; 2],
}

#[derive(Default, Clone)]
pub(crate) struct GlyphPosInfo {
    pub(crate) content: Content,
    pub(crate) rect: Rect,
    pub(crate) cache_rect: Rect,
}

fn glyph_rect_to_pos(
    glyph_rect: Rect,
    origin: Point,
    size: [u32; 2],
    content: Content,
) -> GlyphPosInfo {
    let mut cache_rect = glyph_rect.with_origin(origin);
    cache_rect.x0 /= size[0] as f64;
    cache_rect.x1 /= size[0] as f64;
    cache_rect.y0 /= size[1] as f64;
    cache_rect.y1 /= size[1] as f64;

    GlyphPosInfo {
        content,
        rect: glyph_rect.with_size(Size::new(glyph_rect.size().width, glyph_rect.size().height)),
        cache_rect,
    }
}

#[derive(Hash, Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
pub enum SubpixelOffset {
    Zero = 0,
    Quarter = 1,
    Half = 2,
    ThreeQuarters = 3,
}

impl Default for SubpixelOffset {
    fn default() -> Self {
        SubpixelOffset::Zero
    }
}

impl SubpixelOffset {
    // Skia quantizes subpixel offsets into 1/4 increments.
    // Given the absolute position, return the quantized increment
    fn quantize(pos: f32) -> Self {
        // Following the conventions of Gecko and Skia, we want
        // to quantize the subpixel position, such that abs(pos) gives:
        // [0.0, 0.125) -> Zero
        // [0.125, 0.375) -> Quarter
        // [0.375, 0.625) -> Half
        // [0.625, 0.875) -> ThreeQuarters,
        // [0.875, 1.0) -> Zero
        // The unit tests below check for this.
        let apos = ((pos - pos.floor()) * 8.0) as i32;
        match apos {
            1..=2 => SubpixelOffset::Quarter,
            3..=4 => SubpixelOffset::Half,
            5..=6 => SubpixelOffset::ThreeQuarters,
            _ => SubpixelOffset::Zero,
        }
    }

    pub(crate) fn to_f32(self) -> f32 {
        match self {
            SubpixelOffset::Zero => 0.0,
            SubpixelOffset::Quarter => 0.25,
            SubpixelOffset::Half => 0.5,
            SubpixelOffset::ThreeQuarters => 0.75,
        }
    }
}
