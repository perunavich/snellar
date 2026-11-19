pragma ComponentBehavior: Bound
import QtQuick
import Snellar

Column {
    id: tl
    property var tracks: []
    property bool numbered: false
    property bool showAlbum: true
    property bool radio: false
    property string from: ""
    property var origin: null

    Repeater {
        model: tl.tracks
        delegate: TrackRow {
            required property var modelData
            required property int index
            width: tl.width
            track: modelData
            list: tl.tracks
            rank: index
            numbered: tl.numbered
            showAlbum: tl.showAlbum
            radio: tl.radio
            from: tl.from
            origin: tl.origin
        }
    }
}
