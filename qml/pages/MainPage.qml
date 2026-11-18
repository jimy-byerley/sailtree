import QtQuick 2.0
import Sailfish.Silica 1.0
import Sailtree 1.0
import "../components"
import "../js/logic.js" as Logic

Page {
    id: page
    allowedOrientations: Orientation.All

    // second line with permissions, owner, size and date under each item
    property bool showDetails: false
    // which drop-down of the top bar is open: "", "menu" or "add"
    property string openPanel: ""

    TreeModel {
        id: treeModel
        onFailed: Notices.show(message, Notice.Long, Notice.Bottom)
        onRevealed: list.positionViewAtIndex(row, top ? ListView.Beginning : ListView.Contain)
        onSelectionChanged: if (selectionCount > 0) page.openPanel = ""
        // the local device is the starting point: show its home folder right away
        Component.onCompleted: toggle(0)
    }

    function create(row, folder) {
        page.openPanel = ""
        var dialog = pageStack.push(Qt.resolvedUrl("NameDialog.qml"), {
            title: folder ? qsTr("New folder") : qsTr("New file"),
            acceptText: qsTr("Create")
        })
        dialog.accepted.connect(function() {
            if (folder)
                treeModel.createFolder(row, dialog.name)
            else
                treeModel.createFile(row, dialog.name)
        })
    }

    function renameSelected() {
        var row = treeModel.selectedRow
        if (row < 0)
            return
        var dialog = pageStack.push(Qt.resolvedUrl("NameDialog.qml"), {
            title: qsTr("Rename"),
            acceptText: qsTr("Rename"),
            initialName: treeModel.nameAt(row)
        })
        dialog.accepted.connect(function() {
            // the row may have moved while the dialog was open
            if (treeModel.selectedRow >= 0)
                treeModel.rename(treeModel.selectedRow, dialog.name)
        })
    }

    function changePermissions() {
        var row = treeModel.selectedRow
        if (row < 0)
            return
        var dialog = pageStack.push(Qt.resolvedUrl("PermissionsDialog.qml"), {
            name: treeModel.nameAt(row),
            mode: treeModel.modeAt(row)
        })
        dialog.accepted.connect(function() {
            if (treeModel.selectedRow >= 0)
                treeModel.chmod(treeModel.selectedRow, dialog.mode)
        })
    }

    RemorsePopup {
        id: remorse
    }

    Column {
        id: topBar
        width: parent.width
        z: 1

        Item {
            width: parent.width
            height: Theme.itemSizeLarge

            // without selection: display options and creation
            Item {
                anchors.fill: parent
                visible: treeModel.selectionCount === 0

                IconButton {
                    id: menuButton
                    anchors { left: parent.left; leftMargin: Theme.paddingMedium; verticalCenter: parent.verticalCenter }
                    icon.source: "image://theme/icon-m-menu"
                    highlighted: down || page.openPanel === "menu"
                    onClicked: page.openPanel = page.openPanel === "menu" ? "" : "menu"
                }

                Label {
                    anchors {
                        left: menuButton.right
                        right: addButton.left
                        verticalCenter: parent.verticalCenter
                    }
                    horizontalAlignment: Text.AlignHCenter
                    truncationMode: TruncationMode.Fade
                    font.pixelSize: Theme.fontSizeLarge
                    color: Theme.highlightColor
                    text: "Sailtree"
                }

                IconButton {
                    id: addButton
                    anchors { right: parent.right; rightMargin: Theme.paddingMedium; verticalCenter: parent.verticalCenter }
                    icon.source: "image://theme/icon-m-add"
                    highlighted: down || page.openPanel === "add"
                    onClicked: page.openPanel = page.openPanel === "add" ? "" : "add"
                }
            }

            // with a selection: actions on it
            SelectionActions {
                anchors.fill: parent
                visible: treeModel.selectionCount > 0
                count: treeModel.selectionCount
                single: treeModel.selectedRow >= 0
                onClearRequested: treeModel.clearSelection()
                onRenameRequested: page.renameSelected()
                onPermissionsRequested: page.changePermissions()
                onDeleteRequested: remorse.execute(qsTr("Deleting %n item(s)", "", treeModel.selectionCount),
                                                   function() { treeModel.removeSelected() })
            }
        }

        // drop-down of the menu button
        Column {
            width: parent.width
            visible: page.openPanel === "menu"

            TextSwitch {
                text: qsTr("Show details")
                description: qsTr("Permissions, owner, size and date")
                checked: page.showDetails
                automaticCheck: false
                onClicked: page.showDetails = !page.showDetails
            }
            TextSwitch {
                text: qsTr("Show hidden files")
                checked: treeModel.showHidden
                automaticCheck: false
                onClicked: treeModel.showHidden = !treeModel.showHidden
            }
        }

        // drop-down of the add button
        Column {
            width: parent.width
            visible: page.openPanel === "add"

            Repeater {
                model: [
                    { label: qsTr("Device"), icon: "image://theme/icon-m-computer", action: "" },
                    { label: qsTr("Folder"), icon: "image://theme/icon-m-file-folder", action: "folder" },
                    { label: qsTr("File"), icon: "image://theme/icon-m-file-document-" + (Theme.colorScheme === Theme.LightOnDark ? "dark" : "light"), action: "file" }
                ]

                BackgroundItem {
                    width: parent.width
                    height: Theme.itemSizeSmall
                    // adding devices comes with remote devices
                    enabled: modelData.action !== ""
                    opacity: enabled ? 1 : Theme.opacityLow
                    // -1: in the home folder
                    onClicked: page.create(-1, modelData.action === "folder")

                    Icon {
                        id: entryIcon
                        anchors { left: parent.left; leftMargin: Theme.horizontalPageMargin; verticalCenter: parent.verticalCenter }
                        width: Theme.iconSizeSmallPlus
                        height: width
                        sourceSize.width: width
                        source: modelData.icon
                        highlighted: parent.highlighted
                    }
                    Label {
                        anchors { left: entryIcon.right; leftMargin: Theme.paddingMedium; verticalCenter: parent.verticalCenter }
                        text: modelData.label
                        color: parent.highlighted ? Theme.highlightColor : Theme.primaryColor
                    }
                }
            }
        }
    }

    SilicaListView {
        id: list
        anchors { top: topBar.bottom; left: parent.left; right: parent.right; bottom: parent.bottom }
        clip: true
        model: treeModel

        section.property: "category"
        section.delegate: SectionHeader {
            text: Logic.title(section)
        }

        delegate: TreeDelegate {
            width: list.width
            tree: treeModel
            showDetails: page.showDetails
            onNewFolderRequested: page.create(model.index, true)
            onNewFileRequested: page.create(model.index, false)
        }

        VerticalScrollDecorator { }
    }
}
