import QtQuick
import Snellar

Row {
    id: hero
    property string url
    property string kind
    property string title
    property string subtitle
    property string description
    property var artists: []
    property int cover: Theme.heroCover
    default property alias actions: buttons.data
    spacing: Theme.sectionGap

    Cover {
        width: hero.cover
        height: hero.cover
        radius: Theme.rLg
        px: 400
        url: hero.url
    }

    Column {
        width: hero.width - hero.cover - hero.spacing
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
        Text {
            width: parent.width
            text: hero.title
            color: Theme.text
            font.family: Theme.font
            font.pixelSize: Theme.fsHero
            font.weight: Font.Bold
            elide: Text.ElideRight
        }
        Row {
            width: parent.width

            ArtistLinks {
                id: by
                visible: hero.artists.length > 0
                width: Math.min(implicitWidth, parent.width)
                artists: hero.artists
                fontSize: Theme.fsBody
                color: Theme.text
            }
            Text {
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
        Flow {
            id: buttons
            width: parent.width
            topPadding: Theme.spaceMd
            spacing: Theme.spaceMd
        }
    }
}
