import QtQuick
import Snellar

Item {
    id: sheet

    property bool open: false
    property string tab: "eq"
    property var fx: ({ enabled: true, preamp: 0, eq: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0], bass: 0, speed: 1, pitchLock: false, reverb: 0, echo: 0, stereo: 1, normalize: false })
    readonly property real p: spring.value

    visible: p > 0.002

    Component.onCompleted: {
        Api.call("getFx", {}, v => { if (v) sheet.fx = v })
        Qt.callLater(() => open = Qt.binding(() => Nav.audio))
    }

    function change(key, value) {
        fx = Object.assign({}, fx, { [key]: value })
        push.restart()
    }

    Timer { id: push; interval: Theme.fxDelay; onTriggered: Api.call("fx", { fx: sheet.fx }) }

    SpringValue {
        id: spring
        zeta: sheet.open ? Theme.springBase[0] : Theme.springCritical[0]
        response: sheet.open ? Theme.springBase[1] : Theme.springCritical[1]
        target: sheet.open ? 1 : 0
    }

    Rectangle {
        anchors.fill: parent
        color: Theme.black
        opacity: Theme.scrim.a * Math.min(1, sheet.p)
        MouseArea { anchors.fill: parent; onClicked: Nav.audio = false }
    }

    Item {
        width: Math.min(Theme.sheetWidth, parent.width - 2 * Theme.space2xl)
        height: Math.min(Theme.sheetHeight, parent.height - 2 * Theme.space2xl)
        anchors.centerIn: parent
        opacity: Math.min(1, sheet.p * 1.5)
        scale: 0.92 + 0.08 * sheet.p
        transform: Translate { y: (1 - sheet.p) * Theme.sheetRise }

        Glass { anchors.fill: parent; radius: Theme.rSheet; color: Theme.chromeStrong }
        MouseArea { anchors.fill: parent }

        Item {
            id: head
            x: Theme.spaceXl
            y: Theme.spaceXl
            width: parent.width - 2 * Theme.spaceXl
            height: Theme.controlLg

            Row {
                anchors.verticalCenter: parent.verticalCenter
                spacing: Theme.spaceMd
                Icon { anchors.verticalCenter: parent.verticalCenter; name: "sliders"; size: Theme.iconLg; color: Theme.text }
                Text { text: qsTr("Audio"); color: Theme.text; font.family: Theme.font; font.pixelSize: Theme.fsLargeTitle; font.weight: Font.DemiBold }
            }
            Row {
                anchors { right: parent.right; verticalCenter: parent.verticalCenter }
                spacing: Theme.spaceMd
                Text { anchors.verticalCenter: parent.verticalCenter; text: sheet.fx.enabled ? qsTr("On") : qsTr("Off"); color: Theme.text2; font.pixelSize: Theme.fsBody }
                IosSwitch { anchors.verticalCenter: parent.verticalCenter; on: sheet.fx.enabled; onToggled: sheet.change("enabled", !sheet.fx.enabled) }
                IconButton { icon: "close"; tip: qsTr("Close"); onClicked: Nav.audio = false }
            }
        }

        IosSegment {
            id: tabs
            anchors { top: head.bottom; topMargin: Theme.spaceXl; left: head.left }
            width: Theme.segmentNarrow
            current: sheet.tab
            options: [{ key: "eq", label: qsTr("Equalizer") }, { key: "fx", label: qsTr("Effects") }]
            onPicked: key => sheet.tab = key
        }

        Item {
            anchors { top: tabs.bottom; topMargin: Theme.spaceXl; left: head.left; right: head.right; bottom: reset.top; bottomMargin: Theme.spaceLg }
            enabled: sheet.fx.enabled
            opacity: enabled ? 1 : Theme.disabledOpacity
            Behavior on opacity { NumberAnimation { duration: Theme.normal } }

            EqPane { width: parent.width; fx: sheet.fx; visible: sheet.tab === "eq"; onChanged: (k, v) => sheet.change(k, v) }
            FxPane { width: parent.width; fx: sheet.fx; visible: sheet.tab === "fx"; onChanged: (k, v) => sheet.change(k, v) }
        }

        PillButton {
            id: reset
            anchors { right: head.right; bottom: parent.bottom; bottomMargin: Theme.spaceXl }
            text: qsTr("Reset")
            icon: "refresh"
            onClicked: {
                sheet.fx = { enabled: true, preamp: 0, eq: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0], bass: 0, speed: 1, pitchLock: false, reverb: 0, echo: 0, stereo: 1, normalize: false }
                push.restart()
            }
        }
    }
}
