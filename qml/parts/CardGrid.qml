import QtQuick
import Snellar

Item {
    id: grid
    property alias model: view.model
    property alias delegate: view.delegate
    property real spacing: Theme.spaceXl
    property real cardWidth: Theme.cardCover
    property real cardHeight: Theme.cardCover + Theme.cardText
    readonly property Flickable page: {
        for (let p = parent; p; p = p.parent) if (p instanceof Flickable) return p
        return null
    }
    readonly property int columns: Math.max(1, Math.floor((width + spacing) / (cardWidth + spacing)))
    readonly property int rows: Math.ceil(view.count / columns)
    readonly property real placeY: y + (parent ? parent.y : 0)
    readonly property real shift: page ? Math.max(0, Math.min(height - view.height, page.contentY - placeY)) : 0

    height: rows > 0 ? rows * (cardHeight + spacing) - spacing : 0
    onShiftChanged: view.contentY = shift

    GridView {
        id: view
        y: grid.shift
        width: grid.columns * cellWidth
        height: Math.min(grid.height, grid.page ? grid.page.height : grid.height)
        cellWidth: grid.cardWidth + grid.spacing
        cellHeight: grid.cardHeight + grid.spacing
        cacheBuffer: Math.round(height / 2)
        interactive: false
        reuseItems: true
        currentIndex: -1
        highlightFollowsCurrentItem: false
        boundsBehavior: Flickable.StopAtBounds
        onContentYChanged: if (contentY !== grid.shift) contentY = grid.shift
    }
}
