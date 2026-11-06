pragma ComponentBehavior: Bound
import QtQuick
import Snellar

Item {
    id: panel

    property Item dock: null
    property Item button: null
    property real settle: 1
    property string tab: "eq"
    readonly property var plain: ({ enabled: true, preamp: 0, eq: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0], bass: 0, speed: 1, pitchLock: false, reverb: 0, echo: 0, stereo: 1, normalize: false })
    property var fx: plain
    readonly property bool changed: Object.keys(plain).some(k => k !== "enabled" && JSON.stringify(fx[k]) !== JSON.stringify(plain[k]))
    readonly property Item card: surface
    readonly property bool asking: confirm.opened || eq.picking

    readonly property rect dockBox: dock ? boxOf(dock, settle, width, height) : Qt.rect(0, height, width, 0)
    readonly property rect buttonBox: button ? boxOf(button, dockBox) : dockBox
    readonly property real roomLeft: dockBox.x - 2 * Theme.spaceMd
    readonly property real roomRight: width - dockBox.x - dockBox.width - 2 * Theme.spaceMd
    readonly property bool beside: Math.max(roomLeft, roomRight) >= Theme.audioMinWidth
    readonly property real ceiling: Theme.frameless ? Theme.windowBarHeight : Theme.gap

    visible: Nav.audio

    Component.onCompleted: Api.call("getFx", {}, v => { if (v) panel.fx = v })

    function change(key, value) {
        fx = Object.assign({}, fx, { [key]: value })
        push.restart()
    }

    function reset() {
        fx = Object.assign({}, plain)
        push.restart()
    }

    function boxOf(item) { return item.mapToItem(panel, 0, 0, item.width, item.height) }

    function keeps(scene) {
        const p = mapFromItem(null, scene.x, scene.y)
        const inside = r => p.x >= r.x && p.x < r.x + r.width && p.y >= r.y && p.y < r.y + r.height
        return inside(Qt.rect(surface.x, surface.y, surface.width, surface.height)) || inside(dockBox)
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

    Rectangle {
        id: surface
        width: Math.min(Theme.audioWidth, panel.beside ? Math.max(panel.roomLeft, panel.roomRight) : panel.width - 2 * Theme.gap)
        height: body.height + 2 * Theme.spaceXl
        x: {
            const d = panel.dockBox
            if (!panel.beside) return Math.max(Theme.gap, Math.min(panel.width - Theme.gap - width, d.x + d.width - width))
            return panel.roomLeft >= panel.roomRight ? d.x - Theme.spaceMd - width : d.x + d.width + Theme.spaceMd
        }
        y: {
            const low = panel.height - Theme.gap - height
            if (panel.beside) return Math.max(panel.ceiling, Math.min(low, panel.buttonBox.y + panel.buttonBox.height + Theme.spaceXl - height))
            return Math.max(panel.ceiling, Math.min(low, panel.dockBox.y - Theme.gap - height))
        }
        radius: Theme.rLg
        color: Theme.menu
        border.width: 1
        border.color: Theme.separator

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
                        onClicked: confirm.open()
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
                            cursorShape: Qt.PointingHandCursor
                            onClicked: panel.change("enabled", !panel.fx.enabled)
                        }
                    }
                    IosSwitch {
                        id: power
                        anchors.verticalCenter: parent.verticalCenter
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
                FxPane { id: effects; width: parent.width; fx: panel.fx; visible: panel.tab === "fx"; onChanged: (k, v) => panel.change(k, v) }
            }
        }
    }

    onVisibleChanged: if (!visible) {
        confirm.close()
        eq.closeMenu()
    }

    Confirm {
        id: confirm
        title: qsTr("Reset the equalizer and effects?")
        text: qsTr("Every band and effect goes back to its default.")
        action: qsTr("Reset")
        onAccepted: panel.reset()
    }
}
