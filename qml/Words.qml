pragma Singleton
import QtQuick

QtObject {
    function part(p) {
        switch (p) {
        case "morning": return qsTr("Morning", "part of the day")
        case "day": return qsTr("Day", "part of the day")
        case "evening": return qsTr("Evening", "part of the day")
        case "night": return qsTr("Night", "part of the day")
        default: return ""
        }
    }

    function habit(p, weekend) {
        const days = weekend === true ? "weekend" : (weekend === false ? "weekday" : "any")
        switch (p + " " + days) {
        case "morning weekday": return qsTr("Your weekday mornings")
        case "morning weekend": return qsTr("Your weekend mornings")
        case "morning any": return qsTr("Your mornings")
        case "day weekday": return qsTr("Your weekday afternoons")
        case "day weekend": return qsTr("Your weekend afternoons")
        case "day any": return qsTr("Your afternoons")
        case "evening weekday": return qsTr("Your weekday evenings")
        case "evening weekend": return qsTr("Your weekend evenings")
        case "evening any": return qsTr("Your evenings")
        case "night weekday": return qsTr("Your weekday nights")
        case "night weekend": return qsTr("Your weekend nights")
        default: return qsTr("Your nights")
        }
    }

    function mixOf(names) {
        if (!names || !names.length) return ""
        if (names.length === 1) return qsTr("Mix of %1").arg(names[0])
        return qsTr("Mix of %1, %2").arg(names[0]).arg(names[1])
    }

    function favouritesOf(names) {
        if (!names || !names.length) return ""
        if (names.length === 1) return qsTr("Favorites: %1").arg(names[0])
        return qsTr("Favorites: %1, %2").arg(names[0]).arg(names[1])
    }

    function likeOf(names) {
        if (!names || !names.length) return ""
        if (names.length === 1) return qsTr("Like %1", "a list of new songs like these artists").arg(names[0])
        return qsTr("Like %1, %2", "a list of new songs like these artists").arg(names[0]).arg(names[1])
    }

    function why(w) {
        if (!w) return ""
        switch (w.kind) {
        case "played": return qsTr("Because you played %1").arg(w.artist)
        case "liked": return qsTr("Because you liked %1").arg(w.artist)
        case "similar": return qsTr("Similar to %1").arg(w.artist)
        case "favourite": return qsTr("One of your favorites")
        case "mood": return qsTr("From the %1 mood").arg(w.name)
        default: return ""
        }
    }

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

    function span(seconds) {
        const minutes = Math.max(1, Math.round(seconds / 60))
        const hours = Math.floor(minutes / 60), rest = minutes % 60
        if (!hours) return qsTr("%1 min").arg(rest)
        return rest ? qsTr("%1 h %2 min").arg(hours).arg(rest) : qsTr("%1 h").arg(hours)
    }

    function songs(tracks) {
        const count = qsTr("%n song(s)", "", tracks.length)
        const seconds = tracks.reduce((sum, t) => sum + (t.duration > 0 ? t.duration : NaN), 0)
        return tracks.length && isFinite(seconds) ? count + " · " + span(seconds) : count
    }

    function audience(text) {
        const m = /^([\d.,]+)\s*([KMB]?)(?:\s+(monthly audience)|\s+subscribers?)?$/i.exec(String(text || "").trim())
        if (!m) return text || ""
        const shown = m[1] + m[2]
        const count = Math.round(parseFloat(m[1].replace(/,/g, "")) * { "": 1, K: 1e3, M: 1e6, B: 1e9 }[m[2].toUpperCase()]) || 0
        return m[3] ? qsTr("%1 monthly listener(s)", "", count).arg(shown) : qsTr("%1 subscriber(s)", "", count).arg(shown)
    }
}
