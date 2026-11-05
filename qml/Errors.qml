pragma Singleton
import QtQuick

QtObject {
    function text(code) {
        switch (code) {
        case "net.offline":
        case "net.dns": return qsTr("Cannot reach the internet. Check your network or DNS.")
        case "net.connect": return qsTr("The connection to the service was refused or cut. A firewall or your provider may block it.")
        case "net.tls": return qsTr("A secure connection to the service failed. Something on your network may be intercepting it.")
        case "net.timeout": return qsTr("The service took too long to answer")
        case "net.blocked": return qsTr("The service refuses requests from your network. A proxy may help: Settings, Connections.")
        case "net.refused": return qsTr("The service refused the request")
        case "net.limited": return qsTr("Too many requests to the service. Wait a moment and try again.")
        case "net.server": return qsTr("The service is having problems. Try again in a while.")
        case "proxy.auth": return qsTr("The proxy asks for a login. Add it to the proxy address: Settings, Connections.")
        case "proxy.failed": return qsTr("The proxy could not open the connection to the service. Check the proxy or turn it off: Settings, Connections.")
        case "proxy.down": return qsTr("The proxy does not answer. Check that it is running, or turn it off: Settings, Connections.")
        case "proxy.timeout": return qsTr("No answer came through the proxy in time. Check the proxy or turn it off: Settings, Connections.")
        case "proxy.blocked": return qsTr("The service refuses requests coming through your proxy. Try another proxy or turn it off: Settings, Connections.")
        case "fetch.cancelled": return qsTr("Stopped waiting")
        case "yt.parse": return qsTr("YouTube answered in an unexpected way")
        case "yt.unavailable": return qsTr("This track is not available")
        case "ytdlp.missing": return qsTr("yt-dlp is not installed")
        case "ytdlp.failed": return qsTr("yt-dlp could not get the track")
        case "ytdlp.ready": return qsTr("yt-dlp is ready")
        case "ytdlp.current": return qsTr("yt-dlp is up to date")
        case "shell.blur": return qsTr("GNOME could not set up glass. Nothing else changed.")
        case "shell.blur_skipped": return qsTr("Blur my Shell was not installed, so nothing changed")
        case "shell.blur_undo": return qsTr("GNOME could not put the settings back. You can remove Blur my Shell from the Extensions app.")
        case "ytdlp.checksum": return qsTr("The downloaded yt-dlp failed its checksum, so it was not installed")
        case "download.no_tool": return qsTr("Downloads need yt-dlp and Snellar could not get it. Check your connection and try again. Playing music is not affected.")
        case "library.locked": return qsTr("The library is busy in another program")
        case "library.newer": return qsTr("The library was saved by a newer Snellar")
        case "library.corrupt": return qsTr("The library file is damaged")
        case "library.failed": return qsTr("The library cannot be read or written")
        case "audio.failed": return qsTr("The audio player (mpv) failed")
        case "api.args": return qsTr("Invalid request")
        case "api.method": return qsTr("Unknown request")
        case "api.not_found": return qsTr("It is no longer there")
        case "api.window_only": return qsTr("Only the window can do this")
        case "source.unsupported": return qsTr("This source cannot do that")
        case "fx.reset": return qsTr("Audio effects were reset")
        case "audio.restart": return qsTr("mpv stopped, it will restart on the next play")
        case "cover.unreadable": return qsTr("This image cannot be a cover: use a PNG, JPEG or WebP file up to 20 MB")
        case "import.blocked": return qsTr("The service does not show this playlist without signing in: it may be private. Export it to a CSV file with Exportify (Spotify) or TuneMyMusic (any service) and choose that file here.")
        case "import.unreadable": return qsTr("Snellar could not read the songs on this page. Export the playlist to a CSV file with Exportify (Spotify) or TuneMyMusic (any service) and choose that file here.")
        case "import.unsupported": return qsTr("This is not a playlist or album link from Spotify, Deezer, Apple Music, Yandex Music or SoundCloud")
        case "import.columns": return qsTr("This CSV file has no column with track names. Export it again with Exportify or TuneMyMusic.")
        case "import.empty": return qsTr("There are no songs in it")
        case "shell.extension": return qsTr("GNOME could not add tray support. You can install AppIndicator support from the Extensions app.")
        case "import.file": return qsTr("This file cannot be read: choose a CSV text file up to 5 MB")
        case "import.cancelled": return qsTr("Import cancelled")
        case "proxy.invalid": return qsTr("This is not a proxy address. Use http://host:port")
        default: return qsTr("Something went wrong")
        }
    }

    function toast(t) {
        const cause = text(t.cause)
        switch (t.code) {
        case "play.failed": return qsTr("Cannot play: %1").arg(cause)
        case "autoplay.failed": return qsTr("Autoplay stopped: %1").arg(cause)
        case "library.unsaved": return qsTr("The library cannot be opened, changes will not be saved: %1").arg(cause)
        case "ytdlp.fetching": return qsTr("Getting yt-dlp: %1%").arg(t.arg || "0")
        case "ytdlp.fetch_failed": return qsTr("Could not get yt-dlp: %1").arg(cause)
        case "download.done": return qsTr("Downloaded %1").arg(t.arg || "")
        case "download.failed": return qsTr("Download of %1 failed: %2").arg(t.arg || "").arg(cause)
        default: return text(t.code)
        }
    }

    function unreachable(code) {
        return ["net.offline", "net.dns", "net.connect", "net.tls", "proxy.down"].indexOf(code) >= 0
    }

    function passing(code) {
        return ["net.limited", "net.server", "net.timeout", "proxy.timeout", "proxy.failed"].indexOf(code) >= 0
    }

    function libraryHelp(code) {
        switch (code) {
        case "library.corrupt": return qsTr("Snellar did not change the file. Restore it from a backup or move it away, then try again.")
        case "library.locked": return qsTr("Close the other program. Snellar opens the library by itself once it is free.")
        case "library.newer": return qsTr("Snellar did not change the file. Update Snellar to open it.")
        default: return qsTr("Snellar did not change the file. Check the disk and the folder's permissions, then try again.")
        }
    }

    function reply(v) { return text(v && v.code) }
}
