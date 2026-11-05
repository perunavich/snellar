pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls
import Snellar

Popup {
    id: editor
    property var list: null
    property string cover: ""
    readonly property var covers: list ? list.tracks.map(t => t.thumb).filter((u, i, all) => u && all.indexOf(u) === i).slice(0, 8) : []
    readonly property string shown: cover || (covers.length ? covers[0] : "")
    signal saved()

    function edit(playlist) {
        list = playlist
        cover = playlist.cover
        name.text = playlist.name
        about.text = playlist.description
        open()
        name.forceActiveFocus()
    }

    function chooseCover(playlist) {
        list = playlist
        cover = playlist.cover
        coverPicker.open()
    }

    function pickFile(path) {
        Api.call("playlistCoverFile", { id: list.id, path: path }, (v, e) => {
            if (e) return Api.toast(e)
            cover = v
            list = Object.assign({}, list, { cover: v })
            saved()
        })
    }

    function save() {
        const changes = { id: list.id, name: name.text.trim() || list.name, description: about.text.trim() }
        if (cover !== list.cover && !cover.startsWith("file:")) changes.cover = cover
        close()
        Api.call("editPlaylist", changes, () => saved())
    }

    parent: Overlay.overlay
    anchors.centerIn: parent
    width: Theme.editorWidth
    modal: true
    focus: true
    padding: Theme.spaceXl

    Overlay.modal: Rectangle { color: Theme.scrim }

    Shortcut {
        sequence: "Escape"
        enabled: editor.opened
        onActivated: editor.close()
    }

    background: Rectangle {
        radius: Theme.rLg
        color: Theme.menu
        border.width: 1
        border.color: Theme.separator
    }

    FilePicker {
        id: coverPicker
        title: qsTr("Choose a cover")
        nameFilters: [qsTr("Images (*.png *.jpg *.jpeg *.webp)")]
        onPicked: path => editor.pickFile(path)
    }

    contentItem: Column {
        spacing: Theme.spaceLg

        Text {
            text: qsTr("Edit details")
            color: Theme.text
            font.pixelSize: Theme.fsHeadline
            font.weight: Font.DemiBold
        }

        Row {
            spacing: Theme.spaceLg

            Cover {
                width: Theme.editorCover
                height: Theme.editorCover
                radius: Theme.rMd
                px: 400
                url: editor.shown
            }

            Column {
                width: editor.availableWidth - Theme.editorCover - Theme.spaceLg
                spacing: Theme.spaceSm

                TextField {
                    id: name
                    width: parent.width
                    color: Theme.text
                    placeholderText: qsTr("Name")
                    placeholderTextColor: Theme.text3
                    font.pixelSize: Theme.fsBody
                    selectByMouse: true
                    background: Rectangle { radius: Theme.rSm; color: Theme.fillRaised }
                    onAccepted: editor.save()
                }

                TextArea {
                    id: about
                    width: parent.width
                    height: Theme.editorCover - name.height - Theme.spaceSm
                    color: Theme.text
                    placeholderText: qsTr("Add a description")
                    placeholderTextColor: Theme.text3
                    font.pixelSize: Theme.fsCallout
                    wrapMode: TextEdit.Wrap
                    selectByMouse: true
                    background: Rectangle { radius: Theme.rSm; color: Theme.fillRaised }
                }
            }
        }

        Text {
            visible: editor.covers.length > 1
            text: qsTr("Cover from a song")
            color: Theme.text2
            font.pixelSize: Theme.fsCaption
        }

        Flow {
            width: parent.width
            visible: editor.covers.length > 1
            spacing: Theme.spaceSm

            Repeater {
                model: editor.covers
                delegate: Item {
                    id: choice
                    required property string modelData
                    width: Theme.controlXl
                    height: Theme.controlXl

                    Cover { anchors.fill: parent; radius: Theme.rSm; px: 96; url: choice.modelData }
                    Rectangle {
                        anchors.fill: parent
                        radius: Theme.rSm
                        color: "transparent"
                        border.width: Theme.ring
                        border.color: Theme.accent
                        visible: editor.shown === choice.modelData
                    }
                    MouseArea {
                        anchors.fill: parent
                        cursorShape: Qt.PointingHandCursor
                        onClicked: editor.cover = choice.modelData
                    }
                }
            }
        }

        Flow {
            width: parent.width
            spacing: Theme.spaceSm

            PillButton { text: qsTr("Choose image…"); icon: "folder"; onClicked: coverPicker.open() }
            PillButton {
                visible: editor.cover.length > 0
                text: qsTr("Use the first song's cover")
                onClicked: editor.cover = ""
            }
        }

        Row {
            anchors.right: parent.right
            spacing: Theme.spaceSm

            PillButton { text: qsTr("Cancel"); onClicked: editor.close() }
            PillButton { text: qsTr("Save"); primary: true; onClicked: editor.save() }
        }
    }
}
