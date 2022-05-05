#version 330

uniform float u_scale;
uniform float u_depth;
uniform mat4 view_proj;

layout(location = 0) in vec4 i_rect;
layout(location = 1) in vec4 i_tex_rect;
layout(location = 2) in vec4 i_color;
layout(location = 3) in float i_depth;
layout(location = 4) in vec4 i_clip;

out vec4 v_color;
out vec2 v_tex_pos;
out vec4 v_clip;

void main() {
    float left = i_rect.x;
    float right = i_rect.z;
    float top = i_rect.y;
    float bottom = i_rect.w;

    float tex_left = i_tex_rect.x;
    float tex_right = i_tex_rect.z;
    float tex_top = i_tex_rect.y;
    float tex_bottom = i_tex_rect.w;

    vec2 pos = vec2(0.0);
    vec2 tex_pos = vec2(0.0);
    switch (gl_VertexID) {
        case 0:
            pos = vec2(left, top);
            tex_pos = vec2(tex_left, tex_top);
            break;

        case 1:
            pos = vec2(right, top);
            tex_pos = vec2(tex_right, tex_top);
            break;

        case 2:
            pos = vec2(left, bottom);
            tex_pos = vec2(tex_left, tex_bottom);
            break;

        case 3:
            pos = vec2(right, bottom);
            tex_pos = vec2(tex_right, tex_bottom);
            break;
    }

    v_color = i_color;
    v_tex_pos = tex_pos;
    v_clip = vec4(
        i_clip.x * u_scale,
        i_clip.y * u_scale,
        i_clip.z * u_scale,
        i_clip.w * u_scale
    );

    gl_Position = view_proj * vec4(pos.x * u_scale, pos.y * u_scale, 0.0, 1.0);
    gl_Position.z = 1.0 - i_depth / u_depth;
}
