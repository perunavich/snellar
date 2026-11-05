import QtQuick
import QtQuick.Controls
import Snellar

ListView {
    id: list
    readonly property string artistId: Api.current && Api.current.artists.length ? Api.current.artists[0].id : ""

    Fetch { id: res; method: "artist"; args: ({ id: list.artistId }); ready: list.artistId.length > 0 && list.visible }

    model: res.data ? res.data.related : []

    ErrorState {
        parent: list
        x: Theme.spaceLg
        y: Theme.spaceXl
        width: list.width - 2 * Theme.spaceLg
        fetch: res
    }
    clip: true
    spacing: Theme.spaceXs
    boundsBehavior: Flickable.StopAtBounds
    ScrollBar.vertical: ScrollBar { }

    delegate: Item {
        id: row
        required property var modelData
        width: ListView.view.width
        height: Theme.rowHeight

        Rectangle {
            anchors.fill: parent
            radius: Theme.rRow
            color: hover.hovered ? Theme.fillHover : "transparent"
        }
        Cover {
            id: art
            x: Theme.spaceSm
            anchors.verticalCenter: parent.verticalCenter
            width: Theme.rowCover
            height: Theme.rowCover
            radius: width / 2
            px: 96
            url: row.modelData.thumb
        }
        Column {
            anchors { left: art.right; leftMargin: Theme.spaceMd; right: parent.right; rightMargin: Theme.spaceSm; verticalCenter: parent.verticalCenter }
            Text { width: parent.width; text: row.modelData.name; color: Theme.text; font.pixelSize: Theme.fsBody; elide: Text.ElideRight }
            Text { width: parent.width; text: Words.subscribers(row.modelData.subtitle); color: Theme.text2; font.pixelSize: Theme.fsCaption; elide: Text.ElideRight }
        }
        HoverHandler { id: hover; cursorShape: Qt.PointingHandCursor }
        TapHandler { onTapped: Nav.artist(row.modelData.id) }
    }
}
