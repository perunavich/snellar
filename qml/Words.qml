pragma Singleton
import QtQuick

QtObject {
    function shelf(s) {
        switch (s.kind) {
        case "forYou": return qsTr("For you")
        case "becauseLiked": return qsTr("Because you liked %1").arg(s.arg)
        case "discover": return qsTr("Discover")
        case "trending": return qsTr("Trending now")
        case "popularArtists": return qsTr("Popular artists")
        default: return s.title
        }
    }

    function kind(k) {
        switch (String(k || "").toLowerCase()) {
        case "album": return qsTr("Album")
        case "single": return qsTr("Single")
        case "ep": return qsTr("EP")
        case "playlist": return qsTr("Playlist")
        case "song": return qsTr("Song")
        case "video": return qsTr("Video")
        case "artist": return qsTr("Artist")
        default: return k || ""
        }
    }

    function subscribers(text) {
        const m = /^([\d.,]+\s*[KMB]?)(\s+subscribers?)?$/i.exec(String(text || "").trim())
        return m ? qsTr("%1 subscribers").arg(m[1]) : (text || "")
    }
}
