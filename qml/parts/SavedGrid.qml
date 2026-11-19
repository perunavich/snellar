import QtQuick
import Snellar

Item {
    id: saved
    property var params: ({})
    property alias method: res.method
    property alias name: sorter.name
    property alias options: sorter.options
    property string title
    property string emptyIcon
    property string emptyText
    property Component card

    Fetch { id: res; live: true }

    Sorter {
        id: sorter
        items: res.data || []
    }

    PageScroll {
        anchors.fill: parent

        PageTitle { text: saved.title }

        ListTools { sorter: sorter; visible: !!res.data && res.data.length > 0 }

        ErrorState { width: parent.width; fetch: res }
        EmptyState { visible: !!res.data && res.data.length === 0 && !res.code; width: parent.width; icon: saved.emptyIcon; text: saved.emptyText }
        NoMatch { sorter: sorter; width: parent.width }

        CardGrid {
            width: parent.width
            model: sorter.shown
            delegate: saved.card
        }
    }
}
