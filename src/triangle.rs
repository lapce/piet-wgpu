use std::marker::PhantomData;

use glow::HasContext;
use lyon::lyon_tessellation::VertexBuffers;

use crate::{context::Vertex, pipeline::create_program};

const VERTEX_BUFFER_SIZE: usize = 10_000;
const INDEX_BUFFER_SIZE: usize = 10_000;

pub struct Pipeline {
    program: <glow::Context as HasContext>::Program,
    vertex_array: <glow::Context as HasContext>::VertexArray,
    vertices: Buffer<Vertex>,
    indices: Buffer<u32>,
    scale_location: <glow::Context as HasContext>::UniformLocation,
    view_proj: <glow::Context as HasContext>::UniformLocation,
    depth_location: <glow::Context as HasContext>::UniformLocation,
    current_scale: f32,
}

impl Pipeline {
    pub fn new(gl: &glow::Context) -> Self {
        let program = unsafe {
            create_program(
                gl,
                &[
                    (glow::VERTEX_SHADER, include_str!("./shader/triangle.vert")),
                    (
                        glow::FRAGMENT_SHADER,
                        include_str!("./shader/triangle.frag"),
                    ),
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

        let vertex_array = unsafe { gl.create_vertex_array().expect("Create vertex array") };

        unsafe {
            gl.bind_vertex_array(Some(vertex_array));
        }

        let vertices = unsafe {
            Buffer::new(
                gl,
                glow::ARRAY_BUFFER,
                glow::DYNAMIC_DRAW,
                VERTEX_BUFFER_SIZE,
            )
        };

        let indices = unsafe {
            Buffer::new(
                gl,
                glow::ELEMENT_ARRAY_BUFFER,
                glow::DYNAMIC_DRAW,
                INDEX_BUFFER_SIZE,
            )
        };

        unsafe {
            let stride = std::mem::size_of::<Vertex>() as i32;

            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, stride, 0);

            gl.enable_vertex_attrib_array(1);
            gl.vertex_attrib_pointer_f32(1, 4, glow::FLOAT, false, stride, 4 * 2);

            gl.enable_vertex_attrib_array(2);
            gl.vertex_attrib_pointer_f32(2, 1, glow::FLOAT, false, stride, 4 * (2 + 4));

            gl.enable_vertex_attrib_array(3);
            gl.vertex_attrib_pointer_f32(3, 4, glow::FLOAT, false, stride, 4 * (2 + 4 + 1));

            gl.bind_vertex_array(None);
        }

        Self {
            program,
            scale_location,
            depth_location,
            view_proj,
            vertex_array,
            vertices,
            indices,
            current_scale: 1.0,
        }
    }

    pub fn draw(
        &mut self,
        gl: &glow::Context,
        triangles: &VertexBuffers<Vertex, u32>,
        scale: f32,
        view_proj: &[f32],
        max_depth: u32,
    ) {
        if triangles.vertices.is_empty() {
            return;
        }

        unsafe {
            gl.enable(glow::MULTISAMPLE);
            gl.use_program(Some(self.program));
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
            self.vertices.bind(gl, triangles.vertices.len());
            self.indices.bind(gl, triangles.indices.len());
        }

        unsafe {
            gl.buffer_sub_data_u8_slice(
                glow::ARRAY_BUFFER,
                0,
                bytemuck::cast_slice(&triangles.vertices),
            );

            gl.buffer_sub_data_u8_slice(
                glow::ELEMENT_ARRAY_BUFFER,
                0,
                bytemuck::cast_slice(&triangles.indices),
            );

            gl.draw_elements(
                glow::TRIANGLES,
                triangles.indices.len() as i32,
                glow::UNSIGNED_INT,
                0,
            );

            gl.bind_vertex_array(None);
            gl.use_program(None);
            gl.disable(glow::MULTISAMPLE);
        }
    }
}

#[derive(Debug)]
struct Buffer<T> {
    raw: <glow::Context as HasContext>::Buffer,
    target: u32,
    usage: u32,
    size: usize,
    phantom: PhantomData<T>,
}

impl<T> Buffer<T> {
    pub unsafe fn new(gl: &glow::Context, target: u32, usage: u32, size: usize) -> Self {
        let raw = gl.create_buffer().expect("Create buffer");

        let mut buffer = Buffer {
            raw,
            target,
            usage,
            size: 0,
            phantom: PhantomData,
        };

        buffer.bind(gl, size);

        buffer
    }

    pub unsafe fn bind(&mut self, gl: &glow::Context, size: usize) {
        gl.bind_buffer(self.target, Some(self.raw));

        if self.size < size {
            gl.buffer_data_size(
                self.target,
                (size * std::mem::size_of::<T>()) as i32,
                self.usage,
            );

            self.size = size;
        }
    }
}
