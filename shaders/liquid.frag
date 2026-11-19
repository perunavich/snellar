#version 440
layout(location = 0) in vec2 coord;
layout(location = 0) out vec4 fragColor;
layout(std140, binding = 0) uniform buf {
    mat4 qt_Matrix;
    float qt_Opacity;
    vec2 itemSize;
    vec4 shape;
    vec4 radii;
    vec4 body;
    vec4 bodyFill;
    vec4 fill;
    vec4 edge;
    vec4 contentRect;
    float merge;
    float contentMix;
    float shadow;
    float soft;
    float bodyRadius;
    float bodyEdge;
    float feather;
};
layout(binding = 1) uniform sampler2D source;

float roundBox(vec2 p, vec2 half_, vec4 r) {
    float rr = p.x > 0.0 ? (p.y > 0.0 ? r.z : r.y) : (p.y > 0.0 ? r.w : r.x);
    rr = min(rr, min(half_.x, half_.y));
    vec2 q = abs(p) - half_ + rr;
    return min(max(q.x, q.y), 0.0) + length(max(q, 0.0)) - rr;
}

float smin(float a, float b, float k) {
    if (k <= 0.0) return min(a, b);
    float h = max(k - abs(a - b), 0.0) / k;
    return min(a, b) - h * h * k * 0.25;
}

float rectField(vec2 p) {
    vec2 half_ = max(shape.zw * 0.5, vec2(0.0));
    if (half_.x <= 0.0 || half_.y <= 0.0) return 1e5;
    return roundBox(p - (shape.xy + half_), half_, radii);
}

float bodyField(vec2 p) {
    if (body.z <= 0.5 || body.w <= 0.5) return 1e5;
    return roundBox(p - body.xy, body.zw, vec4(bodyRadius));
}

vec4 premul(vec4 c) { return vec4(c.rgb * c.a, c.a); }

void main() {
    vec2 p = coord * itemSize;
    float dr = rectField(p);
    float db = bodyField(p);
    float d = smin(dr, db, merge);
    float aa = max(fwidth(d), 0.35);
    float cover = smoothstep(0.5, -0.5, d / aa);
    float wb = body.z > 0.5 ? smoothstep(-1.0, 1.0, (dr - db) / max(merge * 0.35, 1.0)) : 0.0;
    vec4 base = mix(premul(fill), premul(bodyFill), wb);
    if (contentMix > 0.0) {
        vec2 uv = (p - contentRect.xy) / max(contentRect.zw, vec2(1.0));
        if (uv.x >= 0.0 && uv.y >= 0.0 && uv.x <= 1.0 && uv.y <= 1.0) {
            vec2 e = min(p - contentRect.xy, contentRect.xy + contentRect.zw - p);
            float edgeFade = smoothstep(0.0, max(feather, 1.0), min(e.x, e.y));
            float bodyFade = body.z > 0.5 ? smoothstep(0.0, max(feather, 4.0), db) : 1.0;
            vec4 c = texture(source, uv) * contentMix * (1.0 - wb) * edgeFade * bodyFade;
            base = base * (1.0 - c.a) + c;
        }
    }
    float rimA = mix(edge.a, bodyEdge, wb);
    float rim = (1.0 - smoothstep(0.0, 1.0, abs(d / aa + 0.75))) * rimA;
    base = mix(base, vec4(edge.rgb, 1.0) * base.a, rim);
    float s = max(soft, 1.0);
    float ds = rectField(p - vec2(0.0, s * 0.4));
    float sh = shadow * exp(-max(ds, 0.0) / s) * smoothstep(-s, 0.0, ds + s * 0.5);
    if (body.z > 0.5) sh *= smoothstep(0.0, 2.0 * s, db);
    fragColor = (base * cover + vec4(0.0, 0.0, 0.0, sh) * (1.0 - cover)) * qt_Opacity;
}
