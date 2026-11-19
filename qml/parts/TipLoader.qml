pragma ComponentBehavior: Bound
import QtQuick
import Snellar

Loader {
    id: host
    property bool want: false
    property string hint
    property bool cut: false
    property bool fading: false
    readonly property Tip tip: item as Tip

    anchors.fill: parent
    active: want || fading
    sourceComponent: Tip {
        text: host.hint
        want: host.want
        cut: host.cut
        onFadingChanged: host.fading = fading
    }
}
