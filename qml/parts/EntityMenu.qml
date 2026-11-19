pragma ComponentBehavior: Bound
import QtQuick
import Snellar

RiffMenu {
    id: menu
    property string kind: "track"
    property var entity
    property int queueIndex: -1
    property var playlistId: null
    property bool played: false
    property var playlists: []
    property var holding: ({})
    property var savedIds: ({})
    property var folders: []

    readonly property bool isTrack: kind === "track"
    readonly property bool isTracks: kind === "tracks"
    readonly property var songs: isTracks && Array.isArray(entity) ? entity : isTrack && entity ? [entity] : []
    readonly property bool isAlbum: kind === "album"
    readonly property bool isArtist: kind === "artist"
    readonly property bool isPlaylist: kind === "playlist"
    readonly property bool isRemote: kind === "remote"
    readonly property string source: entity && !isTrack && !isTracks ? (isRemote ? "album" : kind) + ":" + entity.id : ""
    readonly property var credited: entity && entity.artists ? entity.artists : []
    readonly property bool severalArtists: (isTrack || isAlbum) && credited.length > 1

    onAboutToShow: {
        Api.call("playlists", {}, v => menu.playlists = v || [])
        menu.holding = ({})
        if (isTrack) Api.playlistsWith(entity, v => menu.holding = v)
        if (isPlaylist) Api.call("folders", {}, v => menu.folders = v || [])
        const byId = v => (v || []).reduce((all, a) => Object.assign(all, { [a.id]: true }), {})
        if (isAlbum) Api.call("savedAlbums", {}, v => menu.savedIds = byId(v))
        if (isArtist) Api.call("followed", {}, v => menu.savedIds = byId(v))
        if (isRemote) Api.call("savedPlaylists", {}, v => menu.savedIds = byId(v))
        if (isTrack) Api.warmTrack(entity)
    }

    function withTracks(then) {
        const e = entity
        if (isTrack || isTracks) then(songs)
        else if (isAlbum && e.tracks) then(e.tracks)
        else if (isAlbum) Api.call("album", { id: e.id }, v => { if (v) then(v.tracks) })
        else if (isArtist && e.top) then(e.top)
        else if (isArtist) Api.call("artist", { id: e.id }, v => { if (v) then(v.top) })
        else if (isPlaylist) Api.call("playlist", { id: e.id }, v => { if (v) then(v.tracks) })
        else if (isRemote && e.tracks) then(e.tracks)
        else if (isRemote) Api.call("remotePlaylist", { id: e.id }, v => { if (v) then(v.tracks) })
    }

    function addTo(list) { withTracks(tracks => Api.addToPlaylist(list, tracks)) }

    function addToNew() {
        const name = isTracks ? qsTr("New playlist") : entity.title || entity.name
        withTracks(tracks => Api.call("createPlaylist", { name: name }, id => Api.addToPlaylist({ id: id, name: name }, tracks)))
    }

    MenuRow {
        text: qsTr("%n song(s) selected", "", menu.songs.length)
        glyph: "check"
        shown: menu.isTracks
        available: false
    }
    MenuRow {
        text: qsTr("Play")
        glyph: "playSmall"
        shown: !menu.isTrack
        onTriggered: menu.isArtist ? Api.playArtist(menu.entity) : menu.withTracks(Api.later(t => { if (t.length) Api.play(t, 0, menu.source, menu.isTracks ? null : Api.origin(menu.kind, menu.entity)) }))
    }
    MenuRow {
        text: qsTr("Play next")
        glyph: "playNext"
        shown: !menu.isArtist
        onTriggered: menu.withTracks(t => Api.enqueue(t, true))
    }
    MenuRow {
        text: qsTr("Add to queue")
        glyph: "queue"
        shown: !menu.isArtist
        onTriggered: menu.withTracks(t => Api.enqueue(t, false))
    }
    MenuRow {
        text: qsTr("Start radio")
        glyph: "radio"
        shown: menu.isTrack || menu.isArtist
        onTriggered: menu.isArtist ? Api.artistMix(menu.entity, "radio") : Api.radio(menu.entity)
    }

    RiffMenu {
        id: sub
        nested: true
        title: qsTr("Add to playlist")
        glyph: "playlist"
        shown: menu.isTrack || menu.isTracks || menu.isAlbum || menu.isRemote
        divider: true

        MenuRow {
            text: qsTr("New playlist")
            glyph: "plus"
            onTriggered: menu.addToNew()
        }

        Instantiator {
            model: menu.playlists
            delegate: MenuRow {
                required property var modelData
                required property int index
                text: modelData.name
                glyph: "playlist"
                divider: index === 0
                marked: menu.holding[modelData.id] === true
                onTriggered: menu.addTo(modelData)
            }
            onObjectAdded: (i, o) => sub.insertItem(i + 1, o)
            onObjectRemoved: (i, o) => sub.removeItem(o)
        }
    }

    RiffMenu {
        id: folderSub
        nested: true
        title: qsTr("Move to folder")
        glyph: "folder"
        shown: menu.isPlaylist

        MenuRow {
            text: qsTr("New folder")
            glyph: "plus"
            onTriggered: Api.call("createFolder", { name: qsTr("New folder") }, folder => Api.call("moveToFolder", { id: menu.entity.id, folder: folder }, () => {
                Api.libraryChanged()
                Nav.renamingFolder = folder
            }))
        }
        MenuRow {
            text: qsTr("Remove from folder")
            glyph: "close"
            shown: !!menu.entity && !!menu.entity.folder
            onTriggered: Api.call("moveToFolder", { id: menu.entity.id, folder: null }, () => Api.libraryChanged())
        }

        Instantiator {
            model: menu.folders.filter(f => !menu.entity || f.id !== menu.entity.folder)
            delegate: MenuRow {
                required property var modelData
                required property int index
                text: modelData.name
                glyph: "folder"
                divider: index === 0
                onTriggered: Api.call("moveToFolder", { id: menu.entity.id, folder: modelData.id }, () => Api.libraryChanged())
            }
            onObjectAdded: (i, o) => folderSub.insertItem(i + 2, o)
            onObjectRemoved: (i, o) => folderSub.removeItem(o)
        }
    }

    MenuRow {
        readonly property bool liked: menu.songs.length > 0 && menu.songs.every(t => Api.isLiked(t))
        text: liked ? qsTr("Unlike") : qsTr("Like")
        glyph: liked ? "heartFill" : "heart"
        shown: menu.isTrack || menu.isTracks
        onTriggered: menu.isTracks ? Api.setLiked(menu.songs, !liked) : Api.like(menu.entity)
    }
    MenuRow {
        readonly property bool disliked: menu.isTrack && Api.isDisliked(menu.entity)
        text: disliked ? qsTr("Remove dislike") : qsTr("Dislike")
        glyph: "ban"
        shown: menu.isTrack
        onTriggered: disliked ? Api.undislike(menu.entity, false) : Api.dislike(menu.entity)
    }
    MenuRow {
        text: qsTr("Remove from history")
        glyph: "history"
        shown: menu.isTrack && !!menu.entity && (menu.played || (!!Api.current && menu.entity.id === Api.current.id))
        onTriggered: {
            const title = menu.entity.title
            Api.call("removeFromHistory", { id: menu.entity.id }, (v, error) => { if (!error) Api.toast(qsTr("Removed from history: %1").arg(title)) })
        }
    }
    MenuRow {
        readonly property bool saved: menu.isAlbum && !!menu.entity && menu.savedIds[menu.entity.id] === true
        text: saved ? qsTr("Remove from library") : qsTr("Save to library")
        glyph: "library"
        shown: menu.isAlbum
        onTriggered: Api.call("toggleAlbum", { album: menu.entity }, () => Api.libraryChanged())
    }
    MenuRow {
        readonly property bool saved: menu.isRemote && !!menu.entity && menu.savedIds[menu.entity.id] === true
        text: saved ? qsTr("Remove from library") : qsTr("Save to library")
        glyph: "library"
        shown: menu.isRemote
        onTriggered: Api.toggleRemote(menu.entity)
    }
    MenuRow {
        readonly property bool followed: menu.isArtist && !!menu.entity && menu.savedIds[menu.entity.id] === true
        glyph: followed ? "userMinus" : "userPlus"
        text: followed ? qsTr("Unfollow") : qsTr("Follow")
        shown: menu.isArtist
        onTriggered: Api.follow(menu.entity)
    }
    MenuRow {
        readonly property bool pinned: !!menu.entity && Api.isPinned(menu.kind, menu.entity.id)
        text: pinned ? qsTr("Unpin from sidebar") : qsTr("Pin to sidebar")
        glyph: "pin"
        shown: !menu.isTrack && !menu.isTracks
        onTriggered: Api.pin(menu.kind, menu.entity)
    }
    MenuRow {
        text: qsTr("Remove from this playlist")
        glyph: "trash"
        shown: (menu.isTrack || menu.isTracks) && menu.playlistId !== null
        onTriggered: Api.removeFromPlaylist(menu.playlistId, menu.songs)
    }
    MenuRow {
        text: qsTr("Remove from queue")
        glyph: "close"
        shown: menu.isTrack && menu.queueIndex >= 0
        onTriggered: Api.call("removeQueued", { index: menu.queueIndex, id: menu.entity.id })
    }
    MenuRow {
        readonly property var states: menu.songs.map(t => Api.downloadState(t))
        readonly property bool waiting: states.indexOf("") < 0
        readonly property bool busy: waiting && states.indexOf("downloading") >= 0
        text: busy ? qsTr("Downloading") : waiting ? qsTr("Downloaded") : qsTr("Download")
        glyph: "download"
        shown: menu.isTrack || menu.isTracks
        available: !waiting
        marked: waiting && !busy
        onTriggered: Api.download(menu.songs)
    }
    MenuRow {
        readonly property bool kept: !!menu.entity && !!Api.collection(menu.kind, menu.entity.id)
        text: kept ? qsTr("Remove download") : qsTr("Download")
        glyph: "download"
        shown: menu.isPlaylist || menu.isRemote || menu.isAlbum
        onTriggered: Api.downloadCollection(menu.kind, menu.entity, !kept)
    }

    MenuRow {
        text: qsTr("Go to album")
        glyph: "disc"
        shown: menu.isTrack
        divider: true
        available: !!menu.entity && !!menu.entity.album && !!menu.entity.album.id
        onTriggered: Nav.album(menu.entity.album.id)
    }
    MenuRow {
        text: qsTr("Go to artist")
        glyph: "user"
        shown: (menu.isTrack || menu.isAlbum) && !menu.severalArtists
        divider: menu.isAlbum
        available: menu.credited.length === 1 && !!menu.credited[0].id
        onTriggered: Nav.artist(menu.credited[0].id)
    }
    RiffMenu {
        id: artistSub
        nested: true
        title: qsTr("Go to artist")
        glyph: "user"
        shown: menu.severalArtists
        divider: menu.isAlbum

        Instantiator {
            model: menu.severalArtists ? menu.credited : []
            delegate: MenuRow {
                required property var modelData
                text: modelData.name
                glyph: "user"
                available: !!modelData.id
                onTriggered: Nav.artist(modelData.id)
            }
            onObjectAdded: (i, o) => artistSub.insertItem(i, o)
            onObjectRemoved: (i, o) => artistSub.removeItem(o)
        }
    }

    MenuRow {
        text: qsTr("Use as playlist cover")
        glyph: "image"
        shown: menu.isTrack && menu.playlistId !== null
        divider: true
        available: !!menu.entity && !!menu.entity.thumb
        onTriggered: Api.setPlaylistCover(menu.playlistId, menu.entity.thumb)
    }
    MenuRow {
        text: qsTr("Copy link")
        glyph: "link"
        shown: menu.isAlbum || menu.isArtist || menu.isRemote
        divider: true
        onTriggered: Api.copyLink(menu.isRemote ? "playlist" : menu.kind, menu.entity.id)
    }
    MenuRow {
        text: qsTr("Rename")
        glyph: "rename"
        shown: menu.isPlaylist
        divider: true
        onTriggered: Nav.rename(menu.entity.id)
    }
    MenuRow {
        text: qsTr("Edit details")
        glyph: "edit"
        shown: menu.isPlaylist
        onTriggered: Nav.edit(menu.entity.id)
    }
}
