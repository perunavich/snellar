pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls
import Snellar

Modal {
    id: editor
    property var list: null
    property string cover: ""
    property bool deleting: false
    property var earlier: []
    readonly property var choices: (list && list.cover ? [list.cover] : []).concat(earlier)
    readonly property string shown: cover || (list ? list.thumb : "")
    signal saved()

    function edit(playlist, from) {
        list = playlist
        cover = playlist.cover
        name.text = playlist.name
        about.text = playlist.description
        earlier = []
        recall()
        openFrom(from || null)
        name.forceActiveFocus()
    }

    function recall() {
        const id = list.id
        Api.call("playlistCovers", { id: id }, v => { if (list && list.id === id) earlier = v || [] })
    }

    function pickFile(path) {
        Api.call("playlistCoverFile", { id: list.id, path: path }, (v, e) => {
            if (e) return Api.toast(e)
            cover = v
            list = Object.assign({}, list, { cover: v })
            recall()
            saved()
        })
    }

    function save() {
        const changes = { id: list.id, name: name.text.trim() || list.name, description: about.text.trim() }
        if (cover !== list.cover) changes.cover = cover
        close()
        Api.call("editPlaylist", changes, () => saved())
    }

    function remove() {
        const gone = list
        close()
        Api.deletePlaylist(gone)
    }

    width: Theme.editorWidth
    closePolicy: deleting ? Popup.CloseOnPressOutside : Popup.CloseOnEscape | Popup.CloseOnPressOutside
    onClosed: deleting = false

    Shortcut {
        sequence: "Escape"
        enabled: editor.opened
        onActivated: {
            if (editor.deleting) editor.deleting = false
            else editor.close()
        }
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
                    onAccepted: if (!editor.deleting) editor.save()
                    onTextEdited: editor.deleting = false
                    onActiveFocusChanged: if (activeFocus) editor.deleting = false
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
                    onTextChanged: editor.deleting = false
                    onActiveFocusChanged: if (activeFocus) editor.deleting = false
                }
            }
        }

        Text {
            visible: editor.earlier.length > 0
            text: qsTr("Earlier covers")
            color: Theme.text2
            font.pixelSize: Theme.fsCaption
        }

        Flow {
            width: parent.width
            visible: editor.earlier.length > 0
            spacing: Theme.spaceSm

            Repeater {
                model: editor.choices
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
                        onClicked: editor.cover = choice.modelData
                    }
                }
            }
        }

        PillButton { text: qsTr("Choose image…"); icon: "folder"; onClicked: coverPicker.open() }

        Item {
            width: parent.width
            height: Theme.controlLg

            HoverHandler {
                margin: Theme.spaceMd
                onHoveredChanged: if (!hovered) editor.deleting = false
            }
            PillButton {
                visible: !editor.deleting
                text: qsTr("Delete playlist")
                icon: "trash"
                onClicked: editor.deleting = true
            }
            Text {
                anchors { left: parent.left; right: answers.left; rightMargin: Theme.spaceMd; verticalCenter: parent.verticalCenter }
                visible: editor.deleting
                text: qsTr("Delete “%1”?").arg(editor.list ? editor.list.name : "")
                color: Theme.text
                font.pixelSize: Theme.fsBody
                font.weight: Font.DemiBold
                elide: Text.ElideRight
            }
            Row {
                id: answers
                anchors.right: parent.right
                spacing: Theme.spaceSm

                PillButton {
                    text: qsTr("Cancel")
                    onClicked: {
                        if (editor.deleting) editor.deleting = false
                        else editor.close()
                    }
                }
                PillButton {
                    text: editor.deleting ? qsTr("Delete") : qsTr("Save")
                    primary: !editor.deleting
                    danger: editor.deleting
                    onClicked: {
                        if (editor.deleting) editor.remove()
                        else editor.save()
                    }
                }
            }
        }
    }
}
