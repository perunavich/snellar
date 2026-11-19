import QtQuick
import Snellar

PopButton {
    icon: "sliders"
    tip: qsTr("Sound")
    active: held
    onClicked: Nav.audio = !Nav.audio
}
