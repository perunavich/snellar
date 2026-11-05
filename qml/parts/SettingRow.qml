import QtQuick
import Snellar

Item {
    id: row
    property string title
    property string detail
    property var link: null
    property bool below: false
    property bool divided: false
    default property alias control: slot.data
    readonly property bool stacked: below || width - slot.width - Theme.spaceLg < Theme.settingTextMin

    width: parent.width
    height: Math.max(Theme.settingRow, 2 * Theme.spaceMd + (stacked ? words.height + Theme.spaceSm + slot.height : Math.max(words.height, slot.height)))

    Rectangle {
        visible: row.divided
        width: parent.width
        height: 1
        color: Theme.separator
    }

    Column {
        id: words
        y: row.stacked ? Theme.spaceMd : Math.round((row.height - height) / 2)
        width: row.stacked ? row.width : row.width - slot.width - Theme.spaceLg
        spacing: Theme.lineGap
        Text { width: parent.width; text: row.title; color: Theme.text; font.pixelSize: Theme.fsHeadline; font.weight: Font.Medium; wrapMode: Text.Wrap }
        Text { width: parent.width; visible: text.length > 0; text: row.detail; color: Theme.text2; font.pixelSize: Theme.fsCallout; wrapMode: Text.Wrap }
        Link {
            visible: !!row.link
            width: Math.min(implicitWidth, parent.width)
            elide: Text.ElideNone
            wrapMode: Text.Wrap
            text: row.link ? row.link.text : ""
            color: Theme.accent
            onClicked: Qt.openUrlExternally(row.link.url)
        }
    }

    Item {
        id: slot
        x: row.stacked ? 0 : row.width - width
        y: row.stacked ? words.y + words.height + Theme.spaceSm : Math.round((row.height - height) / 2)
        width: childrenRect.width
        height: childrenRect.height
    }
}
