pragma ComponentBehavior: Bound
import QtQuick
import Snellar

Item {
    id: view
    readonly property var track: res.data
    readonly property var lines: track ? (track.synced ? track.lines : track.plain.split("\n").map(text => ({ time: -1, text }))) : []
    readonly property bool synced: !!track && track.synced
    readonly property int active: {
        if (!synced) return -1
        let a = -1
        for (let i = 0; i < lines.length && lines[i].time <= Api.position; i++) a = i
        return a
    }
    property real browse: 0
    property int lineSize: Theme.fsLyric

    clip: true

    Fetch { id: res; method: "lyrics"; args: ({ track: Api.current }); ready: !!Api.current && view.visible }

    SpringValue {
        id: scroll
        zeta: 1.0
        response: 0.6
        target: view.focusY + view.browse
        epsilon: 0.5
    }

    readonly property real focusY: {
        const row = view.active >= 0 ? repeater.itemAt(view.active) : null
        return row ? Math.max(0, row.y - height * 0.25) : 0
    }

    onFocusYChanged: if (!visible) scroll.snap(focusY + browse)

    Timer { id: settle; interval: Theme.lyricSettle; onTriggered: view.browse = 0 }

    readonly property string trackId: Api.current ? Api.current.id : ""
    onTrackIdChanged: {
        settle.stop()
        browse = 0
        scroll.snap(0)
    }

    WheelHandler {
        onWheel: event => {
            const max = column.height - view.height
            view.browse = Math.max(-view.focusY, Math.min(max - view.focusY, view.browse - event.angleDelta.y))
            settle.restart()
        }
    }

    Column {
        id: column
        y: -scroll.value
        width: parent.width
        spacing: view.lineSize * 0.5
        visible: view.lines.length > 0

        Repeater {
            id: repeater
            model: view.lines
            delegate: Text {
                id: line
                required property var modelData
                required property int index
                readonly property int dist: view.synced ? Math.abs(index - view.active) : 0
                width: column.width
                text: modelData.text
                wrapMode: Text.WordWrap
                color: Theme.text
                font.family: Theme.font
                font.pixelSize: view.lineSize
                font.weight: Font.Bold
                opacity: !view.synced ? 0.85 : (dist === 0 ? 1 : Math.max(Theme.lyricDim, 0.9 - dist * 0.1))
                scale: !view.synced || dist === 0 ? 1 : 0.9
                transformOrigin: Item.Left
                Behavior on opacity { NumberAnimation { duration: Theme.m3; easing.type: Easing.BezierSpline; easing.bezierCurve: Theme.curveStandard } }
                Behavior on scale { NumberAnimation { duration: Theme.m3; easing.type: Easing.BezierSpline; easing.bezierCurve: Theme.curveJelly } }

                MouseArea {
                    anchors.fill: parent
                    enabled: view.synced
                    cursorShape: Qt.PointingHandCursor
                    onClicked: Api.seek(line.modelData.time)
                }
            }
        }
    }

    ErrorState {
        anchors.centerIn: parent
        width: parent.width - 2 * Theme.spaceXl
        fetch: res
    }

    Column {
        anchors.centerIn: parent
        spacing: Theme.spaceXl
        visible: res.loading && !view.track
        Repeater {
            model: 4
            delegate: Skeleton { required property int index; width: Theme.lyricSkeleton - index * Theme.controlMd; height: Theme.fsLyric + Theme.spaceXs; radius: height / 2 }
        }
    }

    Text {
        anchors.centerIn: parent
        width: parent.width - 2 * Theme.spaceMd
        horizontalAlignment: Text.AlignHCenter
        wrapMode: Text.WordWrap
        visible: !res.loading && !res.waiting && !res.code && view.lines.length === 0
        text: qsTr("No lyrics for this song")
        color: Theme.text2
        font.family: Theme.font
        font.pixelSize: Theme.fsLargeTitle
    }
}
