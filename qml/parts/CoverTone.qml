import QtQuick
import Snellar

Canvas {
    property string source
    width: 16
    height: 16
    visible: false
    renderStrategy: Canvas.Immediate

    property string loaded

    onSourceChanged: {
        if (loaded) unloadImage(loaded)
        loaded = source
        if (!source) return
        loadImage(source)
        if (isImageLoaded(source)) requestPaint()
    }
    onImageLoaded: requestPaint()

    onPaint: {
        if (!source || !isImageLoaded(source)) return
        const ctx = getContext("2d")
        ctx.clearRect(0, 0, width, height)
        ctx.drawImage(source, 0, 0, width, height)
        const px = ctx.getImageData(0, 0, width, height).data
        let r = 0, g = 0, b = 0
        const n = px.length / 4
        for (let i = 0; i < px.length; i += 4) {
            r += px[i]
            g += px[i + 1]
            b += px[i + 2]
        }
        r /= n * 255
        g /= n * 255
        b /= n * 255
        Tone.average = Qt.rgba(r, g, b, 1)
        Tone.luma = 0.2126 * r + 0.7152 * g + 0.0722 * b
    }
}
