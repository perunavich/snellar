import QtQuick

Item {
    id: edge
    required property int edges
    required property int shape

    HoverHandler { cursorShape: edge.shape }

    DragHandler {
        target: null
        grabPermissions: PointerHandler.CanTakeOverFromAnything
        onActiveChanged: if (active) edge.Window.window.startSystemResize(edge.edges)
    }
}
