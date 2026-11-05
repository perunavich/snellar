#version 450

layout(location = 0) in vec2 qt_TexCoord0;
layout(location = 0) out vec4 fragColor;

layout(std140, binding = 0) uniform buf {
    mat4  qt_Matrix;
    float qt_Opacity;
    float wPx;
    float hPx;
    float radiusPx;
} ubuf;

layout(binding = 1) uniform sampler2D src;

float sdRoundBox(vec2 p, vec2 b, float r) {
    vec2 q = abs(p) - b + r;
    return min(max(q.x, q.y), 0.0) + length(max(q, vec2(0.0))) - r;
}

void main() {
    vec2 size = vec2(ubuf.wPx, ubuf.hPx);
    vec2 hs = size * 0.5;
    float r = min(ubuf.radiusPx, min(hs.x, hs.y));
    float d = sdRoundBox(qt_TexCoord0 * size - hs, hs, r);
    float a = clamp(0.5 - d, 0.0, 1.0);
    fragColor = texture(src, qt_TexCoord0) * a * ubuf.qt_Opacity;
}
