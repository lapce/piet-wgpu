[[block]]
struct Globals {
    u_resolution: vec2<f32>;
    u_scale: f32;
};

[[group(0), binding(0)]] var<uniform> globals: Globals;
    
struct VertexInput {
    [[location(0)]] v_pos: vec2<f32>;
    [[location(1)]] v_translate: vec2<f32>;
    [[location(2)]] v_color: vec4<f32>;
    [[location(3)]] v_normal: vec2<f32>;
    [[location(4)]] v_width: f32;
    [[location(5)]] v_rect: vec4<f32>;
    [[location(6)]] v_blur_radius: f32;
};

struct VertexOutput {
    [[builtin(position)]] position: vec4<f32>;
    [[location(0)]] color: vec4<f32>;
    [[location(1)]] pos: vec2<f32>;
    [[location(2)]] rect: vec4<f32>;
    [[location(3)]] blur_radius: f32;
};

[[stage(vertex)]]
fn main(input: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    
    var invert_y: vec2<f32> = vec2<f32>(1.0, -1.0);
    
    var translated_pos: vec2<f32> = (input.v_pos + input.v_translate) * globals.u_scale;
    
    if (input.v_width > 0.0) {
        translated_pos = (input.v_pos + input.v_translate + input.v_normal / 2.0 * input.v_width) * globals.u_scale;
    }
    
    var pos: vec2<f32> = (translated_pos / globals.u_resolution * 2.0 - vec2<f32>(1.0, 1.0)) * invert_y;
    
    out.position = vec4<f32>(pos, 0.0, 1.0);
    out.color = input.v_color;
    out.blur_radius = input.v_blur_radius;
    
    if (out.blur_radius > 0.0) {
        out.rect.x = (input.v_rect.x + input.v_translate.x) * globals.u_scale / globals.u_resolution.x * 2.0 - 1.0;
        out.rect.y = ((input.v_rect.y + input.v_translate.y) * globals.u_scale / globals.u_resolution.y * 2.0 - 1.0) * -1.0;
        out.rect.z = (input.v_rect.z + input.v_translate.x) * globals.u_scale / globals.u_resolution.x * 2.0 - 1.0;
        out.rect.w = ((input.v_rect.w + input.v_translate.y) * globals.u_scale / globals.u_resolution.y * 2.0 - 1.0) * -1.0;;
    }
    
    out.rect = input.v_rect;
    out.pos = input.v_pos;
    
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
    return color;
}
