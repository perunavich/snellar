import QtQuick
import Snellar

SpringValue {
    id: lens
    property bool down: false
    property real w: 0
    property real h: 0
    property bool squeezing: false

    reducible: true
    target: 1
    zeta: Motion.trackingDamping
    response: Motion.trackingResponseFor(w, h)

    onDownChanged: down ? press() : letGo()

    function press() {
        squeezing = false
        if (Motion.reduced) return snap(1)
        zeta = Motion.trackingDamping
        response = Motion.trackingResponseFor(w, h)
        target = Motion.pressScaleFor(w, h)
    }

    function letGo() {
        zeta = Motion.pressDampingFor(w, h)
        response = Motion.pressResponseFor(w, h)
        if (Motion.reduced) return snap(1)
        squeezing = value > Motion.releaseScaleFor(w, h) + Motion.releaseSlack
        target = squeezing ? Motion.releaseScaleFor(w, h) : 1
    }

    onValueChanged: if (squeezing && value <= Motion.releaseScaleFor(w, h) + Motion.releaseSlack) {
        squeezing = false
        target = 1
    }
}
