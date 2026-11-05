import QtQuick
import QtCore
import Snellar
import "../files.js" as Files

Canvas {
    property string path
    property string sessionFile: StandardPaths.writableLocation(StandardPaths.HomeLocation) + "/.local/state/DankMaterialShell/session.json"
    readonly property int cols: 96
    readonly property int rows: 54

    width: cols
    height: rows
    visible: false
    renderStrategy: Canvas.Immediate

    function wallpaperOf(text) {
        try {
            const found = JSON.parse(text)?.wallpaperPath
            return typeof found === "string" ? found : ""
        } catch (e) {
            return ""
        }
    }

    function reload() {
        Files.read(sessionFile, text => {
            const next = wallpaperOf(text)
            if (next && next !== path) path = next
        })
    }

    onPathChanged: loadImage("file://" + path)
    onImageLoaded: requestPaint()

    onPaint: {
        const url = "file://" + path
        if (!path || !isImageLoaded(url)) return
        const ctx = getContext("2d")
        ctx.clearRect(0, 0, cols, rows)
        ctx.drawImage(url, 0, 0, cols, rows)
        const px = ctx.getImageData(0, 0, cols, rows).data
        const ls = []
        for (let i = 0; i < px.length; i += 4)
            ls.push((0.2126 * px[i] + 0.7152 * px[i + 1] + 0.0722 * px[i + 2]) / 255)
        ls.sort((a, b) => a - b)
        Theme.wallpaperLuma = ls[Math.floor(ls.length * 0.75)]
    }
}
