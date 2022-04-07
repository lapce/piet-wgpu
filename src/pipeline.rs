use glow::HasContext;

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

pub unsafe fn create_program(
    gl: &glow::Context,
    shader_sources: &[(u32, &str)],
) -> <glow::Context as HasContext>::Program {
    let program = gl.create_program().expect("Cannot create program");

    let mut shaders = Vec::with_capacity(shader_sources.len());

    for (shader_type, shader_source) in shader_sources.iter() {
        let shader = gl
            .create_shader(*shader_type)
            .expect("Cannot create shader");

        gl.shader_source(shader, shader_source);
        gl.compile_shader(shader);

        if !gl.get_shader_compile_status(shader) {
            panic!("{}", gl.get_shader_info_log(shader));
        }

        gl.attach_shader(program, shader);

        shaders.push(shader);
    }

    gl.link_program(program);
    if !gl.get_program_link_status(program) {
        panic!("{}", gl.get_program_info_log(program));
    }

    for shader in shaders {
        gl.detach_shader(program, shader);
        gl.delete_shader(shader);
    }

    program
}
