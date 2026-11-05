pragma ComponentBehavior: Bound
import QtQuick
import Snellar

Column {
    id: list
    property bool open: false
    signal toggled()

    spacing: Theme.spaceSm

    readonly property var names: ({ mpv: "mpv", "yt-dlp": "yt-dlp", ffmpeg: "ffmpeg", qt: "Qt", audio: qsTr("Audio output"), discord: "Discord", portal: qsTr("File portal"), tray: qsTr("Tray"), blur: qsTr("Window blur") })
    readonly property var sources: ({ bundled: qsTr("Bundled"), system: qsTr("System"), home: qsTr("In your home folder") })
    readonly property var states: ({ ok: qsTr("Works"), missing: qsTr("Not installed"), broken: qsTr("Broken"), off: qsTr("Off"), on: qsTr("On"), waiting: qsTr("Not running"), none: qsTr("Not here") })

    function note(item) {
        if (item.state !== "missing") return ""
        if (item.id === "yt-dlp") return qsTr("Only Download needs it, and fetches it on first use")
        if (item.id === "ffmpeg") return qsTr("Optional: without it, downloaded songs have no tags")
        return ""
    }

    function line(item) {
        return [item.version, sources[item.source] || "", note(item)].filter(part => part.length > 0).join(" · ")
    }

    readonly property var ytdlp: found.data ? found.data.components.find(c => c.id === "yt-dlp") : null

    Fetch { id: found; method: "dependencies" }

    Connections {
        target: Api
        function onToolsChanged() { found.reload(true) }
    }

    Flow {
        width: parent.width
        spacing: Theme.spaceSm
        PillButton {
            visible: !!list.ytdlp && (list.ytdlp.state === "missing" || list.ytdlp.source === "home")
            enabled: !Api.ytdlpBusy
            opacity: enabled ? 1 : Theme.disabledOpacity
            text: Api.ytdlpBusy ? qsTr("Getting yt-dlp") : list.ytdlp && list.ytdlp.state === "missing" ? qsTr("Get yt-dlp") : qsTr("Update yt-dlp")
            onClicked: Api.fetchYtdlp()
        }
        PillButton {
            text: qsTr("Copy details")
            enabled: !!found.data
            onClicked: {
                Api.copyText(found.data.details)
                Api.toast(qsTr("Copied"))
            }
        }
        PillButton {
            text: list.open ? qsTr("Hide") : qsTr("Show")
            onClicked: list.toggled()
        }
    }

    Column {
        width: parent.width
        visible: list.open

        Repeater {
            model: found.data ? found.data.components : []

            delegate: Item {
                id: entry
                required property var modelData
                width: parent ? parent.width : 0
                height: Math.max(words.height, stateText.height) + 2 * Theme.spaceXs

                Column {
                    id: words
                    y: Theme.spaceXs
                    width: entry.width - stateText.width - Theme.spaceMd
                    Text { width: parent.width; text: list.names[entry.modelData.id] || entry.modelData.id; color: Theme.text2; font.pixelSize: Theme.fsCallout; wrapMode: Text.Wrap }
                    Text { width: parent.width; visible: text.length > 0; text: list.line(entry.modelData); color: Theme.text3; font.pixelSize: Theme.fsCaption; wrapMode: Text.Wrap }
                }
                Text {
                    id: stateText
                    x: entry.width - width
                    y: Theme.spaceXs
                    width: Math.max(Theme.componentStateMin, implicitWidth)
                    horizontalAlignment: Text.AlignRight
                    text: list.states[entry.modelData.state] || entry.modelData.state
                    color: entry.modelData.state === "broken" ? Theme.red : Theme.text3
                    font.pixelSize: Theme.fsCallout
                }
            }
        }
    }
}
