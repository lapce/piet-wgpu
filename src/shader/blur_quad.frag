#version 330

in vec4 v_color;
in vec2 v_pos;
in vec4 v_blur_rect;
in float v_blur_radius;
in vec4 v_clip;

layout(origin_upper_left) in vec4 gl_FragCoord;

out vec4 o_color;

vec4 erf(vec4 x) {
    vec4 s = sign(x);
    vec4 a = abs(x);
    vec4 r = 1.0 + (0.278393 + (0.230389 + 0.078108 * (a * a)) * a) * a; 
    r = r * r;
    return s - s / (r * r);
}

float box_shadow(vec2 lower, vec2 upper, vec2 point, float radius) {
    vec4 query = vec4(point - lower, point - upper);
    vec4 integral = 0.5 + 0.5 * erf(query * (sqrt(0.5) / radius));
    return (integral.z - integral.x) * (integral.w - integral.y);
}

void main() {
    if (v_clip.z > 0.0 && v_clip.w > 0.0) {
        if (gl_FragCoord.x < v_clip.x || gl_FragCoord.x > v_clip.z || gl_FragCoord.y < v_clip.y || gl_FragCoord.y > v_clip.w) {
            discard;
        }
    }

    if (v_blur_rect.x <= v_pos.x && v_pos.x <= v_blur_rect.z && v_blur_rect.y <= v_pos.y && v_pos.y <= v_blur_rect.w) {
        discard;
    }
    o_color = v_color;
    o_color.a = o_color.a * box_shadow(
        vec2(v_blur_rect.x, v_blur_rect.y),
        vec2(v_blur_rect.z, v_blur_rect.w),
        v_pos,
        v_blur_radius);
}
