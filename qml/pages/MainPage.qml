import QtQuick 2.0
import Sailfish.Silica 1.0
import Sailtree 1.0
import "../components"
import "../js/logic.js" as Logic

Page {
    id: page
    allowedOrientations: Orientation.All

    // which drop-down of the top bar is open: "", "menu", "sort", or "add" for the single selected item
    property string openPanel: ""
    // for the cover
    property alias model: treeModel

    TreeModel {
        id: treeModel
        onFailed: Notices.show(message, Notice.Long, Notice.Bottom)
        onConflict: {
            // a page transition in progress would refuse the dialog, leaving the transfer waiting
            pageStack.completeAnimation()
            var dialog = pageStack.push(Qt.resolvedUrl("ConflictDialog.qml"), { name: name })
            if (!dialog) {
                treeModel.resolveConflict("skip", false)
                return
            }
            dialog.accepted.connect(function() { treeModel.resolveConflict(dialog.answer, dialog.all) })
            dialog.rejected.connect(function() { treeModel.resolveConflict("skip", false) })
        }
        onLoginRequired: {
            var id = deviceIdAt(row)
            pageStack.completeAnimation()
            var dialog = pageStack.push(Qt.resolvedUrl("LoginDialog.qml"), {
                device: deviceText(row, "name"),
                user: deviceText(row, "user"),
                savePassword: deviceSavesPassword(row)
            })
            if (dialog)
                dialog.accepted.connect(function() {
                    treeModel.login(treeModel.deviceRow(id), dialog.user, dialog.password, dialog.savePassword)
                })
        }
        onHostKeyUnknown: {
            var id = deviceIdAt(row)
            pageStack.completeAnimation()
            var dialog = pageStack.push(Qt.resolvedUrl("HostKeyDialog.qml"), {
                device: deviceText(row, "name"),
                fingerprint: fingerprint,
                changed: changed
            })
            if (dialog)
                dialog.accepted.connect(function() {
                    treeModel.trustHost(treeModel.deviceRow(id), fingerprint)
                })
        }
        onOpenRequested: Qt.openUrlExternally(Logic.fileUrl(path))
        onRevealed: list.positionViewAtIndex(row, top ? ListView.Beginning : ListView.Contain)
        // each drop-down belongs to one state of the top bar
        onSelectionChanged: {
            if (page.openPanel === "add" ? selectedRow < 0 : selectionCount > 0)
                page.openPanel = ""
        }
        // the local device is the starting point: show its home folder right away
        Component.onCompleted: toggle(0)
    }

    // new folder or file where the single selected item is: in a folder, in the current folder
    // of a device, next to a file
    function create(folder) {
        page.openPanel = ""
        if (treeModel.selectedRow < 0)
            return
        var dialog = pageStack.push(Qt.resolvedUrl("NameDialog.qml"), {
            title: folder ? qsTr("New folder") : qsTr("New file"),
            acceptText: qsTr("Create")
        })
        dialog.accepted.connect(function() {
            // the row may have moved while the dialog was open
            var row = treeModel.selectedRow
            if (row < 0)
                return
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

    function addDevice() {
        var dialog = pageStack.push(Qt.resolvedUrl("DeviceDialog.qml"), { adding: true })
        dialog.accepted.connect(function() {
            var id = treeModel.addDevice(dialog.name, dialog.host, dialog.port, dialog.user, dialog.password,
                                         dialog.savePassword)
            treeModel.setProtocol(treeModel.deviceRow(id), dialog.protocol)
        })
    }

    // rows move when devices change category: a device is followed by its identifier
    function editDevice(row) {
        var id = treeModel.deviceIdAt(row)
        var dialog = pageStack.push(Qt.resolvedUrl("DeviceDialog.qml"), {
            adding: false,
            connected: treeModel.deviceConnected(row),
            protocol: treeModel.deviceText(row, "protocol"),
            name: treeModel.deviceText(row, "name"),
            host: treeModel.deviceText(row, "host"),
            initialPort: treeModel.deviceText(row, "port"),
            user: treeModel.deviceText(row, "user"),
            password: treeModel.deviceText(row, "password"),
            savePassword: treeModel.deviceSavesPassword(row),
            fingerprint: treeModel.deviceText(row, "fingerprint"),
            folder: treeModel.deviceText(row, "folder")
        })
        dialog.accepted.connect(function() {
            treeModel.setProtocol(treeModel.deviceRow(id), dialog.protocol)
            treeModel.updateDevice(treeModel.deviceRow(id), dialog.name, dialog.host, dialog.port, dialog.user,
                                   dialog.password, dialog.savePassword, dialog.folder)
        })
        dialog.disconnectRequested.connect(function() { treeModel.disconnect(treeModel.deviceRow(id)) })
        dialog.forgetRequested.connect(function() { treeModel.removeDevice(treeModel.deviceRow(id)) })
    }

    // devices answering on the network are looked for while the app is in use
    Timer {
        interval: 30000
        repeat: true
        running: Qt.application.active
        triggeredOnStart: true
        onTriggered: treeModel.probe()
    }

    Timer {
        interval: 120000
        repeat: true
        running: Qt.application.active
        triggeredOnStart: true
        onTriggered: treeModel.discover()
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

            // without selection: display options
            Item {
                anchors.fill: parent
                visible: treeModel.selectionCount === 0 && treeModel.pendingCount === 0

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
                        right: sortButton.left
                        verticalCenter: parent.verticalCenter
                    }
                    horizontalAlignment: Text.AlignHCenter
                    truncationMode: TruncationMode.Fade
                    font.pixelSize: Theme.fontSizeLarge
                    color: Theme.highlightColor
                    text: "Sailtree"
                }

                ActionButton {
                    id: sortButton
                    anchors { right: parent.right; rightMargin: Theme.paddingMedium; verticalCenter: parent.verticalCenter }
                    name: "sort"
                    highlighted: down || page.openPanel === "sort"
                    onClicked: page.openPanel = page.openPanel === "sort" ? "" : "sort"
                }
            }

            // items kept aside for a copy or move: waiting for the destination, chosen with the
            // button of a device or folder row
            Item {
                anchors.fill: parent
                visible: treeModel.pendingCount > 0

                IconButton {
                    id: cancelButton
                    anchors { left: parent.left; leftMargin: Theme.paddingMedium; verticalCenter: parent.verticalCenter }
                    icon.source: "image://theme/icon-m-clear"
                    onClicked: treeModel.cancelTransfer()
                }
                Label {
                    anchors {
                        left: cancelButton.right
                        leftMargin: Theme.paddingMedium
                        right: parent.right
                        rightMargin: Theme.horizontalPageMargin
                        verticalCenter: parent.verticalCenter
                    }
                    truncationMode: TruncationMode.Fade
                    color: Theme.highlightColor
                    text: treeModel.pendingMove ? qsTr("Move %n item(s) to…", "", treeModel.pendingCount)
                                                : qsTr("Copy %n item(s) to…", "", treeModel.pendingCount)
                }
            }

            // with a selection: actions on it
            SelectionActions {
                anchors.fill: parent
                visible: treeModel.selectionCount > 0 && treeModel.pendingCount === 0
                onCopyRequested: treeModel.startTransfer(false)
                onMoveRequested: treeModel.startTransfer(true)
                count: treeModel.selectionCount
                itemCount: treeModel.selectedItemCount
                kind: treeModel.selectedKind
                addOpen: page.openPanel === "add"
                onClearRequested: treeModel.clearSelection()
                onEnterRequested: treeModel.enter(treeModel.selectedRow)
                onAddRequested: page.openPanel = page.openPanel === "add" ? "" : "add"
                onRenameRequested: page.renameSelected()
                onPermissionsRequested: page.changePermissions()
                onDeleteRequested: remorse.execute(qsTr("Deleting %n item(s)", "", treeModel.selectedItemCount),
                                                   function() { treeModel.removeSelected() })
            }
        }

        // a copy or move is running
        Item {
            width: parent.width
            height: progress.height
            visible: treeModel.busy

            ProgressBar {
                id: progress
                anchors { left: parent.left; right: stopButton.left }
                minimumValue: 0
                maximumValue: 1
                value: treeModel.transferProgress
                label: Logic.transferLabel(treeModel.transferMoving, treeModel.transferName, treeModel.transferWaiting)
            }
            IconButton {
                id: stopButton
                anchors { right: parent.right; rightMargin: Theme.paddingMedium; verticalCenter: parent.verticalCenter }
                icon.source: "image://theme/icon-m-clear"
                onClicked: treeModel.cancelRunning()
            }
        }

        // drop-down of the menu button
        Column {
            width: parent.width
            visible: page.openPanel === "menu"

            TextSwitch {
                text: qsTr("Show details")
                description: qsTr("Permissions, owner, size and date")
                checked: treeModel.showDetails
                automaticCheck: false
                onClicked: treeModel.showDetails = !treeModel.showDetails
            }
            TextSwitch {
                text: qsTr("Show hidden files")
                checked: treeModel.showHidden
                automaticCheck: false
                onClicked: treeModel.showHidden = !treeModel.showHidden
            }
        }

        // drop-down of the sort button
        Column {
            width: parent.width
            visible: page.openPanel === "sort"

            Repeater {
                model: [
                    { label: qsTr("Sort by name"), key: "name" },
                    { label: qsTr("Sort by date, newest first"), key: "date" },
                    { label: qsTr("Sort by size, biggest first"), key: "size" }
                ]

                BackgroundItem {
                    id: sortEntry
                    width: parent.width
                    height: Theme.itemSizeSmall
                    readonly property bool current: treeModel.sort === modelData.key
                    onClicked: {
                        treeModel.sort = modelData.key
                        page.openPanel = ""
                    }

                    Label {
                        anchors { left: parent.left; leftMargin: Theme.horizontalPageMargin; verticalCenter: parent.verticalCenter }
                        text: modelData.label
                        color: sortEntry.highlighted || sortEntry.current ? Theme.highlightColor : Theme.primaryColor
                    }
                }
            }
        }

        // drop-down of the add button of the selection actions
        Column {
            width: parent.width
            visible: page.openPanel === "add"

            Repeater {
                model: [
                    { label: qsTr("Folder"), icon: "image://theme/icon-m-file-folder", action: "folder" },
                    { label: qsTr("File"), icon: "image://theme/icon-m-file-document-" + (Theme.colorScheme === Theme.LightOnDark ? "dark" : "light"), action: "file" }
                ]

                BackgroundItem {
                    id: addEntry
                    width: parent.width
                    height: Theme.itemSizeSmall
                    onClicked: page.create(modelData.action === "folder")

                    Icon {
                        id: entryIcon
                        anchors { left: parent.left; leftMargin: Theme.horizontalPageMargin; verticalCenter: parent.verticalCenter }
                        width: Theme.iconSizeSmallPlus
                        height: width
                        sourceSize.width: width
                        source: modelData.icon
                        highlighted: addEntry.highlighted
                    }
                    Label {
                        anchors { left: entryIcon.right; leftMargin: Theme.paddingMedium; verticalCenter: parent.verticalCenter }
                        text: modelData.label
                        color: addEntry.highlighted ? Theme.highlightColor : Theme.primaryColor
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

        PullDownMenu {
            MenuItem {
                text: qsTr("Add device")
                onClicked: page.addDevice()
            }
        }

        section.property: "category"
        section.delegate: SectionHeader {
            text: Logic.title(section)
        }

        delegate: TreeDelegate {
            width: list.width
            tree: treeModel
            showDetails: treeModel.showDetails
            picking: treeModel.pendingCount > 0
            onPicked: treeModel.transferTo(model.index)
            onSettingsRequested: page.editDevice(model.index)
        }

        VerticalScrollDecorator { }
    }
}
