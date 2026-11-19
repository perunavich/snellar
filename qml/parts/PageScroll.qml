import QtQuick
import QtQuick.Controls
import Snellar

Flickable {
    id: page
    default property alias content: col.data
    property alias spacing: col.spacing
    property real padding: Theme.space2xl
    property real headroom: Theme.navBar
    readonly property real innerWidth: width - 2 * padding

    contentWidth: width
    contentHeight: col.height + headroom + padding
    boundsBehavior: Flickable.StopAtBounds
    clip: true
    flickableDirection: Flickable.VerticalFlick
    bottomMargin: Nav.floor

    ScrollBar.vertical: PageScrollBar { }

    Column {
        id: col
        x: page.padding
        y: page.headroom
        width: page.innerWidth
        spacing: Theme.sectionGap
    }
}
