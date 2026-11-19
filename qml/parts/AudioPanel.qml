pragma ComponentBehavior: Bound
import QtQuick
import Snellar

Item {
    id: panel

    property Item dock: null
    property Item button: null
    property real settle: 1
    property string tab: Nav.audioTab
    readonly property var plain: ({ enabled: true, preamp: 0, eq: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0], bass: 0, speed: 1, pitchLock: false, reverb: 0, echo: 0, stereo: 1, normalize: false })
    property var fx: plain
    property bool loaded: false
    readonly property bool changed: Object.keys(plain).some(k => k !== "enabled" && JSON.stringify(fx[k]) !== JSON.stringify(plain[k]))
    readonly property Item card: surface
    readonly property bool asking: eq.picking
    readonly property bool saving: push.running

    property rect dockBox: Qt.rect(0, height, width, 0)
    property rect buttonBox: dockBox
    readonly property real roomLeft: dockBox.x - 2 * Theme.spaceMd
    readonly property real roomRight: width - dockBox.x - dockBox.width - 2 * Theme.spaceMd
    readonly property bool beside: Math.max(roomLeft, roomRight) >= Theme.audioMinWidth
    readonly property real ceiling: Theme.gap

    visible: shape.shown

    onTabChanged: Nav.audioTab = tab

    Component.onCompleted: {
        Api.call("getFx", {}, v => {
            if (v) panel.fx = v
            panel.loaded = true
        })
        Qt.callLater(sync)
    }

    function measure() {
        dockBox = dock ? boxOf(dock) : Qt.rect(0, height, width, 0)
        buttonBox = button ? boxOf(button) : dockBox
    }

    onWidthChanged: measure()
    onHeightChanged: measure()
    onSettleChanged: measure()

    PlaceWatch { item: panel.dock; onMoved: panel.measure() }
    PlaceWatch { item: panel.button; onMoved: panel.measure() }

    function sync() {
        measure()
        if (Nav.audio) return shape.grow(button as Pressable)
        shape.shrink()
        eq.closeMenu()
    }

    Connections {
        target: Nav
        function onAudioChanged() { panel.sync() }
    }

    function change(key, value) {
        fx = Object.assign({}, fx, { [key]: value })
        if (!push.running) push.start()
    }

    function set(values) {
        fx = values
        if (!push.running) push.start()
    }

    function reset() { set(Object.assign({}, plain)) }

    function resetWithUndo() {
        const before = fx
        reset()
        Api.offer(qsTr("Equalizer and effects reset"), qsTr("Undo"), () => panel.set(before))
    }

    function boxOf(item) { return item.mapToItem(panel, 0, 0, item.width, item.height) }

    function keeps(scene) {
        measure()
        const p = mapFromItem(null, scene.x, scene.y)
        const inside = r => p.x >= r.x && p.x < r.x + r.width && p.y >= r.y && p.y < r.y + r.height
        const toast = Nav.toast as Item
        return inside(Qt.rect(surface.x, surface.y, surface.width, surface.height)) || inside(dockBox) || (!!toast && toast.visible && inside(boxOf(toast)))
    }

    Binding {
        target: Nav
        property: "picking"
        value: true
        when: panel.visible && panel.asking
    }

    Timer { id: push; interval: Theme.fxDelay; onTriggered: Api.call("fx", { fx: panel.fx }) }

    PointHandler {
        acceptedButtons: Qt.AllButtons
        onActiveChanged: if (active && !panel.keeps(point.scenePosition)) Nav.audio = false
    }

    Item {
        id: surface
        enabled: Nav.audio
        width: Math.min(Theme.audioWidth, panel.beside ? Math.max(panel.roomLeft, panel.roomRight) : panel.width - 2 * Theme.gap)
        height: body.height + 2 * Theme.spaceXl
        x: {
            const d = panel.dockBox
            if (!panel.beside) return Math.max(Theme.gap, Math.min(panel.width - Theme.gap - width, d.x + d.width - width))
            return panel.roomLeft >= panel.roomRight ? d.x - Theme.spaceMd - width : d.x + d.width + Theme.spaceMd
        }
        y: {
            const low = panel.height - Theme.gap - height
            if (panel.beside) return Math.max(panel.ceiling, Math.min(low, panel.buttonBox.y + (panel.buttonBox.height - height) / 2))
            return Math.max(panel.ceiling, Math.min(low, panel.dockBox.y - Theme.gap - height))
        }

        LiquidShape {
            id: shape
            anchors.fill: parent
            content: body
        }

        MouseArea { anchors.fill: parent; acceptedButtons: Qt.AllButtons }

        Column {
            id: body
            x: Theme.spaceXl
            y: Theme.spaceXl
            width: parent.width - 2 * Theme.spaceXl
            spacing: Theme.spaceLg

            Item {
                width: parent.width
                height: Math.max(tabs.height, power.height)

                Row {
                    anchors.verticalCenter: parent.verticalCenter
                    spacing: Theme.spaceMd
                    IosSegment {
                        id: tabs
                        anchors.verticalCenter: parent.verticalCenter
                        width: Math.max(Theme.segmentNarrow, fit)
                        current: panel.tab
                        options: [{ key: "eq", label: qsTr("Equalizer") }, { key: "fx", label: qsTr("Effects") }]
                        onPicked: key => panel.tab = key
                    }
                    PillButton {
                        anchors.verticalCenter: parent.verticalCenter
                        height: Theme.segmentHeight
                        visible: panel.changed
                        icon: "refresh"
                        text: qsTr("Reset")
                        onClicked: panel.resetWithUndo()
                    }
                }
                Row {
                    anchors { right: parent.right; verticalCenter: parent.verticalCenter }
                    spacing: Theme.spaceSm
                    Text {
                        anchors.verticalCenter: parent.verticalCenter
                        text: qsTr("Enabled", "equalizer and effects switch")
                        color: Theme.text2
                        font.pixelSize: Theme.fsCallout

                        MouseArea {
                            anchors.fill: parent
                            onClicked: panel.change("enabled", !panel.fx.enabled)
                        }
                    }
                    IosSwitch {
                        id: power
                        anchors.verticalCenter: parent.verticalCenter
                        animated: panel.loaded
                        on: panel.fx.enabled
                        onToggled: panel.change("enabled", !panel.fx.enabled)
                    }
                }
            }

            Item {
                width: parent.width
                height: panel.tab === "eq" ? eq.height : effects.height
                enabled: panel.fx.enabled
                opacity: enabled ? 1 : Theme.disabledOpacity

                EqPane { id: eq; width: parent.width; fx: panel.fx; visible: panel.tab === "eq"; onChanged: (k, v) => panel.change(k, v) }
                FxPane { id: effects; width: parent.width; fx: panel.fx; animated: panel.loaded; visible: panel.tab === "fx"; onChanged: (k, v) => panel.change(k, v) }
            }
        }
    }
}
