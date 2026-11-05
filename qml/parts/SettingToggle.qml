import QtQuick
import Snellar

IosSwitch {
    id: toggle
    property string key

    on: Api.defaults[key] === true
    Component.onCompleted: Api.call("getSetting", { key: key }, v => { if (v !== null) toggle.on = v })
    onToggled: {
        on = !on
        Api.call("setSetting", { key: key, value: on })
    }
}
