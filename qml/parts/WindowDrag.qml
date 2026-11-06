import QtQuick
import Snellar

Item {
    id: drag
    signal doubleTapped()
    signal moved()

    DragHandler {
        target: null
        enabled: Theme.frameless
        grabPermissions: PointerHandler.CanTakeOverFromHandlersOfDifferentType | PointerHandler.ApprovesTakeOverByAnything
        onActiveChanged: {
            if (!active) return
            drag.moved()
            drag.Window.window.startSystemMove()
        }
    }

    TapHandler {
        id: tap
        enabled: Theme.frameless
        gesturePolicy: TapHandler.ReleaseWithinBounds
        grabPermissions: PointerHandler.ApprovesTakeOverByAnything
        onTapped: if (tap.tapCount === 2) drag.doubleTapped()
    }
}
