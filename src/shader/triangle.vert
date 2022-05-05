#version 330

uniform float u_scale;
uniform float u_depth;
uniform mat4 view_proj;

layout(location = 0) in vec2 i_pos;
layout(location = 1) in vec4 i_color;
layout(location = 2) in float i_depth;
layout(location = 3) in vec4 i_clip;

out vec4 v_color;
out vec4 v_clip;

void main() {
    v_color = i_color;
    v_clip = vec4(
        i_clip.x * u_scale,
        i_clip.y * u_scale,
        i_clip.z * u_scale,
        i_clip.w * u_scale
    );

    gl_Position = view_proj * vec4(i_pos.x * u_scale, i_pos.y * u_scale, 0.0, 1.0);
    gl_Position.z = 1.0 - i_depth / u_depth;
}
