pragma ComponentBehavior: Bound
import QtQuick
import Snellar

Item {
    id: cache
    property var urls: []
    property var shapes: []
    readonly property int held: kept.count

    visible: false

    onUrlsChanged: {
        const wanted = Array.from(new Set(urls.filter(u => !!u)))
        const have = []
        for (let i = 0; i < kept.count; i++) have.push(kept.get(i).url)
        for (const url of wanted) if (have.indexOf(url) < 0) kept.append({ url: url })
        for (let i = kept.count - 1; i >= 0; i--) if (wanted.indexOf(kept.get(i).url) < 0) kept.remove(i)
    }

    ListModel { id: kept }

    Repeater {
        model: kept

        Item {
            id: copy
            required property string url

            Repeater {
                model: cache.shapes

                Image {
                    required property var modelData
                    source: Theme.art(copy.url, modelData.px)
                    sourceSize: modelData.size
                    fillMode: modelData.crop ? Image.PreserveAspectCrop : Image.Stretch
                    asynchronous: true
                }
            }
        }
    }
}
