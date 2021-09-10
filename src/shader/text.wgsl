[[block]]
struct Globals {
    u_resolution: vec2<f32>;
    u_translate: vec2<f32>;
    u_scale: f32;
};

[[group(0), binding(0)]] var<uniform> globals: Globals;
[[group(0), binding(1)]] var font_sampler: sampler;
[[group(0), binding(2)]] var font_tex: texture_2d<f32>;

struct VertexInput {
    [[builtin(vertex_index)]] vertex_index: u32;
    [[location(0)]] origin: vec3<f32>;
    [[location(1)]] size: vec2<f32>;
    [[location(2)]] tex_left_top: vec2<f32>;
    [[location(3)]] tex_right_bottom: vec2<f32>;
    [[location(4)]] color: vec4<f32>;
};

struct VertexOutput {
    [[builtin(position)]] position: vec4<f32>;
    [[location(0)]] f_tex_pos: vec2<f32>;
    [[location(1)]] f_color: vec4<f32>;
};

[[stage(vertex)]]
fn main(input: VertexInput) -> VertexOutput {
    var out: VertexOutput;

    var pos: vec2<f32> = vec2<f32>(0.0, 0.0);
    var left: f32 = input.origin.x;
    var right: f32 = input.origin.x + input.size.x;
    var top: f32 = input.origin.y;
    var bottom: f32 = input.origin.y + input.size.y;

    switch (i32(input.vertex_index)) {
        case 0: {
            pos = vec2<f32>(left, top);
            out.f_tex_pos = input.tex_left_top;
        }
        case 1: {
            pos = vec2<f32>(right, top);
            out.f_tex_pos = vec2<f32>(input.tex_right_bottom.x, input.tex_left_top.y);
        }
        case 2: {
            pos = vec2<f32>(left, bottom);
            out.f_tex_pos = vec2<f32>(input.tex_left_top.x, input.tex_right_bottom.y);
        }
        case 3: {
            pos = vec2<f32>(right, bottom);
            out.f_tex_pos = input.tex_right_bottom;
        }
    }
    
    var invert_y: vec2<f32> = vec2<f32>(1.0, -1.0);
    
    out.f_color = input.color;
    out.position = vec4<f32>(((pos + globals.u_translate) / globals.u_resolution * globals.u_scale * 2.0 - vec2<f32>(1.0, 1.0)) * invert_y, input.origin.z, 1.0);

    return out;
}

[[stage(fragment)]]
fn main(input: VertexOutput) -> [[location(0)]] vec4<f32> {
    var alpha: f32 = textureSample(font_tex, font_sampler, input.f_tex_pos).r;

    if (alpha <= 0.0) {
        discard;
    }

    return input.f_color * vec4<f32>(1.0, 1.0, 1.0, alpha);
}