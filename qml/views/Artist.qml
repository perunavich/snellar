pragma ComponentBehavior: Bound
import QtQuick
import Snellar

Item {
    id: view
    property var params: ({})
    readonly property var artist: res.data
    readonly property string from: artist ? "artist:" + artist.id : ""
    readonly property var origin: Api.origin("artist", artist)
    property bool followed: false
    readonly property Item barHead: header
    readonly property string barTitle: artist ? artist.name : ""

    function checkFollowed() { if (artist) Api.call("followed", {}, v => { if (view && view.artist) view.followed = (v || []).some(a => a.id === view.artist.id) }) }
    onArtistChanged: checkFollowed()
    Connections { target: Api; function onLibraryChanged() { view.checkFollowed() } }

    Fetch { id: res; method: "artist"; args: ({ id: view.params.id }); ready: !!view.params.id }
    Fetch { id: songs; method: "artistSongs"; args: ({ id: view.params.id }); ready: !!view.artist }

    PageScroll {
        anchors.fill: parent
        padding: 0
        headroom: 0
        spacing: Theme.sectionGap

        Item {
            id: header
            width: parent.width
            height: Theme.bannerHeight(width, view.height)
            clip: true

            Image {
                readonly property real fill: implicitWidth > 0 && implicitHeight > 0 ? Math.max(header.width / implicitWidth, header.height / implicitHeight) : 0
                x: (header.width - width) / 2
                y: (header.height - height) * Theme.bannerFocal
                width: implicitWidth * fill
                height: implicitHeight * fill
                source: view.artist ? Theme.banner(view.artist.banner || view.artist.thumb) : ""
                sourceSize.width: Theme.bannerPx
                asynchronous: true
            }
            Rectangle {
                anchors.fill: parent
                gradient: Gradient {
                    GradientStop { position: 0; color: Theme.bannerTop }
                    GradientStop { position: 1; color: Theme.window }
                }
            }

            Column {
                anchors { left: parent.left; right: parent.right; bottom: parent.bottom; margins: Theme.space2xl }
                spacing: Theme.spaceSm
                Text {
                    width: parent.width
                    text: view.artist ? view.artist.name : ""
                    color: Theme.text
                    font.family: Theme.font
                    font.pixelSize: Theme.fsDisplay
                    font.weight: Font.Bold
                    elide: Text.ElideRight
                }
                Text {
                    text: view.artist ? Words.audience(view.artist.listeners || view.artist.subscribers) : ""
                    color: Theme.text2
                    font.pixelSize: Theme.fsBody
                }
                Row {
                    visible: !!view.artist
                    topPadding: Theme.spaceSm
                    spacing: Theme.spaceMd
                    PlayButtons {
                        tracks: songs.data || (view.artist ? view.artist.top : [])
                        from: view.from
                        origin: view.origin
                        ownShuffle: true
                        onShuffleRequested: Api.artistMix(view.artist, "shuffle")
                    }
                    ActionButton {
                        text: qsTr("Radio")
                        icon: "radio"
                        enabled: !!view.artist
                        onClicked: Api.artistMix(view.artist, "radio")
                    }
                    ActionButton {
                        text: view.followed ? qsTr("Following") : qsTr("Follow")
                        icon: view.followed ? "check" : "userPlus"
                        on: view.followed
                        enabled: !!view.artist
                        onClicked: Api.follow(view.artist)
                    }
                    MoreButton { page: true; kind: "artist"; entity: view.artist }
                }
            }
        }

        Column {
            x: Theme.space2xl
            width: parent.width - 2 * Theme.space2xl
            spacing: Theme.sectionGap

            ListSkeleton { width: parent.width; heading: true; visible: res.loading && !view.artist }
            ErrorState { width: parent.width; fetch: res }

            Column {
                width: parent.width
                spacing: Theme.spaceSm
                visible: !!view.artist && view.artist.top.length > 0
                Item {
                    width: parent.width
                    height: topTitle.height
                    SectionTitle { id: topTitle; text: qsTr("Top songs") }
                    Link {
                        anchors { right: parent.right; verticalCenter: parent.verticalCenter }
                        text: qsTr("Show all")
                        onClicked: Nav.go("songs", { id: view.artist.id })
                    }
                }
                TrackList { width: parent.width; numbered: true; from: view.from; origin: view.origin; tracks: view.artist ? view.artist.top : [] }
            }

            Shelf {
                width: parent.width
                title: qsTr("Albums")
                moreText: qsTr("Show all")
                onMore: Nav.discography(view.artist.id, "albums")
                items: view.artist ? view.artist.albums : []
                delegate: Component { AlbumCard { required property var modelData; album: modelData } }
            }
            Shelf {
                width: parent.width
                title: qsTr("Singles")
                moreText: qsTr("Show all")
                onMore: Nav.discography(view.artist.id, "singles")
                items: view.artist ? view.artist.singles : []
                delegate: Component { AlbumCard { required property var modelData; album: modelData } }
            }
            Shelf {
                width: parent.width
                title: qsTr("Fans also like")
                items: view.artist ? view.artist.related : []
                delegate: Component { ArtistCard { required property var modelData; artist: modelData } }
            }

            Column {
                width: parent.width
                spacing: Theme.spaceSm
                visible: !!view.artist && view.artist.description.length > 0
                SectionTitle { text: qsTr("About") }
                Text {
                    width: Math.min(parent.width, Theme.readingWidth)
                    text: view.artist ? view.artist.description : ""
                    color: Theme.text2
                    font.pixelSize: Theme.fsBody
                    wrapMode: Text.WordWrap
                    lineHeight: Theme.readingLine
                }
            }
        }
    }
}
