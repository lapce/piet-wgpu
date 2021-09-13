[[block]]
struct Globals {
    u_resolution: vec2<f32>;
    u_scale: f32;
};

[[group(0), binding(0)]] var<uniform> globals: Globals;
[[group(0), binding(1)]] var font_sampler: sampler;
[[group(0), binding(2)]] var font_tex: texture_2d<f32>;
    
struct VertexInput {
    [[location(0)]] v_pos: vec2<f32>;
    [[location(1)]] v_z: f32;
    [[location(2)]] v_translate: vec2<f32>;
    [[location(3)]] v_scale: vec2<f32>;
    [[location(4)]] v_color: vec4<f32>;
    [[location(5)]] v_normal: vec2<f32>;
    [[location(6)]] v_width: f32;
    [[location(7)]] v_rect: vec4<f32>;
    [[location(8)]] v_blur_radius: f32;
    [[location(9)]] v_tex: f32;
    [[location(10)]] v_tex_pos: vec2<f32>;
    [[location(11)]] v_clip: f32;
    [[location(12)]] v_clip_rect: vec4<f32>;
    [[location(13)]] v_transform_1: vec4<f32>;
    [[location(14)]] v_transform_2: vec2<f32>;
};

struct VertexOutput {
    [[builtin(position)]] position: vec4<f32>;
    [[location(0)]] color: vec4<f32>;
    [[location(1)]] pos: vec2<f32>;
    [[location(2)]] rect: vec4<f32>;
    [[location(3)]] blur_radius: f32;
    [[location(4)]] tex: f32;
    [[location(5)]] tex_pos: vec2<f32>;
};

[[stage(vertex)]]
fn main(input: VertexInput) -> VertexOutput {

    var out: VertexOutput;
    
    var invert_y: vec2<f32> = vec2<f32>(1.0, -1.0);

    let transform = mat3x3<f32>(
        vec3<f32>(input.v_transform_1.x, input.v_transform_1.y, 0.0),
        vec3<f32>(input.v_transform_1.z, input.v_transform_1.w, 0.0),
        vec3<f32>(input.v_transform_2.x, input.v_transform_2.y, 1.0),
    );

    var transformed_pos = transform * vec3<f32>(input.v_pos.x, input.v_pos.y, 1.0);

    var v_pos: vec2<f32> = vec2<f32>(transformed_pos.x, transformed_pos.y);
    
    var translated_pos: vec2<f32> = (v_pos * input.v_scale + input.v_translate) * globals.u_scale;
    
    if (input.v_width > 0.0) {
        translated_pos = (v_pos * input.v_scale + input.v_translate + input.v_normal / 2.0 * input.v_width) * globals.u_scale;
    }
    
    var pos: vec2<f32> = (translated_pos / globals.u_resolution * 2.0 - vec2<f32>(1.0, 1.0)) * invert_y;
    
    out.position = vec4<f32>(pos, input.v_z, 1.0);
    out.color = input.v_color;
    out.blur_radius = input.v_blur_radius;
    out.rect = input.v_rect;
    out.pos = input.v_pos;
    out.tex = input.v_tex;
    out.tex_pos = input.v_tex_pos;
    
    return out;
}

fn erf(x: vec4<f32>) -> vec4<f32> {
    var s: vec4<f32> = sign(x);
    var a: vec4<f32> = abs(x);
    var r: vec4<f32> = 1.0 + (0.278393 + (0.230389 + 0.078108 * (a * a)) * a) * a; 
    r = r * r;
    return s - s / (r * r);
}

fn box_shadow(lower: vec2<f32>, upper: vec2<f32>, point: vec2<f32>, radius: f32) -> f32 {
    var query: vec4<f32> = vec4<f32>(point - lower, point - upper);
    var integral: vec4<f32> = 0.5 + 0.5 * erf(query * (sqrt(0.5) / radius));
    return (integral.z - integral.x) * (integral.w - integral.y);
}

[[stage(fragment)]]
fn main(input: VertexOutput) -> [[location(0)]] vec4<f32> {
    var color: vec4<f32> = input.color;
    if (input.blur_radius > 0.0) {
        color.w = color.w * box_shadow(
           vec2<f32>(input.rect.x, input.rect.y),
           vec2<f32>(input.rect.z, input.rect.w),
           vec2<f32>(input.pos.x, input.pos.y),
           input.blur_radius
        );
    }

    var alpha: f32 = textureSample(font_tex, font_sampler, input.tex_pos).r;
    if (input.tex > 0.0) {
        if (alpha <= 0.0) {
            discard;
        }
        color.w = color.w * alpha;
    }
    return color;
}
