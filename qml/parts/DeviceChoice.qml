pragma ComponentBehavior: Bound
import QtQuick
import Snellar

PillButton {
    id: choice

    property var devices: []
    property string wanted: "auto"
    property string active: "auto"
    readonly property var picked: devices.find(d => d.name === wanted) || null
    readonly property bool missing: wanted !== "auto" && !picked

    function load() {
        Api.call("audioDevices", {}, v => {
            if (!v) return
            if (JSON.stringify(v.devices) !== JSON.stringify(choice.devices)) choice.devices = v.devices
            choice.wanted = v.wanted
            choice.active = v.active
        })
    }

    function pick(name) {
        wanted = name
        Api.call("setSetting", { key: "audioDevice", value: name }, () => choice.load())
    }

    text: wanted === "auto" ? qsTr("Automatic", "output device") : picked ? picked.description : qsTr("Not connected", "output device")
    icon: "volume"
    Component.onCompleted: load()
    onClicked: {
        load()
        menu.popup(choice, 0, choice.height + Theme.spaceXs)
    }

    RiffMenu {
        id: menu
        MenuRow {
            text: qsTr("Automatic", "output device")
            marked: choice.wanted === "auto"
            onTriggered: choice.pick("auto")
        }
        Instantiator {
            model: choice.devices
            delegate: MenuRow {
                required property var modelData
                text: modelData.description
                marked: modelData.name === choice.wanted
                onTriggered: choice.pick(modelData.name)
            }
            onObjectAdded: (i, o) => menu.insertItem(i + 1, o)
            onObjectRemoved: (i, o) => menu.removeItem(o)
        }
    }
}
