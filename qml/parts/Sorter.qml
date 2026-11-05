import QtQuick
import Snellar
import "../lists.js" as Lists

QtObject {
    property var items: []
    property string name
    property var options: []
    property string query: ""
    readonly property string natural: options.length ? options[0].key : ""
    property string key: natural
    property bool reversed: false
    readonly property bool custom: key === natural && !reversed && !query.trim()
    readonly property var shown: Lists.arrange(items, query, key, reversed, natural)
    readonly property var trackKeys: [{ key: "title", label: qsTr("Title") }, { key: "artist", label: qsTr("Artist") }, { key: "album", label: qsTr("Album") }, { key: "duration", label: qsTr("Duration") }]
    readonly property string label: (options.find(o => o.key === key) || { label: "" }).label

    function pick(k) {
        if (k === key) reversed = !reversed
        else {
            key = k
            reversed = false
        }
        Api.call("setSetting", { key: "sort." + name, value: { key: key, reversed: reversed } })
    }

    Component.onCompleted: Api.call("getSetting", { key: "sort." + name }, v => {
        if (!v || !options.some(o => o.key === v.key)) return
        key = v.key
        reversed = !!v.reversed
    })
}
