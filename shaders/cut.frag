#version 450

layout(location = 0) in vec2 qt_TexCoord0;
layout(location = 0) out vec4 fragColor;

layout(std140, binding = 0) uniform buf {
    mat4  qt_Matrix;
    float qt_Opacity;
    float cut;
    float amount;
    float floorAmount;
    float floorRadius;
    vec2 size;
    vec4 floorBox;
} ubuf;

layout(binding = 1) uniform sampler2D source;
layout(binding = 2) uniform sampler2D blurred;

void main() {
    float covered = step(qt_TexCoord0.y, ubuf.cut);
    vec2 p = qt_TexCoord0 * ubuf.size;
    vec2 centre = (ubuf.floorBox.xy + ubuf.floorBox.zw) * 0.5;
    vec2 extent = (ubuf.floorBox.zw - ubuf.floorBox.xy) * 0.5;
    vec2 q = abs(p - centre) - extent + vec2(ubuf.floorRadius);
    float d = length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - ubuf.floorRadius;
    float under = (1.0 - smoothstep(-0.5, 0.5, d)) * ubuf.floorAmount;
    vec4 page = mix(texture(source, qt_TexCoord0), texture(blurred, qt_TexCoord0), under);
    fragColor = page * (1.0 - covered * ubuf.amount) * ubuf.qt_Opacity;
}
