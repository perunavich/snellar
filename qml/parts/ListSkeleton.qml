import QtQuick
import Snellar

Column {
    spacing: Theme.spaceSm
    Repeater {
        model: 7
        delegate: Skeleton { width: parent.width; height: Theme.rowHeight }
    }
}
