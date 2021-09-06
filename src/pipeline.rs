use lyon::lyon_tessellation::{
    BuffersBuilder, FillOptions, FillTessellator, FillVertex, FillVertexConstructor,
    StrokeVertexConstructor, VertexBuffers,
};
use lyon::tessellation;
use piet::kurbo::{Affine, Rect, Size};
use piet::Color;
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
    pub(crate) translate: [f32; 2],
    pub(crate) scale: [f32; 2],
    pub(crate) color: [f32; 4],
    pub(crate) normal: [f32; 2],
    pub(crate) width: f32,
    pub(crate) blur_rect: [f32; 4],
    pub(crate) blur_radius: f32,
}

unsafe impl bytemuck::Pod for GpuVertex {}
unsafe impl bytemuck::Zeroable for GpuVertex {}

impl Default for GpuVertex {
    fn default() -> Self {
        Self {
            pos: [0.0, 0.0],
            translate: [0.0, 0.0],
            scale: [1.0, 1.0],
            color: [0.0, 0.0, 0.0, 0.0],
            normal: [0.0, 0.0],
            width: 0.0,
            blur_rect: [0.0, 0.0, 0.0, 0.0],
            blur_radius: 0.0,
        }
    }
}

pub struct Pipeline {
    pub pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    globals: wgpu::Buffer,
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

        let shader = device.create_shader_module(&wgpu::ShaderModuleDescriptor {
            label: Some("iced_wgpu::quad::shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(include_str!(
                "shader/geometry.wgsl"
            ))),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Bind group layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(globals_buffer_byte_size),
                },
                count: None,
            }],
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Bind group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(globals.as_entire_buffer_binding()),
            }],
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
                        1 => Float32x2,
                        2 => Float32x2,
                        3 => Float32x4,
                        4 => Float32x2,
                        5 => Float32,
                        6 => Float32x4,
                        7 => Float32,
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
            size: Size::ZERO,
            scale: 1.0,
        }
    }

    pub fn draw(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        queue: &mut wgpu::Queue,
        view: &wgpu::TextureView,
        msaa: &wgpu::TextureView,
        geometry: &VertexBuffers<GpuVertex, u16>,
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
            pass.set_vertex_buffer(0, vbo.slice(..));
            pass.set_index_buffer(ibo.slice(..), wgpu::IndexFormat::Uint16);

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

//pub struct WithId(pub i32);
//
//impl FillVertexConstructor<GpuVertex> for WithId {
//    fn new_vertex(&mut self, vertex: tessellation::FillVertex) -> GpuVertex {
//        GpuVertex {
//            pos: vertex.position().to_array(),
//            normal: [0.0, 0.0],
//            prim_id: self.0,
//        }
//    }
//}
//
//impl StrokeVertexConstructor<GpuVertex> for WithId {
//    fn new_vertex(&mut self, vertex: tessellation::StrokeVertex) -> GpuVertex {
//        GpuVertex {
//            position: vertex.position_on_path().to_array(),
//            normal: vertex.normal().to_array(),
//            prim_id: self.0,
//        }
//    }
//}
