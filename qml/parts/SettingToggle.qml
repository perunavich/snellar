import QtQuick
import Snellar

IosSwitch {
    id: toggle
    property string key
    property bool known: false

    on: Api.setting(key) === true
    animated: known
    Component.onCompleted: Api.call("getSetting", { key: key }, () => {
        toggle.on = Api.setting(toggle.key) === true
        toggle.known = true
    })
    onToggled: {
        on = !on
        Api.call("setSetting", { key: key, value: on })
    }
}
