pragma Singleton
import QtQuick

QtObject {
    function text(code) {
        switch (code) {
        case "net.offline":
        case "net.dns": return qsTr("No internet connection. Check your network.")
        case "net.connect": return qsTr("The connection was refused or cut. A firewall or your provider may be blocking it.")
        case "net.tls": return qsTr("Couldn't make a secure connection. Something on your network may be getting in the way.")
        case "net.timeout": return qsTr("The server took too long to answer")
        case "net.blocked": return qsTr("The server refuses requests from your network. A proxy may help: Settings → Connections.")
        case "net.refused": return qsTr("The request was refused")
        case "net.limited": return qsTr("Too many requests. Wait a moment and try again.")
        case "net.server": return qsTr("The server is having problems. Try again later.")
        case "net.youtube_blocked": return qsTr("YouTube isn't reachable from your network. A VPN may help.")
        case "proxy.auth": return qsTr("The proxy needs a login. Add it to the proxy address in Settings → Connections.")
        case "proxy.failed": return qsTr("The proxy couldn't connect. Check it or turn it off in Settings → Connections.")
        case "proxy.down": return qsTr("The proxy isn't answering. Check that it's running or turn it off in Settings → Connections.")
        case "proxy.timeout": return qsTr("The proxy took too long to answer. Check it or turn it off in Settings → Connections.")
        case "proxy.blocked": return qsTr("The server refuses requests through your proxy. Try another proxy or turn it off in Settings → Connections.")
        case "fetch.cancelled": return qsTr("Stopped waiting")
        case "yt.parse": return qsTr("Snellar couldn't read YouTube's answer")
        case "yt.unavailable": return qsTr("This song isn't available")
        case "yt.age_restricted": return qsTr("YouTube plays 18+ songs only to signed-in users")
        case "yt.removed": return qsTr("YouTube removed this song or made it private")
        case "yt.region": return qsTr("This song isn't available in your country")
        case "ytdlp.missing": return qsTr("yt-dlp is missing. Get it in Settings → Dependencies.")
        case "ytdlp.failed": return qsTr("yt-dlp couldn't get the song")
        case "ytdlp.ready": return qsTr("yt-dlp is ready")
        case "ytdlp.current": return qsTr("yt-dlp is up to date")
        case "shell.blur": return qsTr("GNOME couldn't set up glass. Nothing else changed.")
        case "shell.blur_skipped": return qsTr("Blur my Shell wasn't installed, so nothing changed")
        case "shell.blur_undo": return qsTr("GNOME couldn't put the settings back. You can remove Blur my Shell in the Extensions app.")
        case "ytdlp.checksum": return qsTr("The downloaded yt-dlp was damaged, so Snellar didn't install it")
        case "download.no_tool": return qsTr("Downloads need yt-dlp, and Snellar couldn't get it. Check your connection and try again. Music still plays.")
        case "download.no_space": return qsTr("Not enough disk space. Snellar keeps 1 GB free and goes on once there's room.")
        case "download.folder_invalid": return qsTr("That's not a folder path. Use a full path, such as ~/Music/Snellar")
        case "download.list_set_aside": return qsTr("Saved download list couldn't be read and was set aside")
        case "library.locked": return qsTr("Another program is using your library")
        case "library.newer": return qsTr("Your library was saved by a newer Snellar")
        case "update.no_release": return qsTr("Snellar couldn't read the version of the latest release")
        case "update.checksum": return qsTr("The update download is damaged")
        case "update.broken": return qsTr("The update download won't unpack")
        case "update.place": return qsTr("Snellar can't write to the folder it's installed in")
        case "update.signature": return qsTr("The new version isn't signed by Snellar, so it wasn't installed")
        case "library.corrupt": return qsTr("Your library file is damaged")
        case "library.failed": return qsTr("Snellar can't read or save your library")
        case "audio.failed": return qsTr("The audio player stopped working. See Settings → Dependencies for details.")
        case "api.args": return qsTr("Invalid request")
        case "api.method": return qsTr("Unknown request")
        case "api.not_found": return qsTr("It's no longer there")
        case "api.window_only": return qsTr("Only the window can do this")
        case "source.unsupported": return qsTr("This isn't supported here")
        case "fx.reset": return qsTr("Audio effects were reset")
        case "audio.restart": return qsTr("Playback stopped. Press play to continue.")
        case "cover.unreadable": return qsTr("This image can't be a cover. Choose a PNG, JPEG or WebP up to 20 MB")
        case "import.blocked": return qsTr("This playlist may be private: the service won't show it without signing in. Export it to CSV with Exportify (Spotify) or TuneMyMusic (any service) and choose the file here.")
        case "import.unreadable": return qsTr("Snellar couldn't read the songs on this page. Export the playlist to CSV with Exportify (Spotify) or TuneMyMusic (any service) and choose the file here.")
        case "import.unsupported": return qsTr("That's not a playlist or album link from Spotify, Deezer, Apple Music, Yandex Music or SoundCloud")
        case "import.columns": return qsTr("This CSV file has no column with song titles. Export it again with Exportify or TuneMyMusic.")
        case "import.empty": return qsTr("There are no songs in it")
        case "shell.extension": return qsTr("GNOME couldn't add tray support. You can install AppIndicator support in the Extensions app.")
        case "import.file": return qsTr("Can't read this file. Choose a CSV file up to 5 MB")
        case "proxy.invalid": return qsTr("That's not a proxy address. Use http://host:port")
        default: return qsTr("Something went wrong")
        }
    }

    function toast(t) {
        const cause = text(t.cause)
        switch (t.code) {
        case "play.failed": return qsTr("Can't play: %1").arg(cause)
        case "autoplay.failed": return qsTr("Autoplay stopped: %1").arg(cause)
        case "library.unsaved": return qsTr("Can't open your library, so changes won't be saved: %1").arg(cause)
        case "ytdlp.fetching": return qsTr("Getting yt-dlp: %1%").arg(t.arg || "0")
        case "ytdlp.fetch_failed": return qsTr("Couldn't get yt-dlp: %1").arg(cause)
        case "download.done": return qsTr("Downloaded %1").arg(t.arg || "")
        case "download.failed": return qsTr("Couldn't download %1: %2").arg(t.arg || "").arg(cause)
        default: return text(t.code)
        }
    }

    function unreachable(code) {
        return ["net.offline", "net.dns", "net.connect", "net.tls", "net.youtube_blocked", "proxy.down"].indexOf(code) >= 0
    }

    function passing(code) {
        return ["net.limited", "net.server", "net.timeout", "proxy.timeout", "proxy.failed"].indexOf(code) >= 0
    }

    function libraryHelp(code) {
        switch (code) {
        case "library.corrupt": return qsTr("Snellar didn't change the file. Restore it from a backup or move it elsewhere, then try again.")
        case "library.locked": return qsTr("Close the other program. Snellar opens your library as soon as it's free.")
        case "library.newer": return qsTr("Snellar didn't change the file. Update Snellar to open it.")
        default: return qsTr("Snellar didn't change the file. Check the disk and the folder permissions, then try again.")
        }
    }

    function reply(v) { return text(v && v.code) }
}
