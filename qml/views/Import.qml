pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls
import Snellar

Item {
    id: view
    property var params: ({})
    readonly property var result: Imports.result
    readonly property bool hidden: Imports.error === "import.blocked" || Imports.error === "import.unreadable"

    function startLink() {
        if (Imports.running || !link.text.trim()) return
        Imports.start({ url: link.text.trim(), name: name.text })
    }

    function pickFile(path) {
        Imports.start({ path: path, name: name.text })
    }

    function copyMissing() {
        clipboard.text = Imports.lines()
        clipboard.selectAll()
        clipboard.copy()
        Api.toast(qsTr("List copied"))
    }

    TextEdit { id: clipboard; visible: false }

    FilePicker {
        id: files
        title: qsTr("Choose a CSV file")
        nameFilters: [qsTr("CSV files (*.csv)"), qsTr("All files (*)")]
        onPicked: path => view.pickFile(path)
    }

    PageScroll {
        id: page
        anchors.fill: parent
        spacing: Theme.spaceLg

        Text {
            text: qsTr("Import playlist")
            color: Theme.text
            font.family: Theme.font
            font.pixelSize: Theme.fsLargeTitle
            font.weight: Font.DemiBold
        }

        Text {
            width: Math.min(page.innerWidth, Theme.importWidth)
            text: qsTr("Paste a link to a public playlist or album from Spotify, Deezer, Apple Music, Yandex Music or SoundCloud, or choose a CSV file exported with Exportify or TuneMyMusic. Each song is looked up on YouTube Music.")
            color: Theme.text2
            font.pixelSize: Theme.fsBody
            wrapMode: Text.WordWrap
        }

        Field {
            id: link
            placeholderText: qsTr("Playlist link")
            onAccepted: view.startLink()
        }

        Field {
            id: name
            placeholderText: qsTr("Name of the new playlist (optional)")
            onAccepted: view.startLink()
        }

        Flow {
            width: Math.min(page.innerWidth, Theme.importWidth)
            spacing: Theme.spaceMd
            PillButton {
                text: qsTr("Import")
                primary: true
                opacity: enabled ? 1 : Theme.disabledOpacity
                enabled: !Imports.running && link.text.trim().length > 0
                onClicked: view.startLink()
            }
            PillButton {
                text: qsTr("Choose CSV file")
                enabled: !Imports.running
                opacity: enabled ? 1 : Theme.disabledOpacity
                onClicked: files.open()
            }
            PillButton {
                visible: Imports.running
                text: qsTr("Cancel")
                onClicked: Imports.cancel()
            }
        }

        Column {
            visible: Imports.running
            width: Math.min(page.innerWidth, Theme.importWidth)
            spacing: Theme.spaceSm

            Text {
                text: Imports.stage === "matching"
                    ? qsTr("Finding songs on YouTube Music: %1 of %2, %3 found").arg(Imports.done).arg(Imports.total).arg(Imports.matched)
                    : qsTr("Reading the playlist…")
                color: Theme.text2
                font.pixelSize: Theme.fsCallout
            }
            Rectangle {
                width: parent.width
                height: Theme.importBar
                radius: height / 2
                color: Theme.fillRaised
                Rectangle {
                    width: Imports.total > 0 ? parent.width * Imports.done / Imports.total : 0
                    height: parent.height
                    radius: parent.radius
                    color: Theme.accent
                    Behavior on width { NumberAnimation { duration: Theme.normal } }
                }
            }
        }

        Text {
            visible: Imports.error.length > 0
            width: Math.min(page.innerWidth, Theme.importWidth)
            text: Errors.text(Imports.error)
            color: view.hidden ? Theme.text : Theme.red
            font.pixelSize: Theme.fsBody
            wrapMode: Text.WordWrap
        }

        Column {
            visible: !!view.result
            width: Math.min(page.innerWidth, Theme.importWidth)
            spacing: Theme.spaceMd

            Text {
                width: parent.width
                text: !view.result ? "" : view.result.playlist !== null
                    ? qsTr("Added %1 of %2 songs to “%3”").arg(view.result.matched).arg(view.result.total).arg(view.result.name)
                    : qsTr("None of the %1 songs were found on YouTube Music, so no playlist was made").arg(view.result.total)
                color: Theme.text
                font.pixelSize: Theme.fsBody
                font.weight: Font.DemiBold
                wrapMode: Text.WordWrap
            }
            Text {
                visible: !!view.result && view.result.limited
                width: parent.width
                text: !view.result ? "" : view.result.sourceTotal
                    ? qsTr("The service shows %1 of the %2 songs in this playlist. Export it to a CSV file to import all of it.").arg(view.result.total).arg(view.result.sourceTotal)
                    : qsTr("The page shows only the first %1 songs of this playlist, and it may hold more. Export it to a CSV file to import all of it.").arg(view.result.total)
                color: Theme.text2
                font.pixelSize: Theme.fsCallout
                wrapMode: Text.WordWrap
            }
            Text {
                visible: !!view.result && view.result.failed > 0
                width: parent.width
                text: !view.result ? "" : qsTr("%1 songs could not be looked up: %2 Import again to try them.").arg(view.result.failed).arg(Errors.text(view.result.code))
                color: Theme.text2
                font.pixelSize: Theme.fsCallout
                wrapMode: Text.WordWrap
            }
            PillButton {
                visible: !!view.result && view.result.playlist !== null
                text: qsTr("Open playlist")
                icon: "playlist"
                onClicked: Nav.playlist(view.result.playlist)
            }

            Item {
                visible: missing.count > 0
                width: parent.width
                height: Theme.controlMd
                Text {
                    anchors { left: parent.left; verticalCenter: parent.verticalCenter }
                    text: qsTr("Not found on YouTube Music: %1").arg(missing.count)
                    color: Theme.text
                    font.pixelSize: Theme.fsBody
                }
                Link {
                    anchors { right: parent.right; verticalCenter: parent.verticalCenter }
                    text: qsTr("Copy list")
                    onClicked: view.copyMissing()
                }
            }
            Repeater {
                id: missing
                model: view.result ? view.result.unmatched : []
                delegate: Text {
                    required property var modelData
                    width: parent.width
                    text: [modelData.artists.join(", "), modelData.title].filter(x => x).join(" – ") + (modelData.duration > 0 ? "  ·  " + Theme.time(modelData.duration) : "")
                    color: Theme.text2
                    font.pixelSize: Theme.fsCallout
                    elide: Text.ElideRight
                }
            }
        }
    }

    component Field: TextField {
        width: Math.min(page.innerWidth, Theme.importWidth)
        height: Theme.controlMd
        leftPadding: Theme.spaceMd
        enabled: !Imports.running
        placeholderTextColor: Theme.text3
        color: Theme.text
        font.pixelSize: Theme.fsBody
        background: Rectangle { radius: height / 2; color: Theme.fillRaised }
    }
}
