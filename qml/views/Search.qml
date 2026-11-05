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

    function remember() { return { filter: filter } }
    function recall(memory) { filter = memory.filter }

    onResultChanged: {
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
    }

    PageScroll {
        anchors.fill: parent

        Flow {
            width: parent.width
            spacing: Theme.spaceMd

            Text {
                id: heading
                width: Math.min(implicitWidth, parent.width)
                text: qsTr("Results for “%1”").arg(view.q)
                color: Theme.text
                font.family: Theme.font
                font.pixelSize: Theme.fsLargeTitle
                font.weight: Font.DemiBold
                elide: Text.ElideRight
            }
            Item {
                width: Theme.filterWidth
                height: Math.max(heading.height, segment.height)

                IosSegment {
                    id: segment
                    width: parent.width
                    anchors.verticalCenter: parent.verticalCenter
                    current: view.filter
                    options: [{ key: "all", label: qsTr("All") }, { key: "songs", label: qsTr("Songs") }, { key: "videos", label: qsTr("Videos") }, { key: "artists", label: qsTr("Artists") }, { key: "albums", label: qsTr("Albums") }]
                    onPicked: key => view.filter = key
                }
            }
        }

        ListSkeleton { width: parent.width; visible: res.loading && !view.result }

        ErrorState { width: parent.width; fetch: res }

        Column {
            width: parent.width
            spacing: Theme.sectionGap
            visible: !!view.result
            opacity: res.loading ? 0.5 : 1
            Behavior on opacity { NumberAnimation { duration: Theme.fast } }

            Rectangle {
                id: topCard
                readonly property var hit: view.result ? view.result.top : null
                visible: !!hit
                width: Math.min(parent.width, Theme.topHitWidth)
                height: Theme.topHitHeight
                radius: Theme.rLg
                color: Theme.fillRaised
                scale: topArea.pressed ? 0.98 : 1
                Behavior on scale { NumberAnimation { duration: Theme.normal; easing.type: Easing.BezierSpline; easing.bezierCurve: Theme.curveJelly } }

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
                        elide: Text.ElideRight
                    }
                }
                MouseArea {
                    id: topArea
                    anchors.fill: parent
                    cursorShape: Qt.PointingHandCursor
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
