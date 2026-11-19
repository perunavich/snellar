import QtQuick
import Snellar

NumberAnimation {
    property var curve: Theme.curveStandard
    easing.type: Easing.BezierSpline
    easing.bezierCurve: curve
}
