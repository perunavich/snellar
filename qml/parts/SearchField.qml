pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls
import Snellar

Item {
    id: search

    property var suggestions: []
    property int pick: -1
    property bool holding: false
    property string shownFor: ""
    readonly property bool dropped: (field.activeFocus || holding) && suggestions.length > 0 && shownFor === field.text.trim()

    function entries(v) {
        if (!v) return []
        return (v.queries || []).map(q => ({ query: q })).concat((v.hits || []).map(h => ({ hit: h })))
    }

    function choose(entry) {
        if (entry.query !== undefined) return submit(entry.query)
        field.focus = false
        suggestions = []
        Api.openHit(entry.hit)
    }

    implicitHeight: Theme.sidebarRow + Theme.spaceSm

    function focusField() {
        field.forceActiveFocus()
        field.selectAll()
    }

    function submit(q) {
        const query = q.trim()
        if (!query.length) return
        field.text = query
        field.focus = false
        suggestions = []
        Nav.search(query)
    }

    Timer {
        id: suggestTimer
        interval: Theme.debounce
        onTriggered: {
            const q = field.text.trim()
            if (q.length < 2) { search.suggestions = []; return }
            Api.call("suggest", { q: q }, v => { if (field.text.trim() === q) { search.suggestions = search.entries(v); search.shownFor = q; search.pick = -1 } })
        }
    }

    Rectangle {
        anchors.fill: parent
        radius: height / 2
        color: field.activeFocus ? Theme.fillPress : Theme.fillRaised
        Behavior on color { ColorAnimation { duration: Theme.hover } }

        Icon {
            id: lens
            anchors { left: parent.left; leftMargin: Theme.spaceSm; verticalCenter: parent.verticalCenter }
            name: "search"
            size: Theme.iconSm
            color: Theme.text3
        }

        TextField {
            id: field
            anchors { left: lens.right; right: clear.left; leftMargin: Theme.spaceSm; verticalCenter: parent.verticalCenter }
            background: null
            leftPadding: 0
            color: Theme.text
            placeholderText: qsTr("Search")
            placeholderTextColor: Theme.text3
            font.pixelSize: Theme.fsCallout
            selectByMouse: true
            onTextEdited: suggestTimer.restart()
            onAccepted: search.pick >= 0 ? search.choose(search.suggestions[search.pick]) : search.submit(text)
            Keys.onEscapePressed: { text = ""; focus = false }
            Keys.onDownPressed: search.pick = Math.min(search.pick + 1, search.suggestions.length - 1)
            Keys.onUpPressed: search.pick = Math.max(search.pick - 1, -1)
        }

        IconButton {
            id: clear
            anchors { right: parent.right; rightMargin: Theme.spaceXs; verticalCenter: parent.verticalCenter }
            icon: "close"
            tip: qsTr("Clear")
            size: Theme.sidebarRow
            iconSize: Theme.iconXs
            visible: field.text.length > 0
            onClicked: { field.text = ""; field.forceActiveFocus() }
        }
    }

    Connections {
        target: Nav
        function onQueryChanged() { if (field.text !== Nav.query) field.text = Nav.query }
    }

    Rectangle {
        z: 10
        anchors { top: parent.bottom; topMargin: Theme.spaceXs; left: parent.left; right: parent.right }
        height: list.height + Theme.spaceSm * 2
        radius: Theme.rCard
        color: Theme.chromeStrong
        visible: opacity > 0.01
        opacity: search.dropped ? 1 : 0
        Behavior on opacity { NumberAnimation { duration: Theme.fast } }

        Column {
            id: list
            x: Theme.spaceSm
            y: Theme.spaceSm
            width: parent.width - Theme.spaceSm * 2

            Repeater {
                model: search.suggestions
                delegate: Rectangle {
                    id: row
                    required property var modelData
                    required property int index
                    readonly property var hit: modelData.hit || null
                    width: parent.width
                    height: hit ? Theme.rowHeight : Theme.controlMd
                    radius: Theme.rRow
                    color: search.pick === index || hover.hovered ? Theme.fillHover : "transparent"

                    Cover {
                        id: art
                        visible: !!row.hit
                        x: Theme.spaceSm
                        anchors.verticalCenter: parent.verticalCenter
                        width: Theme.rowCover
                        height: Theme.rowCover
                        radius: row.hit && row.hit.type === "artist" ? Theme.rowCover / 2 : Theme.rArtS
                        px: 96
                        url: row.hit ? row.hit.thumb : ""
                    }
                    Text {
                        id: name
                        anchors { left: row.hit ? art.right : parent.left; leftMargin: row.hit ? Theme.spaceSm : Theme.spaceMd; right: parent.right; rightMargin: Theme.spaceMd }
                        anchors.verticalCenter: parent.verticalCenter
                        anchors.verticalCenterOffset: row.hit ? -sub.height / 2 : 0
                        text: row.hit ? (row.hit.title || row.hit.name) : row.modelData.query
                        color: Theme.text
                        font.pixelSize: Theme.fsCallout
                        elide: Text.ElideRight
                    }
                    Text {
                        id: sub
                        anchors { left: name.left; right: name.right; top: name.bottom }
                        visible: !!row.hit
                        text: row.hit ? Api.hitCaption(row.hit) : ""
                        color: Theme.text2
                        font.pixelSize: Theme.fsCaption
                        elide: Text.ElideRight
                    }
                    HoverHandler { id: hover; cursorShape: Qt.PointingHandCursor }
                    TapHandler {
                        gesturePolicy: TapHandler.ReleaseWithinBounds
                        onPressedChanged: search.holding = pressed
                        onTapped: search.choose(row.modelData)
                    }
                }
            }
        }
    }
}
