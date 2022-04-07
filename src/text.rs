use std::{cell::RefCell, rc::Rc, sync::Arc};

use font_kit::{family_name::FamilyName, font::Font, source::SystemSource};
use glow::HasContext;
use hashbrown::HashMap;
use include_dir::{include_dir, Dir};
use linked_hash_map::LinkedHashMap;
use piet::{
    kurbo::{Point, Rect, Size},
    FontFamily, FontWeight,
};
use swash::{
    scale::{image::Image, Render, ScaleContext, Source, StrikeWith},
    zeno::{self, Vector},
    CacheKey, FontRef,
};

use crate::{context::Tex, pipeline::create_program};

const MAX_INSTANCES: usize = 100_000;
const FONTS_DIR: Dir = include_dir!("./fonts");
const DEFAULT_FONT: &[u8] = include_bytes!("../fonts/CascadiaCode-Regular.otf");
const IS_MACOS: bool = cfg!(target_os = "macos");
const SOURCES: &[Source] = &[
    Source::ColorBitmap(StrikeWith::BestFit),
    Source::ColorOutline(0),
    Source::Outline,
];

pub struct Pipeline {
    program: <glow::Context as HasContext>::Program,
    vertex_array: <glow::Context as HasContext>::VertexArray,
    instances: <glow::Context as HasContext>::Buffer,
    scale_location: <glow::Context as HasContext>::UniformLocation,
    view_proj: <glow::Context as HasContext>::UniformLocation,
    depth_location: <glow::Context as HasContext>::UniformLocation,
    cache: Rc<RefCell<Cache>>,
    current_scale: f32,
}

impl Pipeline {
    pub fn new(gl: &glow::Context, cache: Rc<RefCell<Cache>>) -> Self {
        let program = unsafe {
            create_program(
                gl,
                &[
                    (glow::VERTEX_SHADER, include_str!("./shader/tex.vert")),
                    (glow::FRAGMENT_SHADER, include_str!("./shader/tex.frag")),
                ],
            )
        };

        let scale_location =
            unsafe { gl.get_uniform_location(program, "u_scale") }.expect("Get scale location");
        let depth_location =
            unsafe { gl.get_uniform_location(program, "u_depth") }.expect("Get depth location");
        let view_proj = unsafe { gl.get_uniform_location(program, "view_proj") }
            .expect("Get view_proj location");

        unsafe {
            gl.use_program(Some(program));

            gl.uniform_1_f32(Some(&scale_location), 1.0);

            gl.use_program(None);
        }

        let (vertex_array, instances) = unsafe { create_instance_buffer(gl, MAX_INSTANCES) };

        Self {
            vertex_array,
            instances,
            program,
            cache,
            scale_location,
            depth_location,
            view_proj,
            current_scale: 1.0,
        }
    }

    pub fn draw(
        &mut self,
        gl: &glow::Context,
        instances: &[Tex],
        scale: f32,
        view_proj: &[f32],
        max_depth: u32,
    ) {
        if instances.is_empty() {
            return;
        }

        unsafe {
            gl.use_program(Some(self.program));
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.cache.borrow().gl_texture));
            gl.bind_vertex_array(Some(self.vertex_array));
            gl.uniform_matrix_4_f32_slice(Some(&self.view_proj), false, view_proj);
            gl.uniform_1_f32(Some(&self.depth_location), max_depth as f32);
        }

        if scale != self.current_scale {
            unsafe {
                gl.uniform_1_f32(Some(&self.scale_location), scale);
            }

            self.current_scale = scale;
        }

        unsafe {
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.instances));
            gl.buffer_sub_data_u8_slice(glow::ARRAY_BUFFER, 0, bytemuck::cast_slice(instances));
            gl.bind_buffer(glow::ARRAY_BUFFER, None);

            gl.draw_arrays_instanced(glow::TRIANGLE_STRIP, 0, 4, instances.len() as i32);
            gl.bind_vertex_array(None);
            gl.bind_texture(glow::TEXTURE_2D, None);
            gl.use_program(None);
        }
    }
}

unsafe fn create_instance_buffer(
    gl: &glow::Context,
    size: usize,
) -> (
    <glow::Context as HasContext>::VertexArray,
    <glow::Context as HasContext>::Buffer,
) {
    let vertex_array = gl.create_vertex_array().expect("Create vertex array");
    let buffer = gl.create_buffer().expect("Create instance buffer");

    gl.bind_vertex_array(Some(vertex_array));
    gl.bind_buffer(glow::ARRAY_BUFFER, Some(buffer));
    gl.buffer_data_size(
        glow::ARRAY_BUFFER,
        (size * std::mem::size_of::<Tex>()) as i32,
        glow::DYNAMIC_DRAW,
    );

    let stride = std::mem::size_of::<Tex>() as i32;

    gl.enable_vertex_attrib_array(0);
    gl.vertex_attrib_pointer_f32(0, 4, glow::FLOAT, false, stride, 0);
    gl.vertex_attrib_divisor(0, 1);

    gl.enable_vertex_attrib_array(1);
    gl.vertex_attrib_pointer_f32(1, 4, glow::FLOAT, false, stride, 4 * 4);
    gl.vertex_attrib_divisor(1, 1);

    gl.enable_vertex_attrib_array(2);
    gl.vertex_attrib_pointer_f32(2, 4, glow::FLOAT, false, stride, 4 * (4 + 4));
    gl.vertex_attrib_divisor(2, 1);

    gl.enable_vertex_attrib_array(3);
    gl.vertex_attrib_pointer_f32(3, 1, glow::FLOAT, false, stride, 4 * (4 + 4 + 4));
    gl.vertex_attrib_divisor(3, 1);

    gl.enable_vertex_attrib_array(4);
    gl.vertex_attrib_pointer_f32(4, 4, glow::FLOAT, false, stride, 4 * (4 + 4 + 4 + 1));
    gl.vertex_attrib_divisor(4, 1);

    gl.bind_vertex_array(None);
    gl.bind_buffer(glow::ARRAY_BUFFER, None);

    (vertex_array, buffer)
}

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

    font_source: SystemSource,
    fonts: Vec<Font>,
    piet_fonts: Vec<PietFont>,
    default_font: Font,
    default_piet_font: PietFont,
    fallback_fonts_range: std::ops::Range<usize>,
    fallback_fonts_loaded: bool,
    font_families: HashMap<(FontFamily, FontWeight), usize>,

    glyph_image: Image,

    rows: LinkedHashMap<usize, Row>,
    glyphs: HashMap<GlyphInfo, (usize, usize)>,
    glyph_infos: HashMap<(char, FontFamily, FontWeight), (usize, u32)>,
    pub(crate) scale: f64,
}

fn get_fallback_fonts() -> Vec<Font> {
    let mut fonts = Vec::new();
    for file in FONTS_DIR.files() {
        if let Ok(font) = Font::from_bytes(Arc::new(file.contents().to_vec()), 0) {
            fonts.push(font);
        }
    }
    fonts
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

        let default_font = Font::from_bytes(Arc::new(DEFAULT_FONT.to_vec()), 0).unwrap();
        let default_piet_font = PietFont::from_data(DEFAULT_FONT);

        Cache {
            gl_texture,
            width,
            height,

            scx: ScaleContext::new(),

            font_source: SystemSource::new(),

            font_families: HashMap::new(),
            fonts: Vec::new(),
            piet_fonts: Vec::new(),
            default_font,
            default_piet_font,
            fallback_fonts_range: 0..0,
            fallback_fonts_loaded: false,

            glyph_image: Image::new(),

            rows: LinkedHashMap::new(),
            glyphs: HashMap::new(),
            glyph_infos: HashMap::new(),
            scale: 1.0,
        }
    }

    fn get_glyph_from_fallback_fonts(&mut self, c: char) -> Option<(usize, u32)> {
        if !self.fallback_fonts_loaded {
            self.fallback_fonts_loaded = true;
            let mut fallback_fonts = get_fallback_fonts();
            let start = self.fonts.len();
            let end = start + fallback_fonts.len();
            self.fonts.append(&mut fallback_fonts);
            self.fallback_fonts_range = start..end;
        }
        if self.fallback_fonts_range.clone().count() == 0 {
            return None;
        }

        for font_id in self.fallback_fonts_range.clone() {
            let font = &self.fonts[font_id];
            if let Some(glyph_id) = font.glyph_for_char(c) {
                return Some((font_id, glyph_id));
            }
        }
        None
    }

    fn get_glyph_info(
        &mut self,
        c: char,
        font_family: FontFamily,
        font_weight: FontWeight,
        font_size: u32,
    ) -> Result<GlyphInfo, piet::Error> {
        let key = (c, font_family.clone(), font_weight);
        if !self.glyph_infos.contains_key(&key) {
            let font_id = self.get_font_by_family(font_family, font_weight);
            let font = &self.fonts[font_id];

            let (font_id, glyph_id) = if let Some(glyph_id) = font.glyph_for_char(c) {
                (font_id, glyph_id)
            } else {
                self.get_glyph_from_fallback_fonts(c)
                    .ok_or(piet::Error::MissingFont)?
            };

            self.glyph_infos.insert(key.clone(), (font_id, glyph_id));
        }

        let (font_id, glyph_id) = self.glyph_infos.get(&key).unwrap();

        Ok(GlyphInfo {
            font_id: *font_id,
            font_size,
            glyph_id: *glyph_id,
        })
    }

    pub(crate) fn get_glyph_pos(
        &mut self,
        c: char,
        font_family: FontFamily,
        font_size: f32,
        font_weight: FontWeight,
        gl: &glow::Context,
    ) -> Result<&GlyphPosInfo, piet::Error> {
        let scale = self.scale;

        let font_size = (font_size as f64 * scale).round() as u32;
        let glyph = self.get_glyph_info(c, font_family.clone(), font_weight, font_size)?;

        if let Some((row, index)) = self.glyphs.get(&glyph) {
            let row = self.rows.get(row).unwrap();
            return Ok(&row.glyphs[*index]);
        }

        let padding = 2.0;
        let font = &self.fonts[glyph.font_id];
        let font_metrics = font.metrics();
        let units_per_em = font_metrics.units_per_em as f32;
        let glyph_real_width =
            font.advance(glyph.glyph_id).unwrap().x() / units_per_em * font_size as f32;
        let glyph_real_height =
            (font_metrics.ascent - font_metrics.descent + font_metrics.line_gap) / units_per_em
                * font_size as f32;
        let glyph_metric = GlyphMetricInfo {
            ascent: (font_metrics.ascent / units_per_em * font_size as f32) as f64 / scale,
            descent: (font_metrics.descent / units_per_em * font_size as f32) as f64 / scale,
            line_gap: (font_metrics.line_gap / units_per_em * font_size as f32) as f64 / scale,
            mono: font.is_monospace(),
        };
        let glyph_rect = Size::new(glyph_real_width as f64, glyph_real_height as f64).to_rect();

        let glyph_width = glyph_real_width.ceil() as u32 + padding as u32;
        let glyph_height = glyph_real_height.ceil() as u32 + padding as u32;

        let x = 0.0;
        let y = 0.0;
        let subpx = [SubpixelOffset::quantize(x), SubpixelOffset::quantize(y)];

        let piet_font = &self.piet_fonts[glyph.font_id];

        let mut scaler = self
            .scx
            .builder(piet_font.as_ref())
            .hint(!IS_MACOS)
            .size(font_size as f32)
            .build();

        let embolden = if IS_MACOS { 0.2 } else { 0. };

        self.glyph_image.data.clear();
        Render::new(SOURCES)
            .format(zeno::Format::CustomSubpixel([0.3, 0., -0.3]))
            .offset(Vector::new(subpx[0].to_f32(), subpx[1].to_f32()))
            .embolden(embolden)
            .render_into(&mut scaler, glyph.glyph_id as u16, &mut self.glyph_image);

        let mut offset = [0, 0];
        let mut inserted = false;
        for (row_number, row) in self.rows.iter_mut().rev() {
            if row.height == glyph_height {
                if self.width - row.width > glyph_width {
                    let origin = Point::new(
                        row.width as f64 + padding as f64 / 2.0,
                        row.y as f64 + padding as f64 / 2.0,
                    );
                    let glyph_pos = glyph_rect_to_pos(
                        glyph_rect,
                        origin,
                        &glyph,
                        &glyph_metric,
                        scale,
                        [self.width, self.height],
                    );

                    row.glyphs.push(glyph_pos);
                    offset[0] = row.width;
                    offset[1] = row.y;
                    row.width += glyph_width;
                    self.glyphs
                        .insert(glyph.clone(), (*row_number, row.glyphs.len() - 1));
                    inserted = true;
                    break;
                }
            }
        }

        if !inserted {
            let mut y = 0;
            if self.rows.len() > 0 {
                let last_row = self.rows.get(&(self.rows.len() - 1)).unwrap();
                y = last_row.y + last_row.height;
            }
            if self.height < y + glyph_height {
                return Err(piet::Error::MissingFont);
            }

            let origin = Point::new(0.0 + padding as f64 / 2.0, y as f64 + padding as f64 / 2.0);
            let glyph_pos = glyph_rect_to_pos(
                glyph_rect,
                origin,
                &glyph,
                &glyph_metric,
                scale,
                [self.width, self.height],
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
            self.glyphs.insert(glyph.clone(), (new_row, 0));
        }

        self.update(
            gl,
            offset,
            [
                glyph_real_width.ceil() as u32,
                (glyph_metric.ascent * scale).ceil() as u32,
            ],
        );

        let (row, index) = self.glyphs.get(&glyph).unwrap();
        let row = self.rows.get(row).unwrap();
        Ok(&row.glyphs[*index])
    }

    fn get_font_by_family(&mut self, family: FontFamily, weight: FontWeight) -> usize {
        if !self.font_families.contains_key(&(family.clone(), weight)) {
            let (font, piet_font) = self.get_new_font(&family, weight);
            let font_id = self.fonts.len();
            self.font_families.insert((family.clone(), weight), font_id);
            self.fonts.push(font);
            self.piet_fonts.push(piet_font);
        }

        let font_id = self.font_families.get(&(family, weight)).unwrap();
        *font_id
    }

    fn get_new_font(&self, family: &FontFamily, weight: FontWeight) -> (Font, PietFont) {
        let family_name = match family.inner() {
            piet::FontFamilyInner::Serif => FamilyName::Serif,
            piet::FontFamilyInner::SansSerif => FamilyName::SansSerif,
            piet::FontFamilyInner::Monospace => FamilyName::Monospace,
            piet::FontFamilyInner::SystemUi => FamilyName::SansSerif,
            piet::FontFamilyInner::Named(name) => {
                font_kit::family_name::FamilyName::Title(name.to_string())
            }
            _ => FamilyName::SansSerif,
        };
        let font = self
            .font_source
            .select_best_match(
                &[family_name],
                &font_kit::properties::Properties::new()
                    .weight(font_kit::properties::Weight(weight.to_raw() as f32)),
            )
            .ok()
            .and_then(|h| h.load().ok())
            .unwrap_or_else(|| self.default_font.clone());
        let piet_font = font
            .copy_font_data()
            .map(|d| PietFont::from_data(&d))
            .unwrap_or_else(|| self.default_piet_font.clone());
        (font, piet_font)
    }

    pub fn update(&mut self, gl: &glow::Context, offset: [u32; 2], size: [u32; 2]) {
        let width = self.glyph_image.placement.width;
        let height = self.glyph_image.placement.height;

        unsafe {
            gl.bind_texture(glow::TEXTURE_2D, Some(self.gl_texture));

            gl.tex_sub_image_2d(
                glow::TEXTURE_2D,
                0,
                offset[0] as i32 + self.glyph_image.placement.left + 1,
                offset[1] as i32 + (size[1] as i32 - self.glyph_image.placement.top),
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
}

#[derive(Default, Clone)]
pub(crate) struct GlyphMetricInfo {
    pub(crate) ascent: f64,
    pub(crate) descent: f64,
    pub(crate) line_gap: f64,
    pub(crate) mono: bool,
}

#[derive(Default, Clone)]
pub(crate) struct GlyphPosInfo {
    pub(crate) info: GlyphInfo,
    pub(crate) metric: GlyphMetricInfo,
    pub(crate) width: f64,
    pub(crate) rect: Rect,
    pub(crate) cache_rect: Rect,
}

impl GlyphPosInfo {
    pub fn empty(width: f64) -> Self {
        GlyphPosInfo {
            info: GlyphInfo {
                font_id: 0,
                glyph_id: 0,
                font_size: 0,
            },
            metric: GlyphMetricInfo {
                ascent: 0.0,
                descent: 0.0,
                line_gap: 0.0,
                mono: false,
            },
            width: width,
            rect: Size::new(width, 0.0).to_rect(),
            cache_rect: Rect::ZERO,
        }
    }
}

fn glyph_rect_to_pos(
    glyph_rect: Rect,
    origin: Point,
    glyph: &GlyphInfo,
    glyph_metric: &GlyphMetricInfo,
    scale: f64,
    size: [u32; 2],
) -> GlyphPosInfo {
    let glyph_rect = glyph_rect.with_origin(origin);
    let mut cache_rect = glyph_rect.clone();
    cache_rect.x0 /= size[0] as f64;
    cache_rect.x1 /= size[0] as f64;
    cache_rect.y0 /= size[1] as f64;
    cache_rect.y1 /= size[1] as f64;
    let glyph_pos = GlyphPosInfo {
        info: glyph.clone(),
        rect: glyph_rect.with_size(Size::new(
            glyph_rect.size().width / scale,
            glyph_rect.size().height / scale,
        )),
        width: glyph_rect.size().width / scale,
        metric: glyph_metric.clone(),
        cache_rect,
    };
    glyph_pos
}

#[derive(Hash, Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
pub enum SubpixelOffset {
    Zero = 0,
    Quarter = 1,
    Half = 2,
    ThreeQuarters = 3,
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

    fn to_f32(self) -> f32 {
        match self {
            SubpixelOffset::Zero => 0.0,
            SubpixelOffset::Quarter => 0.25,
            SubpixelOffset::Half => 0.5,
            SubpixelOffset::ThreeQuarters => 0.75,
        }
    }
}

#[derive(Clone)]
struct PietFont {
    data: Arc<Vec<u8>>,
    key: CacheKey,
}

impl PietFont {
    fn from_data(data: &[u8]) -> Self {
        Self {
            data: Arc::new(data.to_vec()),
            key: CacheKey::new(),
        }
    }

    pub fn as_ref(&self) -> FontRef {
        FontRef {
            data: &self.data,
            offset: 0,
            key: self.key,
        }
    }
}
