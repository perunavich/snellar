import QtQuick

QtObject {
    id: watch
    property Item item: null
    property var chain: []
    signal moved()

    function moveSeen() { watch.moved() }

    function relink() {
        unlink()
        const seen = []
        for (let p = item; p; p = p.parent) {
            p.xChanged.connect(moveSeen)
            p.yChanged.connect(moveSeen)
            p.widthChanged.connect(moveSeen)
            p.heightChanged.connect(moveSeen)
            p.parentChanged.connect(relink)
            seen.push(p)
        }
        chain = seen
        moved()
    }

    function unlink() {
        for (const p of chain) {
            try {
                p.xChanged.disconnect(moveSeen)
                p.yChanged.disconnect(moveSeen)
                p.widthChanged.disconnect(moveSeen)
                p.heightChanged.disconnect(moveSeen)
                p.parentChanged.disconnect(relink)
            } catch (e) {
                continue
            }
        }
        chain = []
    }

    onItemChanged: relink()
    Component.onDestruction: unlink()
}
