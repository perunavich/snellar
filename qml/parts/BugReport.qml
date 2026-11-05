import QtQuick
import Snellar

Item {
    id: report

    property Item stage
    property var window
    property string lastToast: ""

    function capture() {
        Api.call("bugReport", {}, (v, err) => { if (err) Api.toast(qsTr("Bug report failed: %1").arg(err)) })
    }

    function selected() {
        const focus = window.activeFocusItem
        if (focus && focus.selectedText) return focus.selectedText
        const stack = [window.contentItem]
        while (stack.length) {
            const item = stack.pop()
            if (item.visible && item.selectedText) return item.selectedText
            for (const child of item.children) stack.push(child)
        }
        return ""
    }

    function snapshot() {
        return {
            screen: Nav.view,
            params: Nav.params,
            overlays: { nowPlaying: Nav.nowPlaying, audio: Nav.audio, tab: Nav.tab },
            window: { width: window.width, height: window.height, compact: window.compact },
            selection: selected(),
            toast: lastToast,
            player: {
                current: Api.current ? { id: Api.current.id, title: Api.current.title, artists: Theme.artists(Api.current), duration: Api.current.duration } : null,
                queue: Api.queue.map(t => ({ id: t.id, title: t.title, artists: Theme.artists(t), duration: t.duration })),
                cursor: Api.cursor,
                position: Api.position,
                duration: Api.duration,
                playing: Api.playing,
                loading: Api.loading,
                volume: Api.volume,
                shuffle: Api.shuffle,
                repeat: Api.repeat
            }
        }
    }

    function pack(name, note) {
        Api.call("bugReportPack", { name: name, note: note, reveal: !Api.mock }, (v, err) => {
            if (err) Api.toast(qsTr("Could not pack the report: %1").arg(err))
            else if (!v.shown && !Api.mock) Api.toast(qsTr("Report packed: %1").arg(v.path))
        })
    }

    function saved(name, ui) {
        Api.call("bugReportAttach", { name: name, ui: ui })
        Nudges.show({
            id: "bug-" + name,
            group: "bugreport",
            front: true,
            title: qsTr("Bug report saved"),
            text: name + "\n" + qsTr("The screenshot and track list show what you are listening to."),
            timeout: 20,
            actions: [{ label: qsTr("Show file"), primary: true, run: text => report.pack(name, text) }],
            field: {
                placeholder: qsTr("What went wrong? Enter saves, Esc skips"),
                run: text => Api.call("bugReportAttach", { name: name, note: text })
            }
        })
    }

    Connections {
        target: Core
        function onCapture(dir) {
            const name = dir.replace(/^.*\//, "")
            const ui = report.snapshot()
            const started = report.stage.grabToImage(img => {
                ui.shot = img.saveToFile(dir + "/window.png")
                report.saved(name, ui)
            })
            if (!started) {
                ui.shot = false
                report.saved(name, ui)
            }
        }
    }

    Connections {
        target: Api
        function onToast(text) { report.lastToast = text }
    }
}
