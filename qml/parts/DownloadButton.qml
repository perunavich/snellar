import QtQuick
import Snellar

ActionButton {
    id: button
    property string kind: "playlist"
    property var entity
    readonly property var kept: entity ? Api.collection(kind, entity.id) : null
    readonly property string phase: Api.collectionPhase(kept)

    visible: !!entity
    icon: phase === "done" ? "downloaded" : "download"
    on: phase !== "off"
    progress: phase === "off" || phase === "done" ? -1 : kept.total > 0 ? kept.done / kept.total : 0
    stalled: phase === "waiting" || phase === "full" || phase === "tool"
    text: {
        if (kept && hovered) return qsTr("Remove download")
        switch (phase) {
        case "off": return qsTr("Download")
        case "done": return kept.refused > 0 ? qsTr("Downloaded, %n unavailable", "", kept.refused) : qsTr("Downloaded")
        case "waiting": return qsTr("Waiting for connection · %1 of %2").arg(kept.done).arg(kept.total)
        case "full": return qsTr("Not enough disk space · %1 of %2").arg(kept.done).arg(kept.total)
        case "tool": return qsTr("Needs yt-dlp · %1 of %2").arg(kept.done).arg(kept.total)
        default: return kept.asked ? qsTr("Downloading") : qsTr("Downloading %1 of %2").arg(kept.done).arg(kept.total)
        }
    }
    onClicked: {
        if (kept) removal.openFrom(button)
        else Api.downloadCollection(kind, entity, true)
    }

    Confirm {
        id: removal
        title: qsTr("Remove from Downloads?")
        text: qsTr("It won't play offline.")
        action: qsTr("Remove")
        onAccepted: Api.downloadCollection(button.kind, button.entity, false)
    }
}
