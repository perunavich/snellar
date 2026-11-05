import QtQuick
import Snellar

EmptyState {
    required property Sorter sorter
    visible: sorter.items.length > 0 && sorter.shown.length === 0
    icon: "search"
    text: qsTr("Nothing matches “%1”").arg(sorter.query.trim())
}
