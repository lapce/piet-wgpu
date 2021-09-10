use bytemuck::{Pod, Zeroable};
use font_kit::{
    canvas::{Canvas, Format, RasterizationOptions},
    family_name::FamilyName,
    font::Font,
    hinting::HintingOptions,
    source::SystemSource,
};
use linked_hash_map::LinkedHashMap;
use pathfinder_geometry::{
    transform2d::Transform2F,
    vector::{Vector2F, Vector2I},
};
use piet::{
    kurbo::{Point, Rect, Size},
    FontFamily,
};
use rustc_hash::{FxHashMap, FxHasher};
use std::{
    collections::HashMap,
    hash::BuildHasherDefault,
    mem,
    num::{NonZeroU32, NonZeroU64},
};
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Copy, Clone)]
struct Globals {
    resolution: [f32; 2],
    translate: [f32; 2],
    scale: f32,
    _pad: f32,
}

unsafe impl bytemuck::Pod for Globals {}
unsafe impl bytemuck::Zeroable for Globals {}

pub(crate) struct Pipeline {
    pipeline: wgpu::RenderPipeline,
    instances: wgpu::Buffer,
    globals: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    pub(crate) pending_instances: Vec<Instance>,
    pub(crate) size: Size,
    pub(crate) cache: Cache,
}

impl Pipeline {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let globals_buffer_byte_size = std::mem::size_of::<Globals>() as u64;
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
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

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("wgpu_glyph::Pipeline uniforms"),
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
            label: Some("wgpu_glyph::Pipeline uniforms"),
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

        let instances = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("wgpu_glyph::Pipeline instances"),
            size: mem::size_of::<Instance>() as u64 * Instance::INITIAL_AMOUNT as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            push_constant_ranges: &[],
            bind_group_layouts: &[&bind_group_layout],
        });

        let shader = device.create_shader_module(&wgpu::ShaderModuleDescriptor {
            label: Some("Text Shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(include_str!(
                "shader/text.wgsl"
            ))),
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: None,
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "main",
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: mem::size_of::<Instance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x3,
                        1 => Float32x2,
                        2 => Float32x2,
                        3 => Float32x2,
                        4 => Float32x4,
                    ],
                }],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                front_face: wgpu::FrontFace::Cw,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::GreaterEqual,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "main",
                targets: &[wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Bgra8Unorm,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                }],
            }),
        });

        Self {
            pipeline,
            bind_group,
            instances,
            pending_instances: Vec::new(),
            globals,
            size: Size::ZERO,
            cache,
        }
    }

    pub(crate) fn queue(&mut self, instances: &[Instance]) {
        self.pending_instances.extend_from_slice(instances);
    }

    pub(crate) fn draw(
        &mut self,
        device: &wgpu::Device,
        staging_belt: &mut wgpu::util::StagingBelt,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        depth_stencil_attachment: wgpu::RenderPassDepthStencilAttachment,
        translate: [f32; 2],
    ) {
        if self.pending_instances.len() == 0 {
            return;
        }

        {
            let globals = vec![Globals {
                resolution: [self.size.width as f32, self.size.height as f32],
                translate,
                scale: self.cache.scale as f32,
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
            let instances_bytes = bytemuck::cast_slice(&self.pending_instances);
            let mut instances = staging_belt.write_buffer(
                encoder,
                &self.instances,
                0,
                unsafe { NonZeroU64::new_unchecked(instances_bytes.len() as u64) },
                device,
            );
            instances.copy_from_slice(instances_bytes);
        }

        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("wgpu_glyph::pipeline render pass"),
            color_attachments: &[wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: true,
                },
            }],
            depth_stencil_attachment: Some(depth_stencil_attachment),
        });
        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, &self.bind_group, &[]);
        render_pass.set_vertex_buffer(0, self.instances.slice(..));
        render_pass.draw(0..4, 0..self.pending_instances.len() as u32);
        self.pending_instances.clear();
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, Default)]
struct GlyphInfo {
    font_id: usize,
    glyph_id: u32,
    font_size: u32,
}

#[derive(Default, Clone)]
pub(crate) struct GlyphPosInfo {
    info: GlyphInfo,
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
    fonts: FxHashMap<FontFamily, Font>,
    font_ids: FxHashMap<FontFamily, usize>,
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
        font_family: &FontFamily,
        font_size: f32,
        device: &wgpu::Device,
        staging_belt: &mut wgpu::util::StagingBelt,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Result<&GlyphPosInfo, piet::Error> {
        let scale = self.scale * 2.0;

        let font_size = (font_size as f64 * scale).round() as u32;
        let (font, font_id) = self.get_font(font_family)?;
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

        let (font, font_id) = self.get_font(font_family)?;
        let font_metrics = font.metrics();
        let units_per_em = font_metrics.units_per_em as f32;
        let glyph_width = font.advance(glyph_id).unwrap().x() / units_per_em * font_size as f32;
        let glyph_height = (font_metrics.ascent - font_metrics.descent + font_metrics.line_gap)
            / units_per_em
            * font_size as f32;
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

    fn get_font(&mut self, family: &FontFamily) -> Result<(&Font, usize), piet::Error> {
        if !self.fonts.contains_key(family) {
            let font = self.get_new_font(family)?;
            self.fonts.insert(family.clone(), font);
            self.font_ids.insert(family.clone(), self.font_ids.len());
        }
        Ok((
            self.fonts.get(family).unwrap(),
            *self.font_ids.get(family).unwrap(),
        ))
    }

    fn get_new_font(&self, family: &FontFamily) -> Result<Font, piet::Error> {
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
                    .weight(font_kit::properties::Weight::MEDIUM),
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

#[repr(C)]
#[derive(Debug, Clone, Copy, Zeroable, Pod)]
pub struct Instance {
    pub(crate) origin: [f32; 3],
    pub(crate) size: [f32; 2],
    pub(crate) tex_left_top: [f32; 2],
    pub(crate) tex_right_bottom: [f32; 2],
    pub(crate) color: [f32; 4],
}

impl Instance {
    const INITIAL_AMOUNT: usize = 50_000;
}
