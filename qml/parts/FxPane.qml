pragma ComponentBehavior: Bound
import QtQuick
import Snellar

Column {
    id: pane
    property var fx
    signal changed(string key, var value)

    spacing: Theme.spaceMd

    IosSlider {
        width: parent.width
        label: qsTr("Bass")
        from: -10
        to: 10
        decimals: 1
        unit: qsTr(" dB")
        value: pane.fx.bass
        onMoved: v => pane.changed("bass", v)
    }
    IosSlider {
        width: parent.width
        label: qsTr("Speed")
        from: 0.5
        to: 2
        decimals: 2
        unit: "×"
        value: pane.fx.speed
        onMoved: v => pane.changed("speed", Math.round(v * 20) / 20)
    }
    FxSwitch { text: qsTr("Keep pitch"); on: pane.fx.pitchLock; onToggled: pane.changed("pitchLock", !pane.fx.pitchLock) }
    IosSlider {
        width: parent.width
        label: qsTr("Reverb")
        decimals: 2
        value: pane.fx.reverb
        onMoved: v => pane.changed("reverb", Math.round(v * 20) / 20)
    }
    IosSlider {
        width: parent.width
        label: qsTr("Echo")
        decimals: 2
        value: pane.fx.echo
        onMoved: v => pane.changed("echo", Math.round(v * 20) / 20)
    }
    IosSlider {
        width: parent.width
        label: qsTr("Stereo width")
        to: 2
        decimals: 2
        unit: "×"
        value: pane.fx.stereo
        onMoved: v => pane.changed("stereo", v)
    }
    FxSwitch { text: qsTr("Normalize volume"); on: pane.fx.normalize; onToggled: pane.changed("normalize", !pane.fx.normalize) }

    component FxSwitch: Item {
        id: sw
        property string text
        property alias on: toggle.on
        signal toggled()
        width: pane.width
        height: Theme.controlSm
        Text { anchors.verticalCenter: parent.verticalCenter; text: sw.text; color: Theme.text; font.pixelSize: Theme.fsBody }
        IosSwitch {
            id: toggle
            anchors { right: parent.right; verticalCenter: parent.verticalCenter }
            onToggled: sw.toggled()
        }
    }
}
