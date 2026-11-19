pragma Singleton
import QtQuick
import Snellar

QtObject {
    id: api

    property var pending: ({})
    readonly property bool mock: Qt.application.arguments.indexOf("--mock=1") >= 0
    readonly property var mp: mock ? Qt.createComponent("dev/MockPlayer.qml").createObject(api) : null
    property var likedIds: ({})
    property var downloadedIds: ({})
    property var downloadingIds: ({})
    property var busyIds: ({})
    property var collections: ({})
    property var dislikedIds: ({})
    property var pins: []
    readonly property var direct: ["bugReport", "bugReportAttach", "bugReportPack", "reports"]
    readonly property var firstDesktop: parse(Core.desktop(), { desktop: "other", glass: "cover", blur: false })
    property var desktopInfo: firstDesktop
    property bool glassBusy: false
    property bool ytdlpBusy: false
    readonly property var registry: parse(Core.settings(), [])
    readonly property var defaults: registry.reduce((all, s) => Object.assign(all, { [s.key]: s.default }), {})
    property bool timeLeft: defaults.timeLeft === true
    property bool reduceMotion: defaults.reduceMotion === true
    property bool updateReady: false
    property var seen: ({})

    function setting(key) {
        const v = seen[key]
        return v === undefined || v === null ? defaults[key] : v
    }

    readonly property var current: mp ? mp.current : parse(Core.current, null)
    readonly property var queue: mp ? mp.queue : parse(Core.queue, [])
    readonly property int cursor: mp ? mp.cursor : Core.cursor
    readonly property bool playing: mp ? mp.playing : Core.playing
    readonly property bool loading: mp ? mp.loading : Core.loading
    readonly property real position: mp ? mp.position : Core.position
    readonly property real duration: mp ? mp.duration : Core.duration
    readonly property real volume: mp ? mp.volume : Core.volume
    readonly property bool shuffle: mp ? mp.shuffle : Core.shuffle
    readonly property string repeat: mp ? mp.repeat : Core.repeat
    readonly property bool liked: mp ? mp.liked : Core.liked
    readonly property var stream: mp ? mp.stream : parse(Core.stream, null)
    readonly property bool trayReady: mp ? mp.trayReady : Core.tray_ready
    readonly property var failure: mp ? mp.failure : parse(Core.failure, null)
    readonly property var libraryProblem: mp ? mp.libraryProblem : parse(Core.library, null)
    readonly property string reach: mp ? mp.reach : Core.reach
    readonly property string part: mp ? mp.part : Core.part
    onReachChanged: if (!reach) retry()
    property int retryBase: Theme.retryBase
    property double holdUntil: 0
    function hold(seconds) {
        if (seconds > 0) holdUntil = Math.max(holdUntil, Date.now() + seconds * 1000)
        return holdUntil
    }

    signal toast(string text)
    signal offer(string text, string actionText, var action)
    signal libraryChanged()
    signal playlistTracksRemoved(var id, var trackIds)
    signal historyChanged()
    signal toolsChanged()
    signal updateChanged()
    signal retry()
    signal settingSaved(string key, var value)

    property var stranded: []
    readonly property bool offline: stranded.length > 0
    property Timer retrying: Timer {
        interval: Theme.offlineRetry
        repeat: true
        running: api.offline
        onTriggered: api.retry()
    }
    function strand(fetch, cut) {
        const was = stranded.indexOf(fetch) >= 0
        if (cut && !was) stranded = stranded.concat([fetch])
        else if (!cut && was) stranded = stranded.filter(f => f !== fetch)
    }

    function parse(s, fallback) {
        try { return JSON.parse(s) } catch (e) { return fallback }
    }

    readonly property var quietly: ["suggest", "prefetch", "warm", "readingLyrics", "artistMix", "getSetting", "getFx", "favorites", "dislikes", "pins", "playlists", "savedPlaylists", "savedAlbums", "folders", "followed", "downloads", "downloadedTracks", "downloadedCollections", "downloadedSongs", "recentPlaylists", "playlistsWith", "audioDevices", "discord"]

    function answer(method, args, done, v, error, code, after, told, text) {
        if (error) {
            if (done && done.length >= 2) done(null, error, code, after)
            else if (!quietly.includes(method) && !told) api.toast(error)
            else if (done) done(null)
            return
        }
        if (method === "getSetting") seen[args.key] = v
        if (method === "setSetting") {
            seen[args.key] = args.value
            if (args.key === "reduceMotion") reduceMotion = args.value === true
            api.settingSaved(args.key, args.value)
        }
        if (done) done(v, null, "", 0, text)
    }

    function applyDesktop(info) {
        if (!info) return
        desktopInfo = info
        Motion.desktopReduced = info.reducedMotion === true
        if (info.finished) glassBusy = false
        Theme.desktopName = info.desktop
        if (Theme.autoGlass === info.glass) return
        if (Theme.glass === "auto") Theme.restyle(() => Theme.autoGlass = info.glass)
        else Theme.autoGlass = info.glass
    }

    function setupGlass() {
        glassBusy = true
        if (mp) mp.glassAsks++
        else Core.setupGlass()
    }

    function undoGlass() {
        glassBusy = true
        if (mp) mp.undoAsks++
        else Core.undoGlass()
    }

    function fetchYtdlp() {
        ytdlpBusy = true
        if (mp) mp.ytdlpAsks++
        else Core.fetchYtdlp()
    }

    function checkUpdate() {
        if (mp) mp.updateChecks++
        else Core.checkUpdate()
    }

    function restartForUpdate() {
        if (mp) mp.updateRestarts++
        else Core.restartForUpdate()
    }

    function addTray() {
        if (mp) mp.trayAsks++
        else Core.addTray()
    }

    function call(method, args, done) {
        if (picking.includes(method)) picks++
        if (mp && !direct.includes(method) && !(method === "pickPath" && !mp.data.pickAnswer)) {
            const answered = (v, error, code) => answer(method, args, done, v, error, code)
            if (mp.deferred(method, args || {}, answered)) return 0
            Qt.callLater(() => {
                let v = null
                try {
                    v = mp.handle(method, args || {})
                } catch (e) {
                    return answered(null, Errors.text(e.message), e.message)
                }
                answered(v, null)
            })
            return 0
        }
        const id = Core.request(method, JSON.stringify(args || {}))
        pending[id] = { method: method, args: args, done: done }
        return id
    }

    function peek(method, args, text) {
        if (mp) return mp.peek(method, args || {})
        const kept = Core.peek(method, JSON.stringify(args || {}))
        if (text && kept !== "") text.raw = kept
        return kept === "" ? undefined : parse(kept, undefined)
    }

    property int picks: 0
    readonly property var picking: ["play", "radio", "jump"]
    function later(then) {
        const pick = ++picks
        return v => { if (pick === picks) then(v) }
    }

    property string from: ""
    function play(tracks, index, source, origin) {
        from = source || ""
        call("play", { tracks: tracks, index: index || 0, context: origin || null })
    }
    function cardOf(kind, entity) {
        if (kind === "playlist") return { id: entity.id, name: entity.name }
        return { id: entity.id, title: entity.title, name: entity.name, artists: entity.artists, kind: kind === "remote" ? "Playlist" : entity.kind, year: entity.year, subtitle: entity.subtitle || entity.listeners || entity.subscribers, thumb: entity.thumb }
    }
    function origin(kind, entity) { return entity ? { kind: kind, item: cardOf(kind, entity) } : null }
    function playingFrom(source, tracks) {
        return source.length > 0 && from === source && !!current && (!tracks || tracks.some(t => t.id === current.id))
    }
    function clearQueue() { call("clearQueue", { keepCurrent: true }) }
    function radio(track) {
        from = ""
        call("radio", { track: track })
    }
    function playArtist(artist) {
        call("artistSongs", { id: artist.id }, later(songs => { if (songs && songs.length) play(songs, 0, "artist:" + artist.id, origin("artist", artist)) }))
    }
    function artistMix(artist, kind) {
        const source = kind === "shuffle" ? "artist:" + artist.id : ""
        call("artistMix", { id: artist.id, kind: kind }, later(mix => {
            if (mix && mix.length) return play(mix, 0, source, source ? origin("artist", artist) : null)
            call("artistSongs", { id: artist.id }, later(songs => {
                if (!songs || !songs.length) return
                if (kind === "radio") return radio(songs[0])
                shufflePlay(songs, source, origin("artist", artist))
            }))
        }))
    }
    function toggleRemote(list, done) {
        const card = { id: list.id, title: list.title, artists: list.artists || [], kind: "Playlist", thumb: list.thumb || "" }
        call("togglePlaylist", { playlist: card }, v => {
            if (done) done(v)
            api.libraryChanged()
        })
    }
    function hitCaption(hit, brief) {
        const kind = hit.type === "track" ? (hit.video ? qsTr("Video") : qsTr("Song")) : { album: qsTr("Album"), artist: qsTr("Artist"), playlist: qsTr("Playlist") }[hit.type]
        if (hit.type === "artist" && brief) return kind
        return [kind, hit.type === "artist" ? Words.audience(hit.subtitle) : Theme.artists(hit)].filter(x => x).join(" · ")
    }
    function openHit(hit) {
        if (hit.type === "track") radio(hit)
        else if (hit.type === "album") Nav.album(hit.id)
        else if (hit.type === "artist") Nav.artist(hit.id)
        else if (hit.type === "playlist") Nav.remote(hit.id)
    }
    function playHit(hit) {
        if (hit.type === "track") radio(hit)
        else if (hit.type === "artist") playArtist(hit)
        else call(hit.type === "album" ? "album" : "remotePlaylist", { id: hit.id }, later(v => { if (v && v.tracks.length) play(v.tracks, 0, "album:" + hit.id, origin(hit.type === "album" ? "album" : "remote", v)) }))
    }
    function follow(artist) {
        const card = { id: artist.id, name: artist.name, subtitle: artist.listeners || artist.subscribers || artist.subtitle || "", thumb: artist.thumb || "" }
        call("follow", { artist: card }, () => api.libraryChanged())
    }
    function toggle() { call("toggle") }
    function toggleShuffle() { call("shuffle", { on: !shuffle }) }
    function cycleRepeat() { call("repeat", { mode: repeat === "off" ? "all" : (repeat === "all" ? "one" : "off") }) }
    function shufflePlay(tracks, from, origin) {
        call("shuffle", { on: true })
        play(tracks, Math.floor(Math.random() * tracks.length), from, origin)
    }
    function next() { call("next") }
    function prev() { call("prev") }
    function seek(pos) { call("seek", { pos: pos }) }
    function setVolume(v) { call("volume", { value: v }) }
    readonly property int volumeStep: 5
    property real mutedFrom: 0
    function toggleMute() {
        if (volume > 0) { mutedFrom = volume; setVolume(0) }
        else setVolume(mutedFrom || 85)
    }
    function stepVolume(delta) {
        const v = Math.max(0, Math.min(100, Math.round(volume + delta)))
        if (v !== volume) setVolume(v)
    }
    function enqueue(tracks, playNext) {
        if (Array.isArray(tracks)) call("enqueue", { tracks: tracks, next: !!playNext })
        else call("enqueue", { track: tracks, next: !!playNext })
    }
    function addToPlaylist(list, tracks, done) {
        call("addToPlaylist", { id: list.id, tracks: tracks }, added => {
            api.libraryChanged()
            api.toast(added > 0 ? qsTr("Added to %1").arg(list.name) : qsTr("Already in %1").arg(list.name))
            if (done) done(added > 0)
        })
    }
    function playlistsWith(track, then) {
        if (!track) return then({})
        call("playlistsWith", { trackId: track.id }, v => then((v || []).reduce((all, id) => Object.assign(all, { [id]: true }), {})))
    }
    function downloadState(track) {
        if (!track) return ""
        if (downloadingIds[track.id] === true || busyIds[track.id] === true) return "downloading"
        return downloadedIds[track.id] === true ? "downloaded" : ""
    }
    function download(tracks) {
        const wanted = tracks.filter(t => !downloadState(t))
        if (!wanted.length) return
        const started = Object.assign({}, downloadingIds)
        for (const t of wanted) started[t.id] = true
        downloadingIds = started
        call("download", { tracks: wanted }, (v, error) => {
            if (!error) return
            api.toast(error)
            for (const t of wanted) downloadEnded(t.id)
        })
    }
    function downloadEnded(id) {
        const left = Object.assign({}, downloadingIds)
        delete left[id]
        downloadingIds = left
    }
    function hear(t) {
        if (t.track) {
            downloadEnded(t.track)
            refreshDownloaded()
        }
        api.toast(Errors.toast(t))
    }
    function refreshDownloaded() {
        call("downloadedTracks", {}, v => downloadedIds = (v || []).reduce((all, id) => Object.assign(all, { [id]: true }), {}))
    }
    function collection(kind, id) { return collections[kind + ":" + id] || null }
    function collectionPhase(c) {
        if (!c) return "off"
        if (c.asked) return "downloading"
        if (c.done + c.refused >= c.total) return "done"
        if (c.stalled === "download.no_space") return "full"
        if (c.stalled === "ytdlp.missing") return "tool"
        return c.stalled ? "waiting" : "downloading"
    }
    function collectionDone(kind, id) { return collectionPhase(collection(kind, id)) === "done" }
    function isKept(entry) { return !!collection(entry.kind, entry.id) }
    function cards(entries) { return entries.map(e => Object.assign({}, e.item, { type: e.kind, at: e.at })) }
    function downloadCollection(kind, entity, on) {
        const key = kind + ":" + entity.id
        const next = Object.assign({}, collections)
        if (on) next[key] = { kind: kind, id: String(entity.id), total: 0, done: 0, refused: 0, busy: 0, asked: true }
        else delete next[key]
        collections = next
        call("downloadCollection", { kind: kind, id: entity.id, on: on }, (v, error) => {
            if (error) api.toast(error)
            refreshCollections()
            api.libraryChanged()
        })
    }
    function refreshCollections() {
        call("downloadedCollections", {}, v => collections = (v || []).reduce((all, c) => Object.assign(all, { [c.kind + ":" + c.id]: c }), {}))
    }
    property Timer downloadsSettle: Timer {
        interval: Theme.debounce
        onTriggered: {
            api.refreshDownloaded()
            api.refreshCollections()
        }
    }
    function heardDownloads(busy) {
        busyIds = (busy || []).reduce((all, id) => Object.assign(all, { [id]: true }), {})
        if (!downloadsSettle.running) downloadsSettle.start()
    }
    function setLiked(tracks, on) { call("setLiked", { tracks: tracks, on: on }, () => api.libraryChanged()) }
    function like(track) { call("like", { track: track }, () => api.libraryChanged()) }
    property var lyricsViews: []
    function showingLyrics(view, on, away) {
        const was = lyricsViews.length > 0
        lyricsViews = lyricsViews.filter(v => v !== view).concat(on ? [view] : [])
        const now = lyricsViews.length > 0
        if (now !== was) call("readingLyrics", now ? { on: true } : { on: false, away: !!away })
    }

    property var prefetched: ({})
    property double lastPrefetch: 0

    function prefetch(track) {
        if (!track || prefetched[track.id]) return
        const now = Date.now()
        if (now - lastPrefetch < 800) return
        lastPrefetch = now
        prefetched[track.id] = true
        call("prefetch", { id: track.id })
    }

    function warm(page) { call("warm", { method: page.method, id: page.id }) }
    function warmTrack(track) {
        if (!track) return
        if (albumOf(track)) warm({ method: "album", id: albumOf(track) })
        const artist = (track.artists || []).find(a => !!a.id)
        if (artist) warm({ method: "artist", id: artist.id })
    }

    property TextEdit clipboard: TextEdit { visible: false }
    function copyText(text) {
        clipboard.text = text
        clipboard.selectAll()
        clipboard.copy()
    }

    function copyLink(kind, id) {
        call("link", { kind: kind, id: id }, url => {
            if (!url) return
            copyText(url)
            api.toast(qsTr("Link copied"))
        })
    }

    function deletePlaylist(list) {
        call("deletePlaylist", { id: list.id }, gone => {
            if (!gone) return
            if (Nav.view === "playlist" && Nav.params.id === gone.id) Nav.root("discover")
            api.libraryChanged()
            api.offer(qsTr("Deleted %1").arg(gone.name), qsTr("Undo"), () => call("restorePlaylist", { playlist: gone }, () => api.libraryChanged()))
        })
    }

    function setPlaylistCover(id, url) {
        call("editPlaylist", { id: id, cover: url }, () => api.libraryChanged())
    }

    function removeFromPlaylist(id, tracks) {
        const gone = tracks.map(t => t.id)
        call("removeFromPlaylist", { id: id, trackIds: gone }, () => api.playlistTracksRemoved(id, gone))
    }

    function showTimeLeft(on) {
        timeLeft = on
        call("setSetting", { key: "timeLeft", value: on })
    }

    function albumOf(track) { return track && track.album && track.album.id ? track.album.id : "" }

    function isPinned(kind, id) { return pins.some(p => p.kind === kind && p.id === String(id)) }
    function pin(kind, entity) {
        call("pin", { kind: kind, item: cardOf(kind, entity) }, () => api.libraryChanged())
    }

    function isLiked(track) { return !!track && likedIds[track.id] === true }
    function isDisliked(track) { return !!track && dislikedIds[track.id] === true }

    function dislike(track) {
        const wasLiked = isLiked(track)
        call("dislike", { track: track }, () => {
            api.libraryChanged()
            api.offer(qsTr("Disliked. It won't be recommended"), qsTr("Undo"), () => undislike(track, wasLiked))
        })
    }
    function undislike(track, relike) {
        call("undislike", { id: track.id }, () => {
            if (relike) like(track)
            else api.libraryChanged()
        })
    }

    function refreshLiked() {
        const byId = v => (v || []).reduce((all, t) => Object.assign(all, { [t.id]: true }), {})
        call("favorites", {}, v => likedIds = byId(v))
        call("dislikes", {}, v => dislikedIds = byId(v))
        refreshDownloaded()
        refreshCollections()
        call("pins", {}, v => { if (JSON.stringify(v || []) !== JSON.stringify(pins)) pins = v || [] })
    }

    onLibraryChanged: refreshLiked()
    Component.onCompleted: {
        refreshLiked()
        refreshUpdate()
        call("getSetting", { key: "timeLeft" }, v => { if (v !== null) timeLeft = v })
        call("getSetting", { key: "reduceMotion" }, v => { if (v !== null) reduceMotion = v === true })
        for (const s of registry) if (s.section) call("getSetting", { key: s.key })
    }

    onToolsChanged: ytdlpBusy = false

    function refreshUpdate() {
        call("updateStatus", {}, v => { updateReady = !!v && v.phase === "ready" })
    }
    onUpdateChanged: refreshUpdate()

    property Connections mockFeed: Connections {
        target: api.mp
        ignoreUnknownSignals: true
        function onPlayed() { api.historyChanged() }
        function onTold(t) { api.hear(t) }
        function onDownloadsChanged(busy) { api.heardDownloads(busy) }
    }

    property Connections link: Connections {
        target: Core
        function onReply(id, ok, json) {
            const asked = api.pending[id]
            if (!asked) return
            delete api.pending[id]
            const v = api.parse(json, null)
            if (ok) api.answer(asked.method, asked.args, asked.done, v, null, "", 0, false, json)
            else api.answer(asked.method, asked.args, asked.done, null, Errors.reply(v), v && v.code, v && v.retryAfter, v && v.told)
        }
        function onToast(json) { api.hear(api.parse(json, {})) }
        function onDownloadsChanged(json) { api.heardDownloads(api.parse(json, [])) }
        function onHistoryChanged() { api.historyChanged() }
        function onToolsChanged() { api.toolsChanged() }
        function onUpdateChanged() { api.updateChanged() }
        function onDesktopChanged(json) { api.applyDesktop(api.parse(json, null)) }
        function onNudge(json) {
            const n = api.parse(json, null)
            if (!n) return
            if (n.kind === "handoff") n.title = qsTr("Continue %1 in Snellar?").arg(n.track)
            const labels = { "continue": qsTr("Continue"), "ignore": qsTr("Not now") }
            n.actions = (n.actions || []).map(a => Object.assign({ label: labels[a.id] || a.id }, a, { run: () => api.call("nudgeAnswer", { id: n.id, action: a.id }) }))
            Nudges.show(n)
        }
    }
}
