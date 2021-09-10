use std::collections::HashMap;
use std::hash::BuildHasherDefault;
use std::num::{NonZeroU32, NonZeroU64};

use font_kit::canvas::{Canvas, Format, RasterizationOptions};
use font_kit::family_name::FamilyName;
use font_kit::font::Font;
use font_kit::hinting::HintingOptions;
use font_kit::source::SystemSource;
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
#[derive(Copy, Clone, Debug)]
pub struct GpuVertex {
    pub(crate) pos: [f32; 2],
    pub(crate) z: f32,
    pub(crate) translate: [f32; 2],
    pub(crate) scale: [f32; 2],
    pub(crate) color: [f32; 4],
    pub(crate) normal: [f32; 2],
    pub(crate) width: f32,
    pub(crate) blur_rect: [f32; 4],
    pub(crate) blur_radius: f32,
    pub(crate) tex: f32,
    pub(crate) tex_pos: [f32; 2],
}

unsafe impl bytemuck::Pod for GpuVertex {}
unsafe impl bytemuck::Zeroable for GpuVertex {}

impl Default for GpuVertex {
    fn default() -> Self {
        Self {
            pos: [0.0, 0.0],
            z: 0.0,
            translate: [0.0, 0.0],
            scale: [1.0, 1.0],
            color: [0.0, 0.0, 0.0, 0.0],
            normal: [0.0, 0.0],
            width: 0.0,
            blur_rect: [0.0, 0.0, 0.0, 0.0],
            blur_radius: 0.0,
            tex: 0.0,
            tex_pos: [0.0, 0.0],
        }
    }
}

pub struct Pipeline {
    pub pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    globals: wgpu::Buffer,
    pub(crate) cache: Cache,
    pub(crate) size: Size,
    pub(crate) scale: f64,
}

impl Pipeline {
    pub fn new(device: &wgpu::Device) -> Self {
        let globals_buffer_byte_size = std::mem::size_of::<Globals>() as u64;

        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Globals ubo"),
            size: globals_buffer_byte_size,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
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
            ],
        });

        let cache = Cache::new(device, 2000, 2000);
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
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
            label: None,
        });

        let render_pipeline_descriptor = wgpu::RenderPipelineDescriptor {
            label: None,
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "main",
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<GpuVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array!(
                        0 => Float32x2,
                        1 => Float32,
                        2 => Float32x2,
                        3 => Float32x2,
                        4 => Float32x4,
                        5 => Float32x2,
                        6 => Float32,
                        7 => Float32x4,
                        8 => Float32,
                        9 => Float32,
                        10 => Float32x2,
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
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
        };

        let pipeline = device.create_render_pipeline(&render_pipeline_descriptor);

        Self {
            pipeline,
            bind_group,
            globals,
            cache,
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
        depth_stencil_attachment: wgpu::RenderPassDepthStencilAttachment,
        geometry: &VertexBuffers<GpuVertex, u32>,
    ) {
        let fill_range = 0..(geometry.indices.len() as u32);

        let vbo = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&geometry.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let ibo = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&geometry.indices),
            usage: wgpu::BufferUsages::INDEX,
        });

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
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[wgpu::RenderPassColorAttachment {
                    view: view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: true,
                    },
                }],
                depth_stencil_attachment: None,
            });

            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_vertex_buffer(0, vbo.slice(..));
            pass.set_index_buffer(ibo.slice(..), wgpu::IndexFormat::Uint32);

            pass.draw_indexed(fill_range.clone(), 0, 0..1);
        }
    }

    pub fn fill_rect(
        &mut self,
        rect: &Rect,
        color: &Color,
        affine: &Affine,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        queue: &mut wgpu::Queue,
        view: &wgpu::TextureView,
    ) {
        println!("now fill rect {} {:?} {}", rect, affine, self.size);
        let tolerance = 0.02;
        let mut geometry: VertexBuffers<GpuVertex, u16> = VertexBuffers::new();
        let mut fill_tess = FillTessellator::new();
        let color = color.as_rgba();
        let color = [
            color.0 as f32,
            color.1 as f32,
            color.2 as f32,
            color.3 as f32,
        ];
        let affine = affine.as_coeffs();
        let translate = [affine[4] as f32, affine[5] as f32];
        fill_tess.tessellate_rectangle(
            &lyon::geom::Rect::new(
                lyon::geom::Point::new(rect.x0 as f32, rect.y0 as f32),
                lyon::geom::Size::new(rect.width() as f32, rect.height() as f32),
            ),
            &FillOptions::tolerance(tolerance).with_fill_rule(tessellation::FillRule::NonZero),
            &mut BuffersBuilder::new(&mut geometry, |vertex: FillVertex| GpuVertex {
                pos: vertex.position().to_array(),
                translate,
                color,
                ..Default::default()
            }),
        );
        let fill_range = 0..(geometry.indices.len() as u32);

        let vbo = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&geometry.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let ibo = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&geometry.indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        queue.write_buffer(
            &self.globals,
            0,
            bytemuck::cast_slice(&[Globals {
                resolution: [self.size.width as f32, self.size.height as f32],
                scale: self.scale as f32,
                _pad: 0.0,
            }]),
        );

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: true,
                    },
                }],
                depth_stencil_attachment: None,
            });

            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_vertex_buffer(0, vbo.slice(..));
            pass.set_index_buffer(ibo.slice(..), wgpu::IndexFormat::Uint16);

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
    fonts: FxHashMap<(FontFamily, FontWeight), Font>,
    font_ids: FxHashMap<(FontFamily, FontWeight), usize>,
    rows: LinkedHashMap<usize, Row, FxBuildHasher>,
    glyphs: FxHashMap<GlyphInfo, (usize, usize)>,
    pub(crate) scale: f64,
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
            fonts: HashMap::default(),
            font_ids: HashMap::default(),
            rows: LinkedHashMap::default(),
            glyphs: HashMap::default(),
            scale: 1.0,
        }
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
        let (font, font_id) = self.get_font(font_family.clone(), font_weight)?;
        let glyph_id = font.glyph_for_char(c).ok_or(piet::Error::MissingFont)?;
        let glyph = GlyphInfo {
            font_id,
            glyph_id,
            font_size,
        };

        if let Some((row, index)) = self.glyphs.get(&glyph) {
            let row = self.rows.get(row).unwrap();
            return Ok(&row.glyphs[*index]);
        }

        let (font, font_id) = self.get_font(font_family, font_weight)?;
        let font_metrics = font.metrics();
        let units_per_em = font_metrics.units_per_em as f32;
        let glyph_width = font.advance(glyph_id).unwrap().x() / units_per_em * font_size as f32;
        let glyph_height = (font_metrics.ascent - font_metrics.descent + font_metrics.line_gap)
            / units_per_em
            * font_size as f32;
        let glyph_metric = GlyphMetricInfo {
            ascent: (font_metrics.ascent / units_per_em * font_size as f32) as f64 / scale,
            descent: (font_metrics.descent / units_per_em * font_size as f32) as f64 / scale,
            line_gap: (font_metrics.line_gap / units_per_em * font_size as f32) as f64 / scale,
        };
        let mut glyph_rect = Size::new(glyph_width as f64, glyph_height as f64).to_rect();

        let mut canvas = Canvas::new(
            Vector2I::new(glyph_width.ceil() as i32, glyph_height.ceil() as i32),
            Format::A8,
        );
        font.rasterize_glyph(
            &mut canvas,
            glyph_id,
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
                y = last_row.y + last_row.height;
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

    fn get_font(
        &mut self,
        family: FontFamily,
        weight: FontWeight,
    ) -> Result<(&Font, usize), piet::Error> {
        if !self.fonts.contains_key(&(family.clone(), weight)) {
            let font = self.get_new_font(&family, weight)?;
            self.fonts.insert((family.clone(), weight), font);
            self.font_ids
                .insert((family.clone(), weight), self.font_ids.len());
        }
        Ok((
            self.fonts.get(&(family.clone(), weight)).unwrap(),
            *self.font_ids.get(&(family.clone(), weight)).unwrap(),
        ))
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
