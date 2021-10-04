use std::collections::HashMap;
use std::hash::BuildHasherDefault;
use std::num::{NonZeroU32, NonZeroU64};
use std::sync::Arc;

use font_kit::canvas::{Canvas, Format, RasterizationOptions};
use font_kit::family_name::FamilyName;
use font_kit::font::Font;
use font_kit::hinting::HintingOptions;
use font_kit::loader::Loader;
use font_kit::source::SystemSource;
use include_dir::include_dir;
use include_dir::Dir;
use linked_hash_map::LinkedHashMap;
use lyon::lyon_tessellation::{
    BuffersBuilder, FillOptions, FillTessellator, FillVertex, FillVertexConstructor,
    StrokeVertexConstructor, VertexBuffers,
};
use lyon::tessellation;
use pathfinder_geometry::transform2d::Transform2F;
use pathfinder_geometry::vector::{Vector2F, Vector2I};
use piet::kurbo::{Affine, Point, Rect, Size};
use piet::{Color, FontFamily, FontWeight};
use rustc_hash::{FxHashMap, FxHasher};
use wgpu::util::DeviceExt;

const FONTS_DIR: Dir = include_dir!("./fonts");

#[repr(C)]
#[derive(Copy, Clone)]
struct Globals {
    resolution: [f32; 2],
    scale: f32,
    _pad: f32,
}

unsafe impl bytemuck::Pod for Globals {}
unsafe impl bytemuck::Zeroable for Globals {}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct Primitive {
    pub(crate) clip_rect: [f32; 4],
    pub(crate) transform_1: [f32; 4],
    pub(crate) blur_rect: [f32; 4],
    pub(crate) transform_2: [f32; 2],
    pub(crate) translate: [f32; 2],
    pub(crate) scale: [f32; 2],
    pub(crate) clip: f32,
    pub(crate) blur_radius: f32,
}

unsafe impl bytemuck::Pod for Primitive {}
unsafe impl bytemuck::Zeroable for Primitive {}

impl Default for Primitive {
    fn default() -> Self {
        Self {
            translate: [0.0, 0.0],
            scale: [1.0, 1.0],
            clip: 0.0,
            clip_rect: [0.0, 0.0, 0.0, 0.0],
            transform_1: [1.0, 0.0, 0.0, 1.0],
            transform_2: [0.0, 0.0],
            blur_rect: [0.0, 0.0, 0.0, 0.0],
            blur_radius: 0.0,
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct GpuVertex {
    pub(crate) pos: [f32; 2],
    pub(crate) translate: [f32; 2],
    pub(crate) color: [f32; 4],
    pub(crate) tex: f32,
    pub(crate) tex_pos: [f32; 2],
    pub(crate) primitive_id: u32,
}

unsafe impl bytemuck::Pod for GpuVertex {}
unsafe impl bytemuck::Zeroable for GpuVertex {}

impl Default for GpuVertex {
    fn default() -> Self {
        Self {
            pos: [0.0, 0.0],
            translate: [0.0, 0.0],
            color: [0.0, 0.0, 0.0, 0.0],
            tex: 0.0,
            tex_pos: [0.0, 0.0],
            primitive_id: 0,
        }
    }
}

pub struct Pipeline {
    pub pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    globals: wgpu::Buffer,
    primitives: wgpu::Buffer,
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    supported_primitives: usize,
    supported_vertices: usize,
    supported_indices: usize,
    pub(crate) size: Size,
    pub(crate) scale: f64,
}

impl Pipeline {
    pub fn new(device: &wgpu::Device, cache: &Cache) -> Self {
        let globals_buffer_byte_size = std::mem::size_of::<Globals>() as u64;
        let supported_primitives = 1000;
        let primitives_buffer_byte_size =
            std::mem::size_of::<Primitive>() as u64 * supported_primitives as u64;
        println!("primitives buffer size {}", primitives_buffer_byte_size);

        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Globals ubo"),
            size: globals_buffer_byte_size,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let primitives = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Pritives ubo"),
            size: primitives_buffer_byte_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let vertices = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Globals ubo"),
            size: std::mem::size_of::<GpuVertex>() as u64,
            usage: wgpu::BufferUsages::VERTEX,
            mapped_at_creation: false,
        });
        let indices = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Globals ubo"),
            size: std::mem::size_of::<u32>() as u64,
            usage: wgpu::BufferUsages::INDEX,
            mapped_at_creation: false,
        });

        let filter_mode = wgpu::FilterMode::Linear;
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: filter_mode,
            min_filter: filter_mode,
            mipmap_filter: filter_mode,
            ..Default::default()
        });

        let shader = device.create_shader_module(&wgpu::ShaderModuleDescriptor {
            label: Some("iced_wgpu::quad::shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(include_str!(
                "shader/geometry.wgsl"
            ))),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Bind group layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(globals_buffer_byte_size),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler {
                        filtering: true,
                        comparison: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(primitives_buffer_byte_size),
                    },
                    count: None,
                },
            ],
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Bind group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(globals.as_entire_buffer_binding()),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&cache.view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Buffer(primitives.as_entire_buffer_binding()),
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
            label: Some("pipeline layout"),
        });

        let render_pipeline_descriptor = wgpu::RenderPipelineDescriptor {
            label: Some("pipeline descriptor"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "main",
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<GpuVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array!(
                        0 => Float32x2,
                        1 => Float32x2,
                        2 => Float32x4,
                        3 => Float32,
                        4 => Float32x2,
                        5 => Uint32,
                    ),
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "main",
                targets: &[wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Bgra8Unorm,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                }],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                polygon_mode: wgpu::PolygonMode::Fill,
                front_face: wgpu::FrontFace::Ccw,
                strip_index_format: None,
                cull_mode: None,
                clamp_depth: false,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: 4,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
        };

        let pipeline = device.create_render_pipeline(&render_pipeline_descriptor);

        Self {
            pipeline,
            bind_group,
            globals,
            vertices,
            indices,
            primitives,
            supported_vertices: 1,
            supported_indices: 1,
            supported_primitives,
            size: Size::ZERO,
            scale: 1.0,
        }
    }

    pub fn draw(
        &mut self,
        device: &wgpu::Device,
        staging_belt: &mut wgpu::util::StagingBelt,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        msaa: &wgpu::TextureView,
        geometry: &VertexBuffers<GpuVertex, u32>,
        primitives: &[Primitive],
    ) {
        let fill_range = 0..(geometry.indices.len() as u32);

        if geometry.vertices.len() > self.supported_vertices {
            self.supported_vertices = geometry.vertices.len();
            let size = std::mem::size_of::<GpuVertex>() as u64 * self.supported_vertices as u64;
            self.vertices = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("vertices ubo"),
                size,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        if geometry.indices.len() > self.supported_indices {
            self.supported_indices = geometry.indices.len();
            let size = std::mem::size_of::<u32>() as u64 * self.supported_indices as u64;
            self.indices = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("indices ubo"),
                size,
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }

        {
            let vertices_bytes = bytemuck::cast_slice(&geometry.vertices);
            let mut vertices = staging_belt.write_buffer(
                encoder,
                &self.vertices,
                0,
                unsafe { NonZeroU64::new_unchecked(vertices_bytes.len() as u64) },
                device,
            );
            vertices.copy_from_slice(vertices_bytes);
        }
        {
            let indices_bytes = bytemuck::cast_slice(&geometry.indices);
            let mut indices = staging_belt.write_buffer(
                encoder,
                &self.indices,
                0,
                unsafe { NonZeroU64::new_unchecked(indices_bytes.len() as u64) },
                device,
            );
            indices.copy_from_slice(indices_bytes);
        }

        {
            let globals = vec![Globals {
                resolution: [self.size.width as f32, self.size.height as f32],
                scale: self.scale as f32,
                _pad: 0.0,
            }];

            let global_bytes = bytemuck::cast_slice(&globals);
            let mut globals = staging_belt.write_buffer(
                encoder,
                &self.globals,
                0,
                unsafe { NonZeroU64::new_unchecked(global_bytes.len() as u64) },
                device,
            );
            globals.copy_from_slice(global_bytes);
        }

        {
            if primitives.len() > self.supported_primitives {
                println!(
                    "warning primities len {} more than supported",
                    primitives.len()
                );
            }

            let primitives_bytes = bytemuck::cast_slice(
                &primitives[..primitives.len().min(self.supported_primitives)],
            );
            let mut primivites_buffer = staging_belt.write_buffer(
                encoder,
                &self.primitives,
                0,
                unsafe { NonZeroU64::new_unchecked(primitives_bytes.len() as u64) },
                device,
            );
            primivites_buffer.copy_from_slice(primitives_bytes);
        }

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[wgpu::RenderPassColorAttachment {
                    view: msaa,
                    resolve_target: Some(view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: true,
                    },
                }],
                depth_stencil_attachment: None,
            });

            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_vertex_buffer(0, self.vertices.slice(..));
            pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);

            pass.draw_indexed(fill_range.clone(), 0, 0..1);
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
    pub(crate) rect: Rect,
    pub(crate) cache_rect: Rect,
}

struct Row {
    y: u32,
    height: u32,
    width: u32,
    glyphs: Vec<GlyphPosInfo>,
}

type FxBuildHasher = BuildHasherDefault<FxHasher>;

pub struct Cache {
    texture: wgpu::Texture,
    pub(super) view: wgpu::TextureView,
    upload_buffer: wgpu::Buffer,
    upload_buffer_size: u64,
    width: u32,
    height: u32,

    font_source: SystemSource,
    fonts: Vec<Font>,
    fallback_fonts_range: std::ops::Range<usize>,
    fallback_fonts_loaded: bool,
    font_families: FxHashMap<(FontFamily, FontWeight), usize>,

    rows: LinkedHashMap<usize, Row, FxBuildHasher>,
    glyphs: FxHashMap<GlyphInfo, (usize, usize)>,
    glyph_infos: FxHashMap<(char, FontFamily, FontWeight), (usize, u32)>,
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
    const INITIAL_UPLOAD_BUFFER_SIZE: u64 = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as u64 * 100;

    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Cache {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("wgpu_glyph::Cache"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
            mip_level_count: 1,
            sample_count: 1,
        });

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let upload_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("wgpu_glyph::Cache upload buffer"),
            size: Self::INITIAL_UPLOAD_BUFFER_SIZE,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });

        Cache {
            texture,
            view,
            upload_buffer,
            upload_buffer_size: Self::INITIAL_UPLOAD_BUFFER_SIZE,
            width,
            height,

            font_source: SystemSource::new(),

            font_families: HashMap::default(),
            fonts: Vec::new(),
            fallback_fonts_range: 0..0,
            fallback_fonts_loaded: false,

            rows: LinkedHashMap::default(),
            glyphs: HashMap::default(),
            glyph_infos: HashMap::default(),
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
            let font_id = self.get_font_by_family(font_family.clone(), font_weight)?;
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
        device: &wgpu::Device,
        staging_belt: &mut wgpu::util::StagingBelt,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Result<&GlyphPosInfo, piet::Error> {
        let scale = self.scale * 2.0;

        let font_size = (font_size as f64 * scale).round() as u32;
        let glyph = self.get_glyph_info(c, font_family.clone(), font_weight, font_size)?;

        if let Some((row, index)) = self.glyphs.get(&glyph) {
            let row = self.rows.get(row).unwrap();
            return Ok(&row.glyphs[*index]);
        }

        let font = &self.fonts[glyph.font_id];
        let font_metrics = font.metrics();
        let units_per_em = font_metrics.units_per_em as f32;
        let glyph_width =
            font.advance(glyph.glyph_id).unwrap().x() / units_per_em * font_size as f32;
        let glyph_height = (font_metrics.ascent - font_metrics.descent + font_metrics.line_gap)
            / units_per_em
            * font_size as f32;
        let glyph_metric = GlyphMetricInfo {
            ascent: (font_metrics.ascent / units_per_em * font_size as f32) as f64 / scale,
            descent: (font_metrics.descent / units_per_em * font_size as f32) as f64 / scale,
            line_gap: (font_metrics.line_gap / units_per_em * font_size as f32) as f64 / scale,
            mono: font.is_monospace(),
        };
        let mut glyph_rect = Size::new(glyph_width as f64, glyph_height as f64).to_rect();

        let mut canvas = Canvas::new(
            Vector2I::new(glyph_width.ceil() as i32, glyph_height.ceil() as i32),
            Format::A8,
        );
        font.rasterize_glyph(
            &mut canvas,
            glyph.glyph_id,
            font_size as f32,
            Transform2F::from_translation(Vector2F::new(
                0.0,
                font_metrics.ascent / units_per_em * font_size as f32,
            )),
            HintingOptions::None,
            RasterizationOptions::GrayscaleAa,
        )
        .map_err(|_| piet::Error::MissingFont)?;

        let mut offset = [0, 0];
        let mut inserted = false;
        for (row_number, row) in self.rows.iter_mut().rev() {
            if row.height == glyph_height.ceil() as u32 {
                if self.width - row.width > glyph_width.ceil() as u32 {
                    let origin = Point::new(row.width as f64, row.y as f64);
                    glyph_rect = glyph_rect.with_origin(origin);
                    let mut cache_rect = glyph_rect.clone();
                    cache_rect.x0 /= self.width as f64;
                    cache_rect.x1 /= self.width as f64;
                    cache_rect.y0 /= self.height as f64;
                    cache_rect.y1 /= self.height as f64;
                    let glyph_pos = GlyphPosInfo {
                        info: glyph.clone(),
                        rect: glyph_rect.with_size(Size::new(
                            glyph_rect.size().width / scale,
                            glyph_rect.size().height / scale,
                        )),
                        metric: glyph_metric.clone(),
                        cache_rect,
                    };

                    row.glyphs.push(glyph_pos);
                    offset[0] = row.width;
                    offset[1] = row.y;
                    row.width += glyph_width.ceil() as u32;
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
                y = last_row.y + last_row.height + 1;
            }
            if self.height < y + glyph_height.ceil() as u32 {
                return Err(piet::Error::MissingFont);
            }

            let origin = Point::new(0.0, y as f64);
            glyph_rect = glyph_rect.with_origin(origin);
            let mut cache_rect = glyph_rect.clone();
            cache_rect.x0 /= self.width as f64;
            cache_rect.x1 /= self.width as f64;
            cache_rect.y0 /= self.height as f64;
            cache_rect.y1 /= self.height as f64;
            let glyph_pos = GlyphPosInfo {
                info: glyph.clone(),
                rect: glyph_rect.with_size(Size::new(
                    glyph_rect.size().width / scale,
                    glyph_rect.size().height / scale,
                )),
                metric: glyph_metric,
                cache_rect,
            };

            offset[0] = 0;
            offset[1] = y;
            let new_row = self.rows.len();
            let glyphs = vec![glyph_pos];
            let row = Row {
                y,
                height: glyph_height.ceil() as u32,
                width: glyph_width.ceil() as u32,
                glyphs,
            };

            self.rows.insert(new_row, row);
            self.glyphs.insert(glyph.clone(), (new_row, 0));
        }

        self.update(
            device,
            staging_belt,
            encoder,
            offset,
            [glyph_width.ceil() as u32, glyph_height.ceil() as u32],
            &canvas.pixels,
        );

        let (row, index) = self.glyphs.get(&glyph).unwrap();
        let row = self.rows.get(row).unwrap();
        Ok(&row.glyphs[*index])
    }

    fn get_font_by_family(
        &mut self,
        family: FontFamily,
        weight: FontWeight,
    ) -> Result<usize, piet::Error> {
        if !self.font_families.contains_key(&(family.clone(), weight)) {
            let font = self.get_new_font(&family, weight)?;
            let font_id = self.fonts.len();
            self.font_families.insert((family.clone(), weight), font_id);
            self.fonts.push(font);
        }

        let font_id = self.font_families.get(&(family.clone(), weight)).unwrap();
        Ok(*font_id)
    }

    fn get_new_font(&self, family: &FontFamily, weight: FontWeight) -> Result<Font, piet::Error> {
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
        let handle = self
            .font_source
            .select_best_match(
                &[family_name],
                &font_kit::properties::Properties::new()
                    .weight(font_kit::properties::Weight(weight.to_raw() as f32)),
            )
            .map_err(|e| piet::Error::MissingFont)?;
        let font = handle.load().map_err(|_| piet::Error::MissingFont)?;
        Ok(font)
    }

    pub fn update(
        &mut self,
        device: &wgpu::Device,
        staging_belt: &mut wgpu::util::StagingBelt,
        encoder: &mut wgpu::CommandEncoder,
        offset: [u32; 2],
        size: [u32; 2],
        data: &[u8],
    ) {
        let width = size[0] as usize;
        let height = size[1] as usize;

        if width == 0 || height == 0 {
            return;
        }

        // It is a webgpu requirement that:
        //  BufferCopyView.layout.bytes_per_row % wgpu::COPY_BYTES_PER_ROW_ALIGNMENT == 0
        // So we calculate padded_width by rounding width
        // up to the next multiple of wgpu::COPY_BYTES_PER_ROW_ALIGNMENT.
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;
        let padded_width_padding = (align - width % align) % align;
        let padded_width = width + padded_width_padding;

        let padded_data_size = (padded_width * height) as u64;

        if self.upload_buffer_size < padded_data_size {
            self.upload_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("wgpu_glyph::Cache upload buffer"),
                size: padded_data_size,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });

            self.upload_buffer_size = padded_data_size;
        }

        let mut padded_data = staging_belt.write_buffer(
            encoder,
            &self.upload_buffer,
            0,
            NonZeroU64::new(padded_data_size).unwrap(),
            device,
        );

        for row in 0..height {
            padded_data[row * padded_width..row * padded_width + width]
                .copy_from_slice(&data[row * width..(row + 1) * width])
        }

        // TODO: Move to use Queue for less buffer usage
        encoder.copy_buffer_to_texture(
            wgpu::ImageCopyBuffer {
                buffer: &self.upload_buffer,
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: NonZeroU32::new(padded_width as u32),
                    rows_per_image: NonZeroU32::new(height as u32),
                },
            },
            wgpu::ImageCopyTexture {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: u32::from(offset[0]),
                    y: u32::from(offset[1]),
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::Extent3d {
                width: size[0] as u32,
                height: size[1] as u32,
                depth_or_array_layers: 1,
            },
        );
    }
}
