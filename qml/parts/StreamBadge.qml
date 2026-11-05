pragma ComponentBehavior: Bound
import QtQuick
import Snellar

Text {
    id: badge

    readonly property var stream: Api.stream
    readonly property var parts: {
        const s = stream
        if (!s || !s.codec) return []
        const out = [s.codec.toUpperCase()]
        if (s.bitrate > 0) out.push(qsTr("%1 kbps").arg(s.bitrate))
        if (s.local) out.push(qsTr("Downloaded"))
        return out
    }
    readonly property string detail: {
        if (!parts.length) return ""
        const s = stream
        const level = {
            gain: s.gain < 0 ? qsTr("Turned down by %1 dB to even out loudness").arg((-s.gain).toLocaleString(Qt.locale(), "f", 1)) : qsTr("Loudness already even"),
            measured: qsTr("Loudness evened out while playing")
        }[s.leveling] || ""
        return [s.sampleRate > 0 ? qsTr("%1 kHz").arg((s.sampleRate / 1000).toLocaleString(Qt.locale(), "f", 1)) : "", level].filter(l => l).join("\n")
    }

    visible: parts.length > 0
    text: parts.join(" · ")
    color: Theme.text3
    font.family: Theme.font
    font.pixelSize: Theme.fsCaption
    elide: Text.ElideRight

    HoverHandler { id: hover }
    Loader {
        active: hover.hovered && badge.detail.length > 0
        sourceComponent: Tip { text: badge.detail }
    }
}
