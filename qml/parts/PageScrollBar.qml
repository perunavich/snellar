import QtQuick
import QtQuick.Controls
import Snellar

ScrollBar {
    id: bar
    implicitWidth: Theme.scrollBar
    contentItem: Rectangle {
        implicitWidth: Theme.scrollThumb
        radius: Theme.scrollThumb / 2
        color: Theme.text2
        opacity: bar.active ? 0.5 : 0
        Behavior on opacity { NumberAnimation { duration: Theme.normal } }
    }
}
