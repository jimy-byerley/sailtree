import QtQuick 2.0
import Sailfish.Silica 1.0
import "../js/logic.js" as Logic

// one row of the tree: a device, a folder, a file, or a placeholder under a folder
ListItem {
    id: root

    // the TreeModel the row comes from
    property var tree
    property bool showDetails: false
    // deeper rows are not indented further
    readonly property int maximumIndent: 6

    readonly property bool isPlaceholder: model.kind === "loading" || model.kind === "error"
    readonly property bool hasDetails: showDetails && model.selectable

    // asked from the context menu: create something in this folder
    signal newFolderRequested()
    signal newFileRequested()

    contentHeight: hasDetails ? Theme.itemSizeMedium : Theme.itemSizeSmall
    highlighted: down || menuOpen || model.selected

    onClicked: {
        if (model.hasChildren)
            tree.toggle(model.index)
        else if (model.kind === "file")
            Qt.openUrlExternally(Logic.fileUrl(model.path))
    }

    Item {
        id: content
        x: Theme.horizontalPageMargin + Logic.indent(model.depth, root.maximumIndent) * Theme.paddingLarge
        width: parent.width - x - (circle.visible ? circle.width : Theme.horizontalPageMargin)
        height: root.contentHeight

        Icon {
            id: arrow
            anchors.verticalCenter: parent.verticalCenter
            width: Theme.iconSizeSmall
            height: width
            sourceSize.width: width
            source: "image://theme/icon-m-right"
            rotation: model.expanded ? 90 : 0
            opacity: model.hasChildren ? 1 : 0
            highlighted: root.highlighted
            Behavior on rotation { NumberAnimation { duration: 100 } }
        }

        Icon {
            id: icon
            anchors { left: arrow.right; leftMargin: Theme.paddingSmall; verticalCenter: parent.verticalCenter }
            width: Theme.iconSizeSmallPlus
            height: width
            sourceSize.width: width
            visible: model.kind !== "loading"
            highlighted: root.highlighted
            source: {
                switch (model.kind) {
                case "device": return "image://theme/icon-m-phone"
                case "folder": return "image://theme/icon-m-file-folder"
                case "error": return "image://theme/icon-m-warning"
                default: return "image://theme/icon-m-file-other-" + (Theme.colorScheme === Theme.LightOnDark ? "dark" : "light")
                }
            }
        }

        BusyIndicator {
            anchors.centerIn: icon
            size: BusyIndicatorSize.ExtraSmall
            running: model.kind === "loading"
        }

        Column {
            anchors {
                left: icon.right
                leftMargin: Theme.paddingMedium
                right: parent.right
                verticalCenter: parent.verticalCenter
            }

            Label {
                width: parent.width
                truncationMode: TruncationMode.Fade
                text: model.kind === "loading" ? qsTr("Loading…") : model.name
                font.pixelSize: root.isPlaceholder ? Theme.fontSizeSmall : Theme.fontSizeMedium
                color: root.isPlaceholder ? Theme.secondaryColor
                                          : (root.highlighted ? Theme.highlightColor : Theme.primaryColor)
            }

            Label {
                width: parent.width
                visible: root.hasDetails
                truncationMode: TruncationMode.Fade
                font.pixelSize: Theme.fontSizeExtraSmall
                color: root.highlighted ? Theme.secondaryHighlightColor : Theme.secondaryColor
                text: root.hasDetails
                      ? Logic.details(model.permissions, model.owner, model.group, model.size,
                                      model.date, model.kind === "folder", model.link)
                      : ""
            }
        }
    }

    SelectionCircle {
        id: circle
        anchors.right: parent.right
        height: root.contentHeight
        visible: model.selectable
        selected: model.selected
        onClicked: tree.toggleSelected(model.index)
    }

    menu: model.hasChildren ? folderMenu : null

    Component {
        id: folderMenu
        ContextMenu {
            MenuItem {
                text: qsTr("New folder")
                onClicked: root.newFolderRequested()
            }
            MenuItem {
                text: qsTr("New file")
                onClicked: root.newFileRequested()
            }
            MenuItem {
                text: qsTr("Refresh")
                visible: model.expanded
                onClicked: tree.refresh(model.index)
            }
        }
    }
}
