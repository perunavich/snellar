pragma ComponentBehavior: Bound
import QtQuick
import Snellar

Item {
    id: view
    property var params: ({})

    Fetch { id: home; method: "home"; live: false }

    property var picked: null
    property bool mixing: false
    property string novelty: "mix"
    readonly property bool shown: !!home.data && (home.data.shelves.length > 0 || home.data.recent.length > 0)
    readonly property var pick: picked || (home.data ? (home.data.shelves.find(s => s.kind === "daypart") || null) : null)
    readonly property var wave: pick ? pick.tracks : []
    readonly property var info: pick && pick.pick ? pick.pick : ({ part: "", habits: [], habit: false, weekend: null, artists: [] })
    readonly property var noveltyOf: ({ familiar: 0.1, mix: null, "new": 0.7 })
    readonly property var parts: ["morning", "day", "evening", "night"]

    property string mode: "mix"
    readonly property var leaders: {
        const count = {}, first = {}
        wave.forEach((t, i) => {
            const name = t.artists && t.artists.length ? t.artists[0].name : ""
            if (!name) return
            if (!(name in count)) { count[name] = 0; first[name] = i }
            count[name]++
        })
        return Object.keys(count).sort((a, b) => count[b] - count[a] || first[a] - first[b]).slice(0, 2)
    }

    property int asking: 0
    function ask(args) {
        mixing = true
        const asked = novelty
        const mine = ++asking
        Api.call("daypart", args, (shelf, error) => {
            if (mine !== view.asking) return
            mixing = false
            if (shelf && shelf.tracks.length) {
                picked = shelf
                mode = asked
            }
            else if (error) Api.toast(error)
        })
    }
    function reshuffleWave() { ask({ part: info.part, reshuffle: true }) }
    function choosePart(part) { if (part !== info.part) ask({ part: part }) }
    function chooseNovelty(key) {
        if (key === novelty) return
        novelty = key
        Api.call("getSetting", { key: "wave" }, saved => {
            const knobs = Object.assign({}, saved || {})
            if (noveltyOf[key] === null) delete knobs.novelty
            else knobs.novelty = noveltyOf[key]
            Api.call("setSetting", { key: "wave", value: knobs }, () => { if (view) view.ask({ part: view.info.part }) })
        })
    }
    property string clockPart: ""
    function followClock() {
        const was = clockPart
        clockPart = Api.part
        if (!was || was === clockPart) return
        asking++
        mixing = false
        picked = null
        mode = novelty
        home.method = "home"
        home.reload(true)
    }
    function wake() { skyRest.restart() }
    Component.onCompleted: {
        wake()
        clockPart = Api.part
        Api.call("getSetting", { key: "wave" }, saved => {
            const n = saved && typeof saved.novelty === "number" ? saved.novelty : null
            view.novelty = n === null ? "mix" : (n < 0.2 ? "familiar" : (n > 0.5 ? "new" : "mix"))
            if (!view.picked) view.mode = view.novelty
        })
    }
    Window.onActiveChanged: if (Window.active) wake()

    Timer { id: skyRest; interval: Motion.skyAwake }

    Connections {
        target: Api
        function onPartChanged() { view.followClock() }
    }

    Connections {
        target: Nav
        function onViewChanged() { if (Nav.view === "discover") view.wake() }
        function onNowPlayingChanged() { if (!Nav.nowPlaying) view.wake() }
    }

    Component { id: trackCard; TrackCard { required property var modelData; track: modelData } }
    Component { id: albumCard; AlbumCard { required property var modelData; album: modelData } }
    Component { id: artistCard; ArtistCard { required property var modelData; artist: modelData } }

    PageScroll {
        id: page
        anchors.fill: parent

        Item {
            width: parent.width
            height: Theme.controlLg

            PageTitle { id: pageTitle; text: qsTr("Home") }
            IconButton {
                anchors { right: parent.right; verticalCenter: pageTitle.verticalCenter }
                icon: "refresh"
                tip: qsTr("Refresh")
                spinning: home.loading
                onClicked: {
                    view.asking++
                    view.mixing = false
                    view.picked = null
                    view.mode = view.novelty
                    home.method = "homeRefresh"
                    home.reload()
                }
            }
        }

        Skeleton { visible: home.loading && !home.data; width: parent.width; height: Theme.waveHeight; radius: Theme.rCard }

        ErrorState { width: parent.width; fetch: home; compact: false; visible: !view.shown && (failed || home.waiting) }

        StaleNote { width: parent.width; fetch: home; visible: view.shown && home.failure.length > 0 }

        Text {
            width: parent.width
            visible: !!home.data && !view.shown && !home.loading && !home.code
            text: qsTr("Nothing here yet. Refresh or search for music.")
            color: Theme.text2
            font.pixelSize: Theme.fsBody
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.WordWrap
        }

        Item {
            id: card
            objectName: "pickCard"
            width: parent.width
            height: Theme.waveHeight
            visible: view.wave.length > 0
            readonly property real head: width - 3 * Theme.spaceXl - Theme.waveButton
            readonly property real room: head - Theme.spaceLg - controls.implicitWidth - (chip.visible ? Theme.spaceSm + chip.width : 0)
            readonly property real widestOfTwo: Math.max(favouritesTwo.advanceWidth, mixTwo.advanceWidth, newWidth.advanceWidth)
            readonly property real widestOfOne: Math.max(favouritesOne.advanceWidth, mixOne.advanceWidth, newWidth.advanceWidth)
            readonly property int names: widestOfTwo <= room ? 2 : 1
            readonly property bool roomy: room >= (view.info.habit ? widestOfOne : title.implicitWidth)
            readonly property var named: view.leaders.slice(0, names)

            TextMetrics { id: favouritesTwo; font: title.font; text: Words.favouritesOf(view.leaders) }
            TextMetrics { id: mixTwo; font: title.font; text: Words.mixOf(view.leaders) }
            TextMetrics { id: favouritesOne; font: title.font; text: Words.favouritesOf(view.leaders.slice(0, 1)) }
            TextMetrics { id: mixOne; font: title.font; text: Words.mixOf(view.leaders.slice(0, 1)) }
            TextMetrics { id: newWidth; font: title.font; text: qsTr("New for you") }

            HoverHandler { onHoveredChanged: if (hovered) view.wake() }

            DaypartBackdrop {
                objectName: "backdrop"
                anchors.fill: parent
                part: view.info.habit ? view.info.part : ""
                onPartChanged: view.wake()
                moving: skyRest.running && card.visible && !Motion.reduced && Window.active && Nav.view === "discover" && !Nav.nowPlaying && page.contentY < page.headroom + card.y + card.height
            }

            Rectangle {
                id: go
                x: Theme.spaceXl
                y: Theme.spaceXl
                width: Theme.waveButton
                height: Theme.waveButton
                radius: Theme.waveButton / 2
                color: Theme.accent
                scale: goLens.value
                PressLens { id: goLens; down: goArea.pressed; w: go.width; h: go.height }
                readonly property bool live: Api.playingFrom("wave", view.wave)
                Icon { anchors.centerIn: parent; name: go.live && Api.playing ? "pause" : "play"; size: Theme.iconHero; color: Theme.accentInk }
                MouseArea {
                    id: goArea
                    anchors.fill: parent
                    onClicked: go.live ? Api.toggle() : Api.play(view.wave, 0, "wave")
                }
            }

            Column {
                id: lead
                anchors { left: go.right; leftMargin: Theme.spaceXl; right: card.roomy ? controls.left : parent.right; rightMargin: card.roomy ? Theme.spaceLg : Theme.spaceXl; verticalCenter: go.verticalCenter }
                spacing: Theme.spaceXs

                Row {
                    id: titleLine
                    width: parent.width
                    spacing: Theme.spaceSm

                    Text {
                        id: title
                        objectName: "pickTitle"
                        anchors.verticalCenter: parent.verticalCenter
                        width: Math.min(implicitWidth, parent.width - (chip.visible ? chip.width + parent.spacing : 0))
                        text: !view.info.habit ? qsTr("Wave") : view.mode === "new" ? qsTr("New for you") : view.mode === "familiar" ? Words.favouritesOf(card.named) : Words.mixOf(card.named)
                        color: Theme.text
                        font.family: Theme.font
                        font.pixelSize: Theme.fsTitle1
                        font.weight: Font.Bold
                        elide: Text.ElideRight
                    }
                    PopPill {
                        id: chip
                        objectName: "daypartChip"
                        anchors.verticalCenter: parent.verticalCenter
                        visible: view.info.habits.length > 0
                        height: Theme.segmentHeight
                        text: Words.part(view.info.part)
                        tail: "down"
                        onClicked: partMenu.toggle(chip, 0, chip.height + Theme.spaceXs)
                    }
                }
                Text {
                    id: reason
                    objectName: "pickReason"
                    width: parent.width
                    text: !view.info.habit ? qsTr("Picked for you") : view.mode === "new" ? Words.likeOf(view.info.artists) : Words.habit(view.info.part, view.info.weekend)
                    color: Theme.text2
                    font.pixelSize: Theme.fsBody
                    elide: Text.ElideRight
                }
            }

            RiffMenu {
                id: partMenu
                objectName: "daypartMenu"
                Instantiator {
                    model: view.parts
                    delegate: MenuRow {
                        required property string modelData
                        text: Words.part(modelData)
                        marked: modelData === view.info.part
                        onTriggered: view.choosePart(modelData)
                    }
                    onObjectAdded: (i, o) => partMenu.insertItem(i, o)
                    onObjectRemoved: (i, o) => partMenu.removeItem(o)
                }
            }

            Row {
                id: controls
                objectName: "pickControls"
                anchors { right: parent.right; rightMargin: Theme.spaceXl }
                y: Math.round(card.roomy ? lead.y + titleLine.y + (titleLine.height - height) / 2 : row.y + (row.height - height) / 2)
                spacing: Theme.spaceMd

                IosSegment {
                    id: knob
                    objectName: "novelty"
                    anchors.verticalCenter: parent.verticalCenter
                    width: Math.ceil(knob.fit)
                    current: view.novelty
                    options: [{ key: "familiar", label: qsTr("Familiar") }, { key: "mix", label: qsTr("Mix") }, { key: "new", label: qsTr("New") }]
                    onPicked: key => view.chooseNovelty(key)
                }
                IconButton {
                    anchors.verticalCenter: parent.verticalCenter
                    icon: "shuffle"
                    tip: qsTr("New mix")
                    filledBg: true
                    spinning: view.mixing
                    onClicked: view.reshuffleWave()
                }
            }

            Item {
                id: row
                objectName: "pickRow"
                anchors { left: parent.left; right: card.roomy ? parent.right : controls.left; top: go.bottom; leftMargin: Theme.spaceXl; rightMargin: card.roomy ? Theme.spaceXl : Theme.spaceLg; topMargin: Theme.spaceLg }
                height: Theme.controlXl + 2 * Theme.spaceXs
                readonly property int count: Math.max(1, Math.min(10, view.wave.length, Math.floor((width + Theme.spaceMd) / (Theme.waveItemMin + Theme.spaceMd))))
                readonly property real tile: Math.min(Theme.waveItemMax, (width - (count - 1) * Theme.spaceMd) / count)

                Row {
                    spacing: Theme.spaceMd

                    Repeater {
                        model: view.wave.slice(0, row.count)

                        delegate: Item {
                            id: item
                            required property var modelData
                            required property int index
                            objectName: "pickTile"
                            width: row.tile
                            height: row.height

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
                            HoverHandler { id: hover }
                            Tap { onTapped: Api.play(view.wave, item.index, "wave") }
                        }
                    }
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
            model: home.data ? home.data.shelves.filter(s => s.kind !== "daypart") : []
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
