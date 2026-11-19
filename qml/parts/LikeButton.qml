import QtQuick
import Snellar

IconButton {
    visible: !!Api.current
    icon: Api.liked ? "heartFill" : "heart"
    tip: Api.liked ? qsTr("Unlike") : qsTr("Like")
    active: Api.liked
    onClicked: Api.like(Api.current)
}
