import QtQuick

QtObject {
    property real x: 0
    property real v: 0
    property real w: 2 * Math.PI / 0.6
    property real k: w * w
    property real c: 2 * 1.0 * w

    function set(zeta, response) {
        w = 2 * Math.PI / response;
        k = w * w;
        c = 2 * zeta * w;
    }
    function step(target, dt) {
        const n = Math.max(1, Math.ceil(dt / 0.002)), h = dt / n;
        let x0 = x, v0 = v;
        for (let i = 0; i < n; i++) {
            const a = -k * (x0 - target) - c * v0;
            v0 += a * h;
            x0 += v0 * h;
        }
        x = x0;
        v = v0;
    }
    function done(target, eps) {
        const e = eps || 0.003;
        return Math.abs(x - target) < e && Math.abs(v) < e * 15;
    }
    function snap(target) { x = target; v = 0; }
}
