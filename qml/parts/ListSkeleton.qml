pragma ComponentBehavior: Bound
import QtQuick
import Snellar

Skeleton {
    id: skeleton
    property bool hero: false
    property bool card: false
    property bool heading: false
    property bool grid: false
    property int rows: 8
    readonly property real lineHeight: Theme.spaceMd
    color: "transparent"
    radius: 0
    height: body.height

    component Bar: Rectangle {
        color: Theme.fillRaised
        radius: height / 2
    }

    Column {
        id: body
        width: parent.width

        Item {
            visible: skeleton.hero
            width: parent.width
            height: Theme.heroCover + Theme.spaceXl + Theme.controlLg + Theme.sectionGap

            Rectangle {
                width: Theme.heroCover
                height: Theme.heroCover
                radius: Theme.rArtL
                color: Theme.fillRaised
            }
            Column {
                x: Theme.heroCover + Theme.sectionGap
                y: Theme.heroCover - height
                width: parent.width - x
                spacing: Theme.spaceMd
                Bar { width: Math.min(parent.width, Theme.topHitWidth); height: Theme.fsHero - Theme.spaceSm }
                Bar { width: Math.min(parent.width, Theme.cardCover); height: skeleton.lineHeight }
            }
            Row {
                y: Theme.heroCover + Theme.spaceXl
                spacing: Theme.spaceMd
                Bar { width: Theme.topHitHeight; height: Theme.controlLg }
                Bar { width: Theme.topHitHeight; height: Theme.controlLg }
                Bar { width: Theme.controlLg; height: Theme.controlLg }
            }
        }

        Item {
            visible: skeleton.card
            width: parent.width
            height: Theme.topHitHeight + Theme.sectionGap
            Rectangle {
                width: Math.min(parent.width, Theme.topHitWidth)
                height: Theme.topHitHeight
                radius: Theme.rCard
                color: Theme.fillRaised
            }
        }

        Item {
            visible: skeleton.heading
            width: parent.width
            height: title.implicitHeight + Theme.spaceSm
            SectionTitle { id: title; visible: false; text: " " }
            Bar { y: (title.implicitHeight - height) / 2; width: Theme.cardCover; height: Theme.fsTitle1 - Theme.spaceSm }
        }

        Item {
            visible: skeleton.grid
            width: parent.width
            height: caption.implicitHeight + Theme.sectionGap
            Text { id: caption; visible: false; text: " "; font.pixelSize: Theme.fsCallout }
            Bar { y: (caption.implicitHeight - height) / 2; width: Theme.topHitHeight; height: skeleton.lineHeight }
        }

        Flow {
            visible: skeleton.grid
            width: parent.width
            spacing: Theme.spaceXl
            Repeater {
                model: skeleton.grid ? Math.max(1, Math.floor((skeleton.width + Theme.spaceXl) / (Theme.cardCover + Theme.spaceXl))) * 2 : 0
                delegate: Item {
                    objectName: "skeletonCard"
                    width: Theme.cardCover
                    height: Theme.cardCover + Theme.cardText
                    Rectangle { width: Theme.cardCover; height: Theme.cardCover; radius: Theme.rArtM; color: Theme.fillRaised }
                    Bar { y: Theme.cardCover + Theme.spaceMd; width: parent.width * 3 / 4; height: skeleton.lineHeight }
                    Bar { y: Theme.cardCover + Theme.spaceMd + skeleton.lineHeight + Theme.spaceSm; width: parent.width / 2; height: skeleton.lineHeight }
                }
            }
        }

        Repeater {
            model: skeleton.grid ? 0 : skeleton.rows
            delegate: Item {
                objectName: "skeletonRow"
                width: body.width
                height: Theme.rowHeight

                Rectangle {
                    id: cover
                    x: Theme.spaceMd
                    anchors.verticalCenter: parent.verticalCenter
                    width: Theme.rowCover
                    height: Theme.rowCover
                    radius: Theme.rArtS
                    color: Theme.fillRaised
                }
                Column {
                    anchors { left: cover.right; leftMargin: Theme.spaceMd; verticalCenter: parent.verticalCenter }
                    width: Math.min(parent.width - Theme.rowCover - 3 * Theme.spaceMd, Theme.topHitWidth)
                    spacing: Theme.spaceSm
                    Bar { width: parent.width * 2 / 3; height: skeleton.lineHeight }
                    Bar { width: parent.width / 3; height: skeleton.lineHeight }
                }
            }
        }
    }
}
