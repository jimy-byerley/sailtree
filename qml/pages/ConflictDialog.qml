import QtQuick 2.0
import Sailfish.Silica 1.0

// asks what to do with an item whose name is taken where a copy or move arrives.
// going back skips the item
Dialog {
    id: dialog

    // name of the item
    property string name
    // "overwrite", "skip" or "rename"
    property string answer: "skip"
    // same answer for the next conflicts of the transfer
    property alias all: allSwitch.checked

    function choose(choice) {
        answer = choice
        accept()
    }

    // the buttons below accept, not the header
    canAccept: false

    Column {
        width: parent.width
        spacing: Theme.paddingLarge

        PageHeader {
            title: qsTr("Already exists")
        }
        Label {
            x: Theme.horizontalPageMargin
            width: parent.width - 2 * x
            wrapMode: Text.Wrap
            color: Theme.highlightColor
            text: qsTr("The destination already has an item named %1").arg(dialog.name)
        }
        Button {
            anchors.horizontalCenter: parent.horizontalCenter
            preferredWidth: Theme.buttonWidthLarge
            text: qsTr("Keep both")
            onClicked: { dialog.canAccept = true; dialog.choose("rename") }
        }
        Button {
            anchors.horizontalCenter: parent.horizontalCenter
            preferredWidth: Theme.buttonWidthLarge
            text: qsTr("Replace")
            onClicked: { dialog.canAccept = true; dialog.choose("overwrite") }
        }
        Button {
            anchors.horizontalCenter: parent.horizontalCenter
            preferredWidth: Theme.buttonWidthLarge
            text: qsTr("Skip")
            onClicked: { dialog.canAccept = true; dialog.choose("skip") }
        }
        TextSwitch {
            id: allSwitch
            text: qsTr("Do the same for the next ones")
        }
    }
}
