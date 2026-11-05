import QtQuick
import Snellar

Rectangle {
    id: gate
    readonly property var problem: Api.libraryProblem
    visible: !!problem && problem.saving
    color: Theme.ink

    MouseArea { anchors.fill: parent; hoverEnabled: true; preventStealing: true; onWheel: w => w.accepted = true }

    Column {
        anchors.centerIn: parent
        width: Math.min(parent.width - 2 * Theme.spaceXl, Theme.toastText)
        spacing: Theme.spaceLg

        Icon { anchors.horizontalCenter: parent.horizontalCenter; name: "alert"; size: Theme.iconXl; color: Theme.red }
        Text {
            width: parent.width
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.WordWrap
            text: gate.problem ? Errors.text(gate.problem.code) : ""
            color: Theme.text
            font.family: Theme.font
            font.pixelSize: Theme.fsTitle1
            font.weight: Font.Bold
        }
        Text {
            width: parent.width
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.WordWrap
            text: gate.problem ? Errors.libraryHelp(gate.problem.code) : ""
            color: Theme.text2
            font.pixelSize: Theme.fsBody
        }
        TextEdit {
            width: parent.width
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.WrapAnywhere
            readOnly: true
            selectByMouse: true
            text: gate.problem ? gate.problem.path : ""
            color: Theme.text2
            font.pixelSize: Theme.fsCaption
        }
        Row {
            anchors.horizontalCenter: parent.horizontalCenter
            spacing: Theme.spaceMd
            PillButton { text: qsTr("Try again"); icon: "refresh"; primary: true; onClicked: Api.call("libraryRetry") }
            PillButton { text: qsTr("Open folder"); onClicked: Api.call("libraryFolder") }
        }
        Column {
            width: parent.width
            spacing: Theme.spaceSm
            Link {
                anchors.horizontalCenter: parent.horizontalCenter
                text: qsTr("Continue without saving")
                color: Theme.text
                font.pixelSize: Theme.fsBody
                font.weight: Font.DemiBold
                onClicked: Api.call("libraryContinue")
            }
            Text {
                width: parent.width
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.WordWrap
                text: qsTr("Nothing you do in this session will be kept, and your library is not shown.")
                color: Theme.text3
                font.pixelSize: Theme.fsCaption
            }
        }
    }
}
