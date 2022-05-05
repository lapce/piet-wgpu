#version 410

in vec4 v_color;
in vec2 v_tex_pos;
in vec4 v_clip;
layout(origin_upper_left) in vec4 gl_FragCoord;

layout(location = 0, index = 0) out vec4 o_frag_color;
layout(location = 0, index = 1) out vec4 o_frag_alpha;

uniform sampler2D tex;

float luma(vec4 color) {
    return color.x * 0.25 + color.y * 0.72 + color.z * 0.075;
}

float gamma_correct(float luma, float alpha, float gamma, float contrast) {
    float inverse_luma = 1.0 - luma;
    float inverse_alpha = 1.0 - alpha;
    float g = pow(luma * alpha + inverse_luma * inverse_alpha, gamma);
    float a = (g - inverse_luma) / (luma - inverse_luma);
    a = a + ((1.0 - a) * contrast * a);
    return clamp(a, 0.0, 1.0);
}

vec4 gamma_correct_subpx(vec4 color, vec4 mask) {
    float l = luma(color);
    float inverse_luma = 1.0 - l;
    float gamma = mix(1.0 / 1.2, 1.0 / 2.4, inverse_luma);
    float contrast = mix(0.1, 0.8, inverse_luma);
    return vec4(
        gamma_correct(l, mask.x * color.a, gamma, contrast),
        gamma_correct(l, mask.y * color.a, gamma, contrast),
        gamma_correct(l, mask.z * color.a, gamma, contrast),
        1.0
    );
}

void main() {
    if (v_clip.z > 0.0 && v_clip.w > 0.0) {
        if (gl_FragCoord.x < v_clip.x || gl_FragCoord.x > v_clip.z || gl_FragCoord.y < v_clip.y || gl_FragCoord.y > v_clip.w) {
            discard;
        }
    }
    vec4 frag = v_color;
    vec4 alpha = texture(tex, v_tex_pos);
    o_frag_color = vec4(v_color.xyz, 1.0);
    o_frag_alpha = alpha;
}
