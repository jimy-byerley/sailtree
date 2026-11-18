import QtQuick 2.0
import Sailfish.Silica 1.0

// content of the top bar while items are selected
Item {
    id: root

    // number of selected items
    property int count: 0
    // whether exactly one item is selected: some actions only apply to one
    property bool single: false

    signal clearRequested()
    signal renameRequested()
    signal permissionsRequested()
    signal deleteRequested()

    IconButton {
        id: clear
        anchors { left: parent.left; leftMargin: Theme.paddingMedium; verticalCenter: parent.verticalCenter }
        icon.source: "image://theme/icon-m-clear"
        onClicked: root.clearRequested()
    }

    Label {
        anchors {
            left: clear.right
            leftMargin: Theme.paddingSmall
            right: actions.left
            verticalCenter: parent.verticalCenter
        }
        truncationMode: TruncationMode.Fade
        color: Theme.highlightColor
        text: qsTr("%n selected", "", root.count)
    }

    Row {
        id: actions
        anchors { right: parent.right; rightMargin: Theme.paddingMedium; verticalCenter: parent.verticalCenter }

        IconButton {
            icon.source: "image://theme/icon-m-edit"
            enabled: root.single
            onClicked: root.renameRequested()
        }
        IconButton {
            icon.source: "image://theme/icon-m-keys"
            enabled: root.single
            onClicked: root.permissionsRequested()
        }
        IconButton {
            icon.source: "image://theme/icon-m-delete"
            onClicked: root.deleteRequested()
        }
    }
}
