import QtCore
import QtQuick
import QtQuick.Dialogs
import Snellar

Item {
    id: picker
    property string title
    property var nameFilters: []
    property bool folder: false
    property string start: ""
    property bool busy: false
    readonly property var dialog: folder ? folderDialog : fileDialog
    readonly property string home: decodeURIComponent(String(StandardPaths.writableLocation(StandardPaths.HomeLocation)).replace(/^file:\/\//, ""))
    readonly property string startPath: /^~(\/|$)/.test(start) ? home + start.slice(1) : start
    signal picked(string path)

    visible: false

    function open() {
        if (busy) return
        busy = true
        Api.call("pickPath", { title: title, folder: folder, filters: nameFilters, start: startPath }, (v, e) => {
            if (!picker) return
            busy = false
            if (e || !v || v.status === "unavailable") showOwnDialog()
            else if (v.status === "picked") picked(v.path)
        })
    }

    function showOwnDialog() {
        dialog.currentFolder = startPath ? "file://" + encodeURI(startPath) : StandardPaths.writableLocation(StandardPaths.HomeLocation)
        dialog.open()
    }

    function choose(url) { picked(decodeURIComponent(String(url).replace(/^file:\/\//, ""))) }

    FileDialog {
        id: fileDialog
        title: picker.title
        nameFilters: picker.nameFilters
        onAccepted: picker.choose(selectedFile)
    }

    FolderDialog {
        id: folderDialog
        title: picker.title
        onAccepted: picker.choose(selectedFolder)
    }
}
