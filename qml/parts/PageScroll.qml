import QtQuick
import QtQuick.Controls
import Snellar

Flickable {
    id: page
    default property alias content: col.data
    property alias spacing: col.spacing
    property real padding: Theme.space2xl
    readonly property real innerWidth: width - 2 * padding

    contentWidth: width
    contentHeight: col.height + 2 * padding
    boundsBehavior: Flickable.StopAtBounds
    clip: true
    flickableDirection: Flickable.VerticalFlick

    ScrollBar.vertical: PageScrollBar { }

    Column {
        id: col
        x: page.padding
        y: page.padding
        width: page.innerWidth
        spacing: Theme.sectionGap
    }
}
