import QtQuick

TapHandler {
    gesturePolicy: TapHandler.ReleaseWithinBounds
    grabPermissions: PointerHandler.ApprovesTakeOverByItems
}
