import QtQuick
import Snellar

Row {
    id: seek
    spacing: Theme.spaceMd

    Text {
        width: Theme.timeWidth
        anchors.verticalCenter: parent.verticalCenter
        horizontalAlignment: Text.AlignRight
        text: Theme.time(scrub.dragging ? scrub.dragValue * Api.duration : Api.position)
        color: Theme.text2
        font.pixelSize: Theme.fsCaption
        font.family: Theme.fontMono
    }
    Scrubber {
        id: scrub
        width: seek.width - 2 * (Theme.timeWidth + seek.spacing)
        anchors.verticalCenter: parent.verticalCenter
        value: Api.duration > 0 ? Api.position / Api.duration : 0
        busy: Api.loading
        onCommitted: f => Api.seek(f * Api.duration)
    }
    Text {
        width: Theme.timeWidth
        anchors.verticalCenter: parent.verticalCenter
        text: Theme.time(Api.duration)
        color: Theme.text2
        font.pixelSize: Theme.fsCaption
        font.family: Theme.fontMono
    }
}
