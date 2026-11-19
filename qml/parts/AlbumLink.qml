import QtQuick
import Snellar

Item {
    id: link
    property var track
    property alias gesturePolicy: tap.gesturePolicy
    readonly property string album: Api.albumOf(track)
    anchors.fill: parent

    Tap { id: tap; enabled: link.album.length > 0; onTapped: Nav.album(link.album) }
    HoverHandler { enabled: link.album.length > 0; cursorShape: Qt.PointingHandCursor }
}
