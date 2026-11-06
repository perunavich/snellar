pragma ComponentBehavior: Bound
import QtQuick
import Snellar

Item {
    id: view
    property var params: ({})

    Fetch { id: home; method: "home"; live: false }

    property var order: []
    readonly property bool shown: !!home.data && (home.data.shelves.length > 0 || home.data.recent.length > 0)
    readonly property var wave: {
        const shelf = home.data ? home.data.shelves.find(s => s.tracks.length) : null
        const tracks = shelf ? shelf.tracks.slice(0, 10) : []
        return order.length === tracks.length ? order.map(i => tracks[i]) : tracks
    }

    function reshuffleWave() {
        const n = wave.length
        const idx = Array.from({ length: n }, (_, i) => i)
        for (let i = n - 1; i > 0; i--) {
            const j = Math.floor(Math.random() * (i + 1));
            [idx[i], idx[j]] = [idx[j], idx[i]]
        }
        order = idx
    }

    Component { id: trackCard; TrackCard { required property var modelData; track: modelData } }
    Component { id: albumCard; AlbumCard { required property var modelData; album: modelData } }
    Component { id: artistCard; ArtistCard { required property var modelData; artist: modelData } }

    PageScroll {
        anchors.fill: parent

        Item {
            width: parent.width
            height: Theme.controlLg

            Text {
                anchors.verticalCenter: parent.verticalCenter
                text: qsTr("Home")
                color: Theme.text
                font.family: Theme.font
                font.pixelSize: Theme.fsLargeTitle
                font.weight: Font.Bold
            }
            IconButton {
                anchors { right: parent.right; verticalCenter: parent.verticalCenter }
                icon: "refresh"
                tip: qsTr("Refresh")
                spinning: home.loading
                onClicked: {
                    view.order = []
                    home.method = "homeRefresh"
                    home.reload()
                }
            }
        }

        Skeleton { visible: home.loading && !home.data; width: parent.width; height: Theme.waveHeight; radius: Theme.rCard }

        ErrorState { width: parent.width; fetch: home; visible: !view.shown && (failed || home.waiting) }

        Row {
            id: homeNote
            width: parent.width
            spacing: Theme.spaceSm
            visible: view.shown && home.failure.length > 0
            Icon { anchors.verticalCenter: parent.verticalCenter; name: home.offline ? "offline" : "alert"; size: Theme.iconSm; color: Theme.text3 }
            Text {
                anchors.verticalCenter: parent.verticalCenter
                width: parent.width - Theme.iconSm - againLink.implicitWidth - 2 * homeNote.spacing
                text: Errors.text(home.failure)
                color: Theme.text2
                font.pixelSize: Theme.fsCaption
                wrapMode: Text.WordWrap
            }
            Link {
                id: againLink
                anchors.verticalCenter: parent.verticalCenter
                text: qsTr("Try again")
                color: Theme.accent
                font.pixelSize: Theme.fsCaption
                font.weight: Font.DemiBold
                onClicked: home.reload()
            }
        }

        Text {
            width: parent.width
            visible: !!home.data && !view.shown && !home.loading && !home.code
            text: qsTr("Nothing to show yet. Refresh, or search for something to play.")
            color: Theme.text2
            font.pixelSize: Theme.fsBody
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.WordWrap
        }

        Rectangle {
            width: parent.width
            height: Theme.waveHeight
            radius: Theme.rCard
            visible: view.wave.length > 0
            gradient: Gradient {
                orientation: Gradient.Horizontal
                GradientStop { position: 0; color: Theme.accentFill }
                GradientStop { position: 1; color: Theme.fillGroup }
            }

            Rectangle {
                id: go
                x: Theme.spaceXl
                y: Theme.spaceXl
                width: Theme.waveButton
                height: Theme.waveButton
                radius: Theme.waveButton / 2
                color: Theme.accent
                scale: goArea.pressed ? 0.94 : 1
                Behavior on scale { NumberAnimation { duration: Theme.normal; easing.type: Easing.BezierSpline; easing.bezierCurve: Theme.curveJelly } }
                readonly property bool live: Api.playingFrom("wave", view.wave)
                Icon { anchors.centerIn: parent; name: go.live && Api.playing ? "pause" : "play"; size: Theme.iconHero; color: Theme.accentInk }
                MouseArea {
                    id: goArea
                    anchors.fill: parent
                    cursorShape: Qt.PointingHandCursor
                    onClicked: go.live ? Api.toggle() : Api.play(view.wave, 0, "wave")
                }
            }

            Column {
                anchors { left: go.right; leftMargin: Theme.spaceXl; right: reshuffle.left; rightMargin: Theme.spaceLg; verticalCenter: go.verticalCenter }
                spacing: Theme.spaceXs
                Text { text: qsTr("Wave"); color: Theme.text; font.family: Theme.font; font.pixelSize: Theme.fsTitle1; font.weight: Font.Bold }
                Text { width: parent.width; text: qsTr("Up next, picked for you"); color: Theme.text2; font.pixelSize: Theme.fsBody; elide: Text.ElideRight }
            }

            IconButton {
                id: reshuffle
                anchors { right: parent.right; rightMargin: Theme.spaceXl; verticalCenter: go.verticalCenter }
                icon: "shuffle"
                tip: qsTr("New mix")
                filledBg: true
                onClicked: view.reshuffleWave()
            }

            ListView {
                anchors { left: parent.left; right: parent.right; top: go.bottom; bottom: parent.bottom; leftMargin: Theme.spaceXl; rightMargin: Theme.spaceXl; topMargin: Theme.spaceLg; bottomMargin: Theme.spaceLg }
                orientation: ListView.Horizontal
                spacing: Theme.spaceMd
                clip: true
                boundsBehavior: Flickable.StopAtBounds
                model: view.wave

                delegate: Item {
                    id: item
                    required property var modelData
                    required property int index
                    width: Theme.waveItem
                    height: ListView.view.height

                    Rectangle {
                        anchors.fill: parent
                        radius: Theme.rRow
                        color: hover.hovered ? Theme.fillHover : "transparent"
                    }
                    Cover {
                        id: art
                        x: Theme.spaceXs
                        anchors.verticalCenter: parent.verticalCenter
                        width: Theme.controlXl
                        height: Theme.controlXl
                        radius: Theme.rArtM
                        px: 96
                        url: item.modelData.thumb
                    }
                    Column {
                        anchors { left: art.right; leftMargin: Theme.spaceSm; right: parent.right; rightMargin: Theme.spaceSm; verticalCenter: parent.verticalCenter }
                        Text { width: parent.width; text: item.modelData.title; color: Theme.text; font.pixelSize: Theme.fsCallout; font.weight: Font.Medium; elide: Text.ElideRight }
                        Text { width: parent.width; text: Theme.artists(item.modelData); color: Theme.text2; font.pixelSize: Theme.fsCaption; elide: Text.ElideRight }
                    }
                    HoverHandler { id: hover; cursorShape: Qt.PointingHandCursor }
                    Tap { onTapped: Api.play(view.wave, item.index, "wave") }
                }
            }
        }

        Shelf {
            width: parent.width
            title: qsTr("Jump back in")
            items: home.data ? home.data.recent : []
            delegate: Component { RecentCard { required property var modelData; entry: modelData } }
        }

        Repeater {
            model: home.data ? home.data.shelves : []
            delegate: Shelf {
                required property var modelData
                width: parent.width
                title: Words.shelf(modelData)
                items: modelData.tracks.length ? modelData.tracks : (modelData.albums.length ? modelData.albums : modelData.artists)
                delegate: modelData.tracks.length ? trackCard : (modelData.albums.length ? albumCard : artistCard)
            }
        }
    }
}
