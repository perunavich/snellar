import QtQuick
import Snellar

Item {
    id: frame
    readonly property bool maximized: !!Window.window && Window.window.visibility === Window.Maximized
    readonly property int edge: Theme.windowEdge
    readonly property int corner: Theme.windowCorner
    readonly property bool resizable: !maximized

    visible: Theme.frameless

    function toggleMaximized() {
        if (maximized) Window.window.showNormal()
        else Window.window.showMaximized()
    }

    Rectangle {
        anchors.fill: parent
        visible: !frame.maximized
        radius: Theme.windowRadius
        color: "transparent"
        border.width: 1
        border.color: Theme.hairline
    }

    WindowDrag {
        anchors { left: parent.left; right: parent.right; top: parent.top }
        height: Theme.spaceXl
        visible: Nav.nowPlaying
        onDoubleTapped: frame.toggleMaximized()
    }

    WindowDrag {
        anchors { right: parent.right; top: parent.top }
        width: Theme.inspectorWidth + Theme.gap
        height: Theme.windowBarHeight
        onDoubleTapped: frame.toggleMaximized()
    }

    Row {
        id: controls
        objectName: "windowControls"
        anchors { right: parent.right; top: parent.top; margins: Theme.spaceSm }
        spacing: Theme.spaceXs

        IconButton {
            icon: "minimize"
            tip: qsTr("Minimize")
            size: Theme.windowButton
            iconSize: Theme.iconXs
            onClicked: frame.Window.window.showMinimized()
        }
        IconButton {
            icon: frame.maximized ? "restore" : "maximize"
            tip: frame.maximized ? qsTr("Restore") : qsTr("Maximize")
            size: Theme.windowButton
            iconSize: Theme.iconXs
            onClicked: frame.toggleMaximized()
        }
        IconButton {
            icon: "close"
            tip: qsTr("Close window")
            size: Theme.windowButton
            iconSize: Theme.iconXs
            onClicked: frame.Window.window.close()
        }
    }

    Item {
        anchors.fill: parent
        visible: frame.resizable

        WindowEdge { x: frame.corner; width: parent.width - 2 * frame.corner; height: frame.edge; edges: Qt.TopEdge; shape: Qt.SizeVerCursor }
        WindowEdge { x: frame.corner; y: parent.height - height; width: parent.width - 2 * frame.corner; height: frame.edge; edges: Qt.BottomEdge; shape: Qt.SizeVerCursor }
        WindowEdge { y: frame.corner; width: frame.edge; height: parent.height - 2 * frame.corner; edges: Qt.LeftEdge; shape: Qt.SizeHorCursor }
        WindowEdge { x: parent.width - width; y: frame.corner; width: frame.edge; height: parent.height - 2 * frame.corner; edges: Qt.RightEdge; shape: Qt.SizeHorCursor }
        WindowEdge { width: frame.corner; height: frame.corner; edges: Qt.TopEdge | Qt.LeftEdge; shape: Qt.SizeFDiagCursor }
        WindowEdge { x: parent.width - width; width: frame.corner; height: frame.corner; edges: Qt.TopEdge | Qt.RightEdge; shape: Qt.SizeBDiagCursor }
        WindowEdge { y: parent.height - height; width: frame.corner; height: frame.corner; edges: Qt.BottomEdge | Qt.LeftEdge; shape: Qt.SizeBDiagCursor }
        WindowEdge { x: parent.width - width; y: parent.height - height; width: frame.corner; height: frame.corner; edges: Qt.BottomEdge | Qt.RightEdge; shape: Qt.SizeFDiagCursor }
    }
}
