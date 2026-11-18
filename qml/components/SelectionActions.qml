import QtQuick 2.0
import Sailfish.Silica 1.0

// content of the top bar while items are selected
Item {
    id: root

    // number of selected items
    property int count: 0
    // number of selected files and folders, the ones that can be copied, moved or deleted
    property int itemCount: 0
    // "device", "folder" or "file" when exactly one item is selected: some actions only apply to one
    property string kind: ""
    // whether the drop-down of the add button is open
    property bool addOpen: false

    signal clearRequested()
    signal renameRequested()
    signal permissionsRequested()
    signal copyRequested()
    signal moveRequested()
    signal deleteRequested()
    signal addRequested()
    signal enterRequested()

    readonly property bool singleItem: kind === "folder" || kind === "file"
    // everything shares the width, so the bar also fits narrow screens
    readonly property real slot: Math.min(Theme.itemSizeMedium, width / 9)

    IconButton {
        id: clear
        anchors { left: parent.left; verticalCenter: parent.verticalCenter }
        width: root.slot
        icon.source: "image://theme/icon-m-clear"
        onClicked: root.clearRequested()
    }

    Label {
        anchors {
            left: clear.right
            right: actions.left
            verticalCenter: parent.verticalCenter
        }
        truncationMode: TruncationMode.Fade
        color: Theme.highlightColor
        text: "" + root.count
    }

    Row {
        id: actions
        anchors { right: parent.right; verticalCenter: parent.verticalCenter }

        IconButton {
            width: root.slot
            icon.source: "image://theme/icon-m-edit"
            enabled: root.singleItem
            onClicked: root.renameRequested()
        }
        IconButton {
            width: root.slot
            icon.source: "image://theme/icon-m-keys"
            enabled: root.singleItem
            onClicked: root.permissionsRequested()
        }
        // copy and move then ask for the destination
        ActionButton {
            width: root.slot
            name: "copy"
            enabled: root.itemCount > 0
            onClicked: root.copyRequested()
        }
        ActionButton {
            width: root.slot
            name: "move"
            enabled: root.itemCount > 0
            onClicked: root.moveRequested()
        }
        IconButton {
            width: root.slot
            icon.source: "image://theme/icon-m-delete"
            enabled: root.itemCount > 0
            onClicked: root.deleteRequested()
        }
        // new folder or file where the selected item is
        ActionButton {
            width: root.slot
            name: "add"
            enabled: root.kind !== ""
            highlighted: down || root.addOpen
            onClicked: root.addRequested()
        }
        // set as current folder of its device
        ActionButton {
            width: root.slot
            name: "enter"
            enabled: root.kind === "folder"
            onClicked: root.enterRequested()
        }
    }
}
