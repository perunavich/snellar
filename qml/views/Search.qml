pragma ComponentBehavior: Bound
import QtQuick
import Snellar

Item {
    id: view
    property var params: ({})
    property string filter: "all"
    readonly property string q: params.q || ""
    readonly property var result: res.data
    property string openedLink: ""
    // Once a search failed over another search's results, those stay hidden until a search of its own answers.
    property bool strayed: false
    readonly property bool showing: !!result && (res.own || !(res.code || res.waiting || strayed))

    function remember() { return { filter: filter, openedLink: openedLink } }
    function recall(memory) {
        filter = memory.filter
        openedLink = memory.openedLink
    }

    onResultChanged: Qt.callLater(follow)
    function follow() {
        if (!result || openedLink === q) return
        if (result.linked) {
            openedLink = q
            Api.play([result.linked], 0)
        } else if (result.page) {
            openedLink = q
            const views = { album: ["album", {}], artist: ["artist", {}], playlist: ["album", { playlist: true }] }
            const [name, extra] = views[result.page.kind]
            Nav.replace(name, Object.assign({ id: result.page.id }, extra))
        }
    }

    Fetch {
        id: res
        method: "search"
        keepStale: true
        args: ({ q: view.q, filter: view.filter })
        ready: view.q.length > 0
        onCodeChanged: if (code && !own) view.strayed = true
        onOwnChanged: if (own) view.strayed = false
    }

    PageScroll {
        anchors.fill: parent

        Column {
            width: parent.width
            spacing: Theme.spaceLg

            PageTitle {
                id: heading
                width: Math.min(implicitWidth, parent.width)
                text: view.q ? qsTr("Results for “%1”").arg(view.q) : qsTr("Search")
            }
            Item {
                width: Math.min(Theme.searchFilterWidth, parent.width)
                visible: view.q.length > 0
                height: segment.height

                IosSegment {
                    id: segment
                    width: parent.width
                    current: view.filter
                    options: [{ key: "all", label: qsTr("All") }, { key: "songs", label: qsTr("Songs") }, { key: "videos", label: qsTr("Videos") }, { key: "artists", label: qsTr("Artists") }, { key: "albums", label: qsTr("Albums") }, { key: "playlists", label: qsTr("Playlists") }]
                    onPicked: key => view.filter = key
                }
            }
        }

        ListSkeleton { width: parent.width; card: true; heading: true; visible: res.loading && !view.showing }

        ErrorState { width: parent.width; fetch: res }

        SearchRecents { width: parent.width; visible: !view.q }

        Column {
            width: parent.width
            spacing: Theme.sectionGap
            visible: view.showing
            opacity: res.loading ? 0.5 : 1
            Behavior on opacity { NumberAnimation { duration: Theme.fast } }

            Rectangle {
                id: topCard
                readonly property var hit: view.result ? view.result.top : null
                visible: !!hit
                width: Math.min(parent.width, Theme.topHitWidth)
                height: Theme.topHitHeight
                radius: Theme.rCard
                color: Theme.fillRaised
                scale: topLens.value

                PressLens { id: topLens; down: topArea.pressed; w: topCard.width; h: topCard.height }

                Cover {
                    id: topArt
                    x: Theme.spaceLg
                    anchors.verticalCenter: parent.verticalCenter
                    width: Theme.topHitCover
                    height: Theme.topHitCover
                    radius: topCard.hit && topCard.hit.type === "artist" ? width / 2 : Theme.rMd
                    url: topCard.hit ? topCard.hit.thumb : ""
                }
                Column {
                    anchors { left: topArt.right; leftMargin: Theme.spaceLg; right: topPlay.left; rightMargin: Theme.spaceSm; verticalCenter: parent.verticalCenter }
                    Text {
                        width: parent.width
                        text: topCard.hit ? (topCard.hit.name || topCard.hit.title) : ""
                        color: Theme.text
                        font.family: Theme.font
                        font.pixelSize: Theme.fsTitle1
                        font.weight: Font.DemiBold
                        elide: Text.ElideRight
                    }
                    Text {
                        width: parent.width
                        text: topCard.hit ? Api.hitCaption(topCard.hit) : ""
                        color: Theme.text2
                        font.pixelSize: Theme.fsCallout
                        wrapMode: Text.WordWrap
                        maximumLineCount: 2
                        elide: Text.ElideRight
                    }
                }
                MouseArea {
                    id: topArea
                    anchors.fill: parent
                    onClicked: Api.openHit(topCard.hit)
                }
                IconButton {
                    id: topPlay
                    anchors { right: parent.right; rightMargin: Theme.spaceLg; verticalCenter: parent.verticalCenter }
                    icon: "play"
                    tip: qsTr("Play")
                    size: Theme.controlLg
                    filledBg: true
                    onClicked: Api.playHit(topCard.hit)
                }
            }

            Column {
                width: parent.width
                visible: !!view.result && view.result.tracks.length > 0
                spacing: Theme.spaceSm
                SectionTitle { text: qsTr("Songs") }
                TrackList { width: parent.width; radio: true; tracks: view.result ? view.result.tracks : [] }
            }

            Column {
                width: parent.width
                visible: !!view.result && !!view.result.videos && view.result.videos.length > 0
                spacing: Theme.spaceSm
                SectionTitle { text: qsTr("Videos") }
                TrackList { width: parent.width; radio: true; tracks: view.result && view.result.videos ? view.result.videos : [] }
            }

            Shelf {
                width: parent.width
                title: qsTr("Albums")
                items: view.result ? view.result.albums : []
                delegate: Component { AlbumCard { required property var modelData; album: modelData } }
            }

            Shelf {
                width: parent.width
                title: qsTr("Artists")
                items: view.result ? view.result.artists : []
                delegate: Component { ArtistCard { required property var modelData; artist: modelData } }
            }

            Shelf {
                width: parent.width
                title: qsTr("Playlists")
                items: view.result && view.result.playlists ? view.result.playlists : []
                delegate: Component { AlbumCard { required property var modelData; album: modelData } }
            }

            Text {
                visible: !!view.result && !view.result.top && !(view.result.playlists && view.result.playlists.length) && !view.result.tracks.length && !(view.result.videos && view.result.videos.length) && !view.result.albums.length && !view.result.artists.length
                text: qsTr("Nothing found")
                color: Theme.text2
                font.pixelSize: Theme.fsBody
            }
        }
    }
}
