struct Primitive {
    u_clip_rect: vec4<f32>;
    u_transform_1: vec4<f32>;
    u_blur_rect: vec4<f32>;
    u_transform_2: vec2<f32>;
    u_normal: vec2<f32>;
    u_translate: vec2<f32>;
    u_scale: vec2<f32>;
    u_width: f32;
    u_clip: f32;
    u_blur_radius: f32;
};

[[block]]
struct Globals {
    u_resolution: vec2<f32>;
    u_scale: f32;
};

[[block]]
struct Primitives {
    data: [[stride(96)]] array<Primitive>;
};

[[group(0), binding(0)]] var<uniform> globals: Globals;
[[group(0), binding(1)]] var font_sampler: sampler;
[[group(0), binding(2)]] var font_tex: texture_2d<f32>;
[[group(0), binding(3)]] var<storage> primitives: Primitives;
    
struct VertexInput {
    [[location(0)]] v_pos: vec2<f32>;
    [[location(1)]] v_translate: vec2<f32>;
    [[location(2)]] v_color: vec4<f32>;
    [[location(3)]] v_tex: f32;
    [[location(4)]] v_tex_pos: vec2<f32>;
    [[location(5)]] v_primitive_id: u32;
};

struct VertexOutput {
    [[builtin(position)]] position: vec4<f32>;
    [[location(0)]] color: vec4<f32>;
    [[location(1)]] pos: vec2<f32>;
    [[location(2)]] rect: vec4<f32>;
    [[location(3)]] blur_radius: f32;
    [[location(4)]] tex: f32;
    [[location(5)]] tex_pos: vec2<f32>;
    [[location(6)]] clip: f32;
    [[location(7)]] clip_rect: vec4<f32>;
};

[[stage(vertex)]]
fn main(input: VertexInput) -> VertexOutput {
    var primitive = primitives.data[input.v_primitive_id];

    var out: VertexOutput;
    
    var invert_y: vec2<f32> = vec2<f32>(1.0, -1.0);

    let transform = mat3x3<f32>(
        vec3<f32>(primitive.u_transform_1.x, primitive.u_transform_1.y, 0.0),
        vec3<f32>(primitive.u_transform_1.z, primitive.u_transform_1.w, 0.0),
        vec3<f32>(primitive.u_transform_2.x, primitive.u_transform_2.y, 1.0),
    );

    var transformed_pos = transform * vec3<f32>(input.v_pos.x, input.v_pos.y, 1.0);

    var v_pos: vec2<f32> = vec2<f32>(transformed_pos.x, transformed_pos.y);
    
    var translated_pos: vec2<f32> = (v_pos * primitive.u_scale + primitive.u_translate + input.v_translate) * globals.u_scale;
    
    if (primitive.u_width > 0.0) {
        translated_pos = (v_pos * primitive.u_scale + primitive.u_translate + input.v_translate + primitive.u_normal / 2.0 * primitive.u_width) * globals.u_scale;
    }
    
    var pos: vec2<f32> = (translated_pos / globals.u_resolution * 2.0 - vec2<f32>(1.0, 1.0)) * invert_y;
    
    out.position = vec4<f32>(pos, 0.0, 1.0);
    out.color = input.v_color;
    out.blur_radius = primitive.u_blur_radius;
    out.rect = primitive.u_blur_rect;
    out.pos = input.v_pos;
    out.tex = input.v_tex;
    out.tex_pos = input.v_tex_pos;
    out.clip = primitive.u_clip;
    out.clip_rect = primitive.u_clip_rect;
    
    if (out.clip > 0.0) {
        var left_top = vec2<f32>(primitive.u_clip_rect.x, primitive.u_clip_rect.y);
        var left_top = left_top * globals.u_scale;
        
        var right_bottom = vec2<f32>(primitive.u_clip_rect.z, primitive.u_clip_rect.w);
        var right_bottom = right_bottom * globals.u_scale;
        out.clip_rect = vec4<f32>(left_top, right_bottom);
    }
    
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
    
    if (input.clip > 0.0) {
        if (input.position.x < input.clip_rect.x || input.position.x > input.clip_rect.z || input.position.y < input.clip_rect.y || input.position.y > input.clip_rect.w) {
            discard;
        }
    }
    
    return color;
}
