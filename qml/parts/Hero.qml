import QtQuick
import Snellar

Column {
    id: hero
    property string url
    property string glyph
    property string kind
    property string title
    property string subtitle
    property string description
    property var artists: []
    property int cover: Theme.heroCover
    property bool editingTitle: false
    property Item tools: null
    readonly property Item actionRow: buttons
    default property alias actions: buttons.data
    property alias titleEditor: titleSlot.data
    readonly property real toolsWidth: tools && tools.visible ? tools.width : 0
    readonly property bool toolsInline: buttons.width + Theme.spaceXl + toolsWidth <= width
    signal titleDoubleTapped()
    spacing: Theme.spaceXl

    onToolsChanged: if (tools) tools.parent = toolSlot

    Row {
        width: parent.width
        spacing: Theme.sectionGap

        Item {
            width: hero.cover
            height: hero.cover

            Cover {
                anchors.fill: parent
                visible: !hero.glyph
                radius: Theme.rArtL
                px: 400
                url: hero.url
            }
            Rectangle {
                anchors.fill: parent
                visible: !!hero.glyph
                radius: Theme.rArtL
                color: Theme.fillRaised
                Icon {
                    anchors.centerIn: parent
                    name: hero.glyph
                    size: Theme.heroGlyph
                    color: Theme.accent
                }
            }
        }

        Column {
            width: hero.width - hero.cover - parent.spacing
            anchors.bottom: parent.bottom
            spacing: Theme.spaceSm

            Text {
                visible: hero.kind.length > 0
                text: hero.kind.toUpperCase()
                color: Theme.text2
                font.pixelSize: Theme.fsCaption
                font.weight: Font.DemiBold
                font.letterSpacing: 1
            }
            Item {
                width: parent.width
                height: heading.height

                Text {
                    id: heading
                    width: parent.width
                    visible: !hero.editingTitle
                    text: hero.title
                    color: Theme.text
                    font.family: Theme.font
                    font.pixelSize: Theme.fsHero
                    font.weight: Font.Bold
                    elide: Text.ElideRight
                }
                Item {
                    width: heading.paintedWidth
                    height: heading.height
                    visible: titleSlot.children.length > 0 && !hero.editingTitle
                    MouseArea {
                        anchors.fill: parent
                        cursorShape: Qt.IBeamCursor
                        onDoubleClicked: hero.titleDoubleTapped()
                    }
                }
                Item {
                    id: titleSlot
                    anchors.fill: parent
                    visible: hero.editingTitle
                }
            }
            Row {
                width: parent.width

                ArtistLinks {
                    id: by
                    visible: hero.artists.length > 0
                    width: Math.min(implicitWidth, Math.max(parent.width / 2, parent.width - meta.implicitWidth))
                    artists: hero.artists
                    fontSize: Theme.fsBody
                    color: Theme.text
                }
                Text {
                    id: meta
                    width: parent.width - (by.visible ? by.width : 0)
                    text: (by.visible && hero.subtitle ? " · " : "") + hero.subtitle
                    color: Theme.text2
                    font.pixelSize: Theme.fsBody
                    elide: Text.ElideRight
                }
            }
            Text {
                width: parent.width
                visible: hero.description.length > 0
                text: hero.description
                color: Theme.text2
                font.pixelSize: Theme.fsCallout
                wrapMode: Text.WordWrap
                maximumLineCount: 2
                elide: Text.ElideRight
            }
        }
    }

    Item {
        width: parent.width
        height: hero.toolsInline || !hero.toolsWidth ? Math.max(buttons.height, hero.tools ? hero.tools.height : 0) : buttons.height + Theme.spaceMd + hero.tools.height

        Row {
            id: buttons
            y: hero.toolsInline ? (parent.height - height) / 2 : 0
            spacing: Theme.spaceMd
        }
        Item {
            id: toolSlot
            x: hero.toolsInline ? parent.width - width : 0
            y: hero.toolsInline ? (parent.height - height) / 2 : buttons.height + Theme.spaceMd
            width: hero.toolsWidth
            height: hero.tools ? hero.tools.height : 0
        }
    }
}
