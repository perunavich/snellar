import QtQuick
import QtQuick.Controls
import Snellar

Item {
    id: host
    property var spec: null
    property Item before: null
    readonly property real p: spring.value

    width: Theme.inspectorWidth
    height: card.height
    visible: p > 0.002
    opacity: Math.min(1, p * 1.5)
    transform: Translate { y: (1 - host.p) * Theme.spaceXl }

    Connections {
        target: Nudges
        function onCurrentChanged() {
            if (Nudges.current) host.spec = Nudges.current
            if (Nudges.current && Nudges.current.field) {
                if (!host.before && !field.activeFocus) host.before = host.Window.window.activeFocusItem
                field.text = ""
                field.forceActiveFocus()
            } else {
                field.focus = false
                if (host.before) host.before.forceActiveFocus()
                host.before = null
            }
        }
    }

    Binding { target: Nudges; property: "typing"; value: field.activeFocus }
    Binding { target: Nudges; property: "draft"; value: field.text }

    SpringValue {
        id: spring
        zeta: Nudges.current ? Theme.springBase[0] : Theme.springCritical[0]
        response: Nudges.current ? Theme.springBase[1] : Theme.springCritical[1]
        target: Nudges.current ? 1 : 0
    }

    HoverHandler { id: hover; onHoveredChanged: Nudges.paused = hovered }

    Glass {
        id: card
        width: parent.width
        height: body.height + 2 * Theme.spaceLg
        radius: Theme.rCard
        color: Theme.chromeStrong
    }

    Column {
        id: body
        x: Theme.spaceLg
        y: Theme.spaceLg
        width: parent.width - 2 * Theme.spaceLg
        spacing: Theme.spaceMd

        Row {
            width: parent.width
            spacing: Theme.spaceMd

            Cover {
                visible: !!host.spec && !!host.spec.cover
                width: Theme.controlXl
                height: Theme.controlXl
                radius: Theme.rArtM
                px: 96
                url: host.spec && host.spec.cover ? host.spec.cover : ""
            }

            Column {
                width: parent.width - (host.spec && host.spec.cover ? Theme.controlXl + Theme.spaceMd : 0) - Theme.controlSm
                spacing: Theme.spaceXs

                Text {
                    width: parent.width
                    text: host.spec ? host.spec.title : ""
                    color: Theme.text
                    font.pixelSize: Theme.fsHeadline
                    font.weight: Font.DemiBold
                    wrapMode: Text.WordWrap
                }
                Text {
                    visible: !!host.spec && !!host.spec.text
                    width: parent.width
                    text: host.spec && host.spec.text ? host.spec.text : ""
                    color: Theme.text2
                    font.pixelSize: Theme.fsCallout
                    wrapMode: Text.WordWrap
                }
            }

            IconButton {
                icon: "close"
                tip: qsTr("Close")
                size: Theme.controlSm
                iconSize: Theme.iconSm
                onClicked: Nudges.dismiss()
            }
        }

        Rectangle {
            visible: !!host.spec && !!host.spec.field
            width: parent.width
            height: Theme.controlMd
            radius: height / 2
            color: field.activeFocus ? Theme.fillPress : Theme.fillRaised

            TextField {
                id: field
                anchors { fill: parent; leftMargin: Theme.spaceMd; rightMargin: Theme.spaceMd }
                background: null
                color: Theme.text
                placeholderText: host.spec && host.spec.field ? host.spec.field.placeholder : ""
                placeholderTextColor: Theme.text3
                font.pixelSize: Theme.fsCallout
                selectByMouse: true
                onAccepted: Nudges.submit(text)
                Keys.onEscapePressed: Nudges.dismiss()
            }
        }

        Row {
            visible: !!host.spec && !!host.spec.actions && host.spec.actions.length > 0
            spacing: Theme.spaceSm

            Repeater {
                model: host.spec && host.spec.actions ? host.spec.actions.slice(0, 2) : []
                delegate: PillButton {
                    required property var modelData
                    implicitHeight: Theme.controlSm
                    text: modelData.label
                    primary: !!modelData.primary
                    onClicked: Nudges.choose(modelData)
                }
            }
        }
    }
}
