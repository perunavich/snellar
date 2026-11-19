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

        PageTitle { text: qsTr("Import playlist") }

        Text {
            width: Math.min(page.innerWidth, Theme.importWidth)
            text: qsTr("Paste a link to a public playlist or album from Spotify, Deezer, Apple Music, Yandex Music or SoundCloud, or choose a CSV file from Exportify or TuneMyMusic. Snellar finds each song on YouTube Music.")
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
                enabled: !Imports.running && link.text.trim().length > 0
                onClicked: view.startLink()
            }
            PillButton {
                text: qsTr("Choose CSV file")
                enabled: !Imports.running
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
                    width: parent.width * fill.value
                    height: parent.height
                    radius: parent.radius
                    color: Theme.accent

                    SpringValue {
                        id: fill
                        reducible: true
                        zeta: Motion.controlDamping
                        response: Motion.controlResponse
                        target: Imports.total > 0 ? Imports.done / Imports.total : 0
                    }
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
                    : qsTr("None of the %1 songs are on YouTube Music, so no playlist was created").arg(view.result.total)
                color: Theme.text
                font.pixelSize: Theme.fsBody
                font.weight: Font.DemiBold
                wrapMode: Text.WordWrap
            }
            Text {
                visible: !!view.result && view.result.limited
                width: parent.width
                text: !view.result ? "" : view.result.sourceTotal
                    ? qsTr("The service shows %1 of the %2 songs in this playlist. Export it to CSV to import all of them.").arg(view.result.total).arg(view.result.sourceTotal)
                    : qsTr("The page shows only the first %1 songs, and the playlist may have more. Export it to CSV to import all of them.").arg(view.result.total)
                color: Theme.text2
                font.pixelSize: Theme.fsCallout
                wrapMode: Text.WordWrap
            }
            Text {
                visible: !!view.result && view.result.failed > 0
                width: parent.width
                text: !view.result ? "" : qsTr("Couldn't look up %n song(s): %1 Import again to retry them.", "", view.result.failed).arg(Errors.text(view.result.code))
                color: Theme.text2
                font.pixelSize: Theme.fsCallout
                wrapMode: Text.WordWrap
            }
            Text {
                readonly property var places: view.result ? view.result.hidden || [] : []
                visible: places.length > 0
                width: parent.width
                text: qsTr("%n song(s) in this playlist aren't available in your country, and the service doesn't say what they are, so they weren't imported. Their places in the playlist: %1.", "", places.length).arg(places.join(", "))
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
                delegate: SongLine {}
            }

            Text {
                visible: twice.count > 0
                height: Theme.controlMd
                verticalAlignment: Text.AlignVCenter
                text: qsTr("Already in the playlist: %1").arg(twice.count)
                color: Theme.text
                font.pixelSize: Theme.fsBody
            }
            Repeater {
                id: twice
                model: view.result ? view.result.merged || [] : []
                delegate: SongLine {}
            }
        }
    }

    component SongLine: Text {
        required property var modelData
        width: parent.width
        text: [modelData.artists.join(", "), modelData.title].filter(x => x).join(" – ") + (modelData.duration > 0 ? "  ·  " + Theme.time(modelData.duration) : "")
        color: Theme.text2
        font.pixelSize: Theme.fsCallout
        elide: Text.ElideRight
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
