pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls
import QtQuick.Effects
import Snellar

Item {
    id: search

    property var suggestions: []
    property var recents: []
    readonly property Item recentSurface: surface
    property int pick: -1
    readonly property bool holding: inside.active
    readonly property bool busy: field.activeFocus || holding
    property bool floating: false
    property bool lensRight: false
    property real fold: 0
    property real dropWidth: 0
    function holds(x, y) {
        const inside = item => item.visible && item.contains(item.mapFromItem(null, x, y))
        return inside(search) || (dropped && inside(dropdown))
    }
    property string shownFor: ""
    readonly property bool recalling: field.text.trim() === ""
    readonly property var shown: recalling ? recents : suggestions
    readonly property bool dropped: (field.activeFocus || holding) && shown.length > 0 && (recalling || shownFor === field.text.trim())

    function entries(v) {
        if (!v) return []
        return (v.queries || []).map(q => ({ query: q })).concat((v.hits || []).map(h => ({ hit: h })))
    }

    function choose(entry) {
        if (entry.query !== undefined) return submit(entry.query)
        remember({ hit: entry.hit })
        field.focus = false
        suggestions = []
        Api.openHit(entry.hit)
    }

    function same(a, b) {
        if (a.hit || b.hit) return !!a.hit && !!b.hit && a.hit.type === b.hit.type && a.hit.id === b.hit.id
        return a.query.toLocaleLowerCase() === b.query.toLocaleLowerCase()
    }

    function keepRecents(list) {
        recents = list
        pick = Math.min(pick, list.length - 1)
        Api.call("setSetting", { key: "recentSearches", value: list })
    }

    function remember(entry) { keepRecents([entry].concat(recents.filter(e => !same(e, entry))).slice(0, Theme.recentSearches)) }

    function forget(entry) {
        keepRecents(recents.filter(e => !same(e, entry)))
        field.forceActiveFocus()
    }

    function forgetAll() {
        keepRecents([])
        field.forceActiveFocus()
    }

    function loadRecents() {
        Api.call("getSetting", { key: "recentSearches" }, v => {
            recents = (Array.isArray(v) ? v : []).filter(e => e && (typeof e.query === "string" || (e.hit && e.hit.type))).slice(0, Theme.recentSearches)
        })
    }

    implicitHeight: Theme.fieldButton + Theme.spaceSm

    function leave() { field.focus = false }

    function focusField() {
        field.forceActiveFocus()
        field.selectAll()
    }

    Component.onCompleted: loadRecents()

    function follow() { if (field.text !== Nav.query) field.text = Nav.query }

    function submit(q) {
        const query = q.trim()
        if (!query.length) return
        field.text = query
        field.focus = false
        suggestions = []
        remember({ query: query })
        Nav.search(query)
    }

    TextMetrics {
        id: hint
        font: field.font
        text: field.placeholderText
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

    Item {
        anchors.fill: parent

        Rectangle {
            anchors.fill: parent
            radius: height / 2
            visible: opacity > 0
            opacity: Motion.railWords(search.fold)
            color: search.floating ? Theme.menu : field.activeFocus ? Theme.fillPress : fieldHover.hovered ? Theme.washed(Theme.fillRaised) : Theme.fillRaised
            border.width: search.floating ? 1 : 0
            border.color: Theme.separator
            Behavior on color { ColorAnimation { duration: Theme.hover } }
        }

        MouseArea {
            anchors.fill: parent
            onPressed: field.forceActiveFocus()
        }
        HoverHandler { id: fieldHover; cursorShape: Qt.IBeamCursor }

        Icon {
            id: lens
            x: search.lensRight ? parent.width - width - Theme.spaceSm : Theme.spaceSm
            anchors.verticalCenter: parent.verticalCenter
            name: "search"
            size: Theme.iconMd
            color: Theme.text3
        }

        TextField {
            id: field
            x: search.lensRight ? clear.x + clear.width : lens.x + lens.width + Theme.spaceSm
            width: (search.lensRight ? lens.x - Theme.spaceSm : clear.x) - x
            anchors.verticalCenter: parent.verticalCenter
            visible: opacity > 0
            opacity: Motion.railWords(search.fold, field.length ? Theme.spaceSm : field.width - field.leftPadding - field.rightPadding - hint.advanceWidth)
            background: null
            leftPadding: 0
            color: Theme.text
            placeholderText: qsTr("Search")
            placeholderTextColor: Theme.text3
            font.pixelSize: Theme.fsCallout
            selectByMouse: true
            onTextEdited: {
                search.pick = -1
                suggestTimer.restart()
            }
            onActiveFocusChanged: if (activeFocus) search.pick = -1
            onAccepted: search.pick >= 0 && search.dropped ? search.choose(search.shown[search.pick]) : search.submit(text)
            Keys.onEscapePressed: { text = ""; focus = false }
            Keys.onDownPressed: search.pick = Math.min(search.pick + 1, search.shown.length - 1)
            Keys.onUpPressed: search.pick = Math.max(search.pick - 1, -1)
        }

        IconButton {
            id: clear
            x: search.lensRight ? Theme.spaceXs : parent.width - width - Theme.spaceXs
            anchors.verticalCenter: parent.verticalCenter
            icon: "close"
            tip: qsTr("Clear")
            size: Theme.fieldButton
            iconSize: Theme.iconXs
            visible: field.text.length > 0 && opacity > 0
            opacity: Motion.railWords(search.fold)
            onClicked: { field.text = ""; search.pick = -1; field.forceActiveFocus() }
        }

        Item {
            objectName: "lensButton"
            x: search.lensRight ? parent.width - width : 0
            width: Theme.sidebarRow
            height: parent.height
            visible: search.floating
            Tap { onTapped: search.leave() }
        }
    }

    Connections {
        target: Nav
        function onQueryChanged() { search.follow() }
        function onArrived() { search.follow() }
    }

    Item {
        id: dropdown
        z: 10
        anchors { top: parent.bottom; topMargin: Theme.spaceXs }
        x: search.lensRight ? search.width - width : 0
        width: Math.max(search.width, search.dropWidth)
        height: list.height + Theme.spaceSm * 2
        visible: opacity > 0.01
        enabled: search.dropped
        opacity: search.dropped ? 1 : 0
        Behavior on opacity { NumberAnimation { duration: Theme.fast } }

        MouseArea { anchors.fill: parent }

        Rectangle {
            id: surface
            anchors.fill: parent
            radius: Theme.rCard
            color: Theme.menu
            border.width: 1
            border.color: Theme.separator
            layer.enabled: true
            visible: false
        }
        MultiEffect {
            anchors.fill: surface
            source: surface
            shadowEnabled: true
            shadowColor: Theme.shade
            shadowOpacity: Motion.shadowAlpha
            shadowBlur: 1
            shadowVerticalOffset: Motion.shadowDrop / 2
        }

        PointHandler { id: inside }

        Column {
            id: list
            x: Theme.spaceSm
            y: Theme.spaceSm
            width: parent.width - Theme.spaceSm * 2

            Item {
                visible: search.recalling
                width: parent.width
                height: Theme.controlMd

                Text {
                    anchors { left: parent.left; leftMargin: Theme.spaceSm; right: clearAll.left; rightMargin: Theme.spaceSm; verticalCenter: parent.verticalCenter }
                    text: qsTr("Recent searches")
                    color: Theme.text2
                    font.pixelSize: Theme.fsCaption
                    font.weight: Font.DemiBold
                    elide: Text.ElideRight
                }
                Link {
                    id: clearAll
                    anchors { right: parent.right; rightMargin: Theme.spaceSm; verticalCenter: parent.verticalCenter }
                    text: qsTr("Clear all")
                    font.pixelSize: Theme.fsCaption
                    onClicked: search.forgetAll()
                }
            }

            Repeater {
                model: search.shown
                delegate: Rectangle {
                    id: row
                    required property var modelData
                    required property int index
                    readonly property var hit: modelData.hit || null
                    readonly property bool past: search.recalling
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
                        px: Theme.coverRowPx
                        url: row.hit ? row.hit.thumb : ""
                    }
                    Icon {
                        id: mark
                        visible: row.past && !row.hit
                        x: Theme.spaceSm
                        anchors.verticalCenter: parent.verticalCenter
                        name: "history"
                        size: Theme.iconSm
                        color: Theme.text3
                    }
                    Text {
                        id: name
                        anchors { left: row.hit ? art.right : (row.past ? mark.right : parent.left); leftMargin: row.hit || row.past ? Theme.spaceSm : Theme.spaceMd; right: row.past ? drop.left : parent.right; rightMargin: row.past ? Theme.spaceXs : Theme.spaceMd }
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
                        text: row.hit ? Api.hitCaption(row.hit, true) : ""
                        color: Theme.text2
                        font.pixelSize: Theme.fsCaption
                        elide: Text.ElideRight
                    }
                    HoverHandler { id: hover }
                    TapHandler {
                        gesturePolicy: TapHandler.ReleaseWithinBounds
                        onTapped: point => row.past && drop.contains(drop.mapFromItem(row, point.position)) ? search.forget(row.modelData) : search.choose(row.modelData)
                    }
                    IconButton {
                        id: drop
                        visible: row.past
                        anchors { right: parent.right; rightMargin: Theme.spaceXs; verticalCenter: parent.verticalCenter }
                        icon: "close"
                        tip: qsTr("Remove from recent searches")
                        size: Theme.fieldButton
                        iconSize: Theme.iconXs
                        onClicked: search.forget(row.modelData)
                    }
                }
            }
        }
    }
}
