import QtQuick
import QtQuick.Controls
import Snellar

TextField {
    id: field
    property string key
    property string hint
    property string problem

    width: Theme.fieldWidth
    height: Theme.controlSm
    leftPadding: Theme.spaceMd
    placeholderText: hint
    placeholderTextColor: Theme.text3
    color: Theme.text
    font.pixelSize: Theme.fsCallout
    selectByMouse: true
    background: Rectangle { radius: height / 2; color: Theme.fillPress; border.width: field.problem ? 1 : 0; border.color: Theme.red }

    Component.onCompleted: Api.call("getSetting", { key: key }, v => { if (v) field.text = v })
    onTextEdited: problem = ""
    onEditingFinished: save()

    function assign(value) {
        text = value
        problem = ""
        save()
    }

    function save() {
        Api.call("setSetting", { key: key, value: text.trim() }, (v, e, code) => {
            field.problem = e ? code || "internal" : ""
            if (e) Api.toast(Errors.text(field.problem))
        })
    }
}
