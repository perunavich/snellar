#version 450

layout(location = 0) in vec2 qt_TexCoord0;
layout(location = 0) out vec4 fragColor;

layout(std140, binding = 0) uniform buf {
    mat4  qt_Matrix;
    float qt_Opacity;
    float wPx;
    float hPx;
    float radiusPx;
    vec4  tint;
    vec4  edgeTop;
    vec4  edgeBottom;
    vec4  hairline;
} ubuf;

float sdRoundBox(vec2 p, vec2 b, float r) {
    vec2 q = abs(p) - b + r;
    return min(max(q.x, q.y), 0.0) + length(max(q, vec2(0.0))) - r;
}

void main() {
    vec2  size = vec2(ubuf.wPx, ubuf.hPx);
    vec2  hs = size * 0.5;
    vec2  p    = qt_TexCoord0 * size - hs;
    float r    = min(ubuf.radiusPx, min(hs.x, hs.y));

    float d = sdRoundBox(p, hs, r);

    float inside = 1.0 - smoothstep(-0.7, 0.7, d);
    if (inside <= 0.0) {
        fragColor = vec4(0.0);
        return;
    }

    const float h = 1.0;
    float dx = sdRoundBox(p + vec2(h, 0.0), hs, r) - sdRoundBox(p - vec2(h, 0.0), hs, r);
    float dy = sdRoundBox(p + vec2(0.0, h), hs, r) - sdRoundBox(p - vec2(0.0, h), hs, r);
    vec2  n  = normalize(vec2(dx, dy) + vec2(1e-6));

    float band = (1.0 - smoothstep(0.0, 1.6, abs(d + 0.8))) * inside;

    float up   = clamp(-n.y, 0.0, 1.0);
    float down = clamp( n.y, 0.0, 1.0);

    vec3  rgb = ubuf.tint.rgb;
    float a   = ubuf.tint.a;

    float wHair = band * ubuf.hairline.a;
    rgb = mix(rgb, ubuf.hairline.rgb, wHair);
    a   = a + (1.0 - a) * wHair;

    float wTop = band * up * up * ubuf.edgeTop.a;
    rgb = mix(rgb, ubuf.edgeTop.rgb, wTop);
    a   = a + (1.0 - a) * wTop;

    float wBot = band * down * down * ubuf.edgeBottom.a;
    rgb = mix(rgb, ubuf.edgeBottom.rgb, wBot);
    a   = a + (1.0 - a) * wBot;

    a *= inside * ubuf.qt_Opacity;
    fragColor = vec4(rgb * a, a);
}
