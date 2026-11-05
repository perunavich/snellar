import QtQuick
import Snellar

Rectangle {
    id: skeleton
    radius: Theme.rMd
    color: Theme.fillRaised
    SequentialAnimation on opacity {
        running: skeleton.visible
        loops: Animation.Infinite
        NumberAnimation { from: 0.45; to: 0.9; duration: Theme.pulse; easing.type: Easing.InOutSine }
        NumberAnimation { from: 0.9; to: 0.45; duration: Theme.pulse; easing.type: Easing.InOutSine }
    }
}
