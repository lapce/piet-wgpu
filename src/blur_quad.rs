use glow::HasContext;

use crate::{context::BlurQuad, pipeline::create_program};

const MAX_INSTANCES: usize = 100_000;

pub struct Pipeline {
    program: <glow::Context as HasContext>::Program,
    vertex_array: <glow::Context as HasContext>::VertexArray,
    instances: <glow::Context as HasContext>::Buffer,
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
                    (glow::VERTEX_SHADER, include_str!("./shader/blur_quad.vert")),
                    (
                        glow::FRAGMENT_SHADER,
                        include_str!("./shader/blur_quad.frag"),
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

        let (vertex_array, instances) = unsafe { create_instance_buffer(gl, MAX_INSTANCES) };

        Self {
            vertex_array,
            instances,
            program,
            scale_location,
            depth_location,
            view_proj,
            current_scale: 1.0,
        }
    }

    pub fn draw(
        &mut self,
        gl: &glow::Context,
        instances: &[BlurQuad],
        scale: f32,
        view_proj: &[f32],
        max_depth: u32,
    ) {
        if instances.is_empty() {
            return;
        }

        unsafe {
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
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.instances));
            gl.buffer_sub_data_u8_slice(glow::ARRAY_BUFFER, 0, bytemuck::cast_slice(instances));
            gl.bind_buffer(glow::ARRAY_BUFFER, None);

            gl.draw_arrays_instanced(glow::TRIANGLE_STRIP, 0, 4, instances.len() as i32);
            gl.bind_vertex_array(None);
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
        (size * std::mem::size_of::<BlurQuad>()) as i32,
        glow::DYNAMIC_DRAW,
    );

    let stride = std::mem::size_of::<BlurQuad>() as i32;

    gl.enable_vertex_attrib_array(0);
    gl.vertex_attrib_pointer_f32(0, 4, glow::FLOAT, false, stride, 0);
    gl.vertex_attrib_divisor(0, 1);

    gl.enable_vertex_attrib_array(1);
    gl.vertex_attrib_pointer_f32(1, 4, glow::FLOAT, false, stride, 4 * 4);
    gl.vertex_attrib_divisor(1, 1);

    gl.enable_vertex_attrib_array(2);
    gl.vertex_attrib_pointer_f32(2, 1, glow::FLOAT, false, stride, 4 * (4 + 4));
    gl.vertex_attrib_divisor(2, 1);

    gl.enable_vertex_attrib_array(3);
    gl.vertex_attrib_pointer_f32(3, 4, glow::FLOAT, false, stride, 4 * (4 + 4 + 1));
    gl.vertex_attrib_divisor(3, 1);

    gl.enable_vertex_attrib_array(4);
    gl.vertex_attrib_pointer_f32(4, 1, glow::FLOAT, false, stride, 4 * (4 + 4 + 1 + 4));
    gl.vertex_attrib_divisor(4, 1);

    gl.enable_vertex_attrib_array(5);
    gl.vertex_attrib_pointer_f32(5, 4, glow::FLOAT, false, stride, 4 * (4 + 4 + 1 + 4 + 1));
    gl.vertex_attrib_divisor(5, 1);

    gl.bind_vertex_array(None);
    gl.bind_buffer(glow::ARRAY_BUFFER, None);

    (vertex_array, buffer)
}
