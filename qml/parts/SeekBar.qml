import QtQuick
import Snellar

Row {
    id: seek
    spacing: Theme.spaceMd

    property real shown: 0
    readonly property real live: visible ? Api.position : shown
    readonly property real span: Api.duration
    readonly property real pixels: Math.max(1, scrub.width * Screen.devicePixelRatio)

    function remain(t) { return Math.floor(Math.max(0, span - t)) }

    function settle(force) {
        if (force || Math.floor(live) !== Math.floor(shown) || (Api.timeLeft && remain(live) !== remain(shown))
                || (span > 0 && Math.abs(live - shown) / span * pixels >= Theme.scrubStepPx))
            shown = live
    }

    onLiveChanged: settle(false)
    onSpanChanged: settle(true)
    Component.onCompleted: settle(true)

    Text {
        width: Theme.timeWidth
        anchors.verticalCenter: parent.verticalCenter
        text: Theme.time(scrub.dragging ? scrub.dragValue * seek.span : seek.shown)
        color: Theme.text2
        font.pixelSize: Theme.fsCaption
        font.family: Theme.fontMono
    }
    Scrubber {
        id: scrub
        width: seek.width - 2 * (Theme.timeWidth + seek.spacing)
        anchors.verticalCenter: parent.verticalCenter
        value: seek.span > 0 ? seek.shown / seek.span : 0
        busy: Api.loading
        onCommitted: f => Api.seek(f * seek.span)
    }
    Text {
        width: Theme.timeWidth
        anchors.verticalCenter: parent.verticalCenter
        horizontalAlignment: Text.AlignRight
        text: Api.timeLeft ? "-" + Theme.time(seek.remain(seek.shown)) : Theme.time(seek.span)
        color: Theme.text2
        font.pixelSize: Theme.fsCaption
        font.family: Theme.fontMono

        Tap { gesturePolicy: TapHandler.DragThreshold; onTapped: Api.showTimeLeft(!Api.timeLeft) }
    }
}
