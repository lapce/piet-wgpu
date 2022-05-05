#version 410

in vec4 v_color;
in vec4 v_clip;
layout(origin_upper_left) in vec4 gl_FragCoord;

out vec4 o_color;

void main() {
    if (v_clip.z > 0.0 && v_clip.w > 0.0) {
        if (gl_FragCoord.x < v_clip.x || gl_FragCoord.x > v_clip.z || gl_FragCoord.y < v_clip.y || gl_FragCoord.y > v_clip.w) {
            discard;
        }
    }

    o_color = v_color;
}
