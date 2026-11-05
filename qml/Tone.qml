pragma Singleton
import QtQuick

QtObject {
    property real luma: 0.5
    property bool light: false
    property color average: Theme.toneDefault

    onLumaChanged: {
        if (!light && luma > Theme.lumaHigh) light = true
        else if (light && luma < Theme.lumaLow) light = false
    }
}
