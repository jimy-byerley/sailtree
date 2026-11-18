import QtQuick 2.0
import Sailfish.Silica 1.0
import "../js/logic.js" as Logic

// one row of the tree: a device, a folder, a file, or a placeholder under a folder
ListItem {
    id: root

    // the TreeModel the row comes from
    property var tree
    property bool showDetails: false
    // a destination is being chosen for a copy or move: folders offer themselves instead of
    // being selectable
    property bool picking: false
    signal picked()
    // long press on a remote device
    signal settingsRequested()

    onPressAndHold: if (model.kind === "device" && model.remote) settingsRequested()
    // deeper rows are not indented further
    readonly property int maximumIndent: 6

    readonly property int row: model.index
    readonly property bool isDevice: model.kind === "device"
    // current folder of a device
    readonly property string currentPath: isDevice ? model.path : ""
    readonly property bool isPlaceholder: model.kind === "loading" || model.kind === "error"
    readonly property bool hasDetails: showDetails && (model.kind === "folder" || model.kind === "file")
    // height of the part with the name; a device has its current folder below it
    readonly property real lineHeight: hasDetails ? Theme.itemSizeMedium : Theme.itemSizeSmall

    contentHeight: lineHeight + (isDevice ? Theme.itemSizeExtraSmall : 0)
    highlighted: down || model.selected

    onClicked: {
        if (model.hasChildren)
            tree.toggle(model.index)
        else if (model.kind === "file")
            tree.open(model.index)
    }

    Item {
        id: content
        x: Theme.horizontalPageMargin + Logic.indent(model.depth, root.maximumIndent) * Theme.paddingLarge
        width: parent.width - x - (side.width > 0 ? side.width : Theme.horizontalPageMargin)
        height: root.lineHeight

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
            visible: model.kind !== "loading" && !model.busy
            highlighted: root.highlighted
            source: {
                switch (model.kind) {
                case "device": return model.remote ? "image://theme/icon-m-computer" : "image://theme/icon-m-phone"
                case "folder": return "image://theme/icon-m-file-folder"
                case "error": return "image://theme/icon-m-warning"
                default: return Logic.fileIcon(model.name, Theme.colorScheme === Theme.LightOnDark)
                }
            }
        }

        BusyIndicator {
            anchors.centerIn: icon
            size: BusyIndicatorSize.ExtraSmall
            running: model.kind === "loading" || model.busy
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
                      ? Logic.details(model.permissions, model.owner, model.group, model.date, model.link,
                                      model.kind === "folder" ? -1 : model.size)
                      : ""
            }
        }
    }

    Row {
        id: side
        anchors.right: parent.right
        height: root.lineHeight

        // share of the files of its folder taken by this file
        ProgressCircleBase {
            anchors.verticalCenter: parent.verticalCenter
            width: Theme.iconSizeSmall
            height: width
            visible: model.ratio >= 0
            value: Math.max(0, model.ratio)
            borderWidth: Math.max(3, Math.round(Theme.pixelRatio * 4))
            progressColor: Theme.highlightColor
            backgroundColor: Theme.rgba(Theme.primaryColor, Theme.opacityLow)
        }

        SelectionCircle {
            height: parent.height
            visible: model.selectable && !root.picking
            selected: model.selected
            onClicked: tree.toggleSelected(model.index)
        }

        // destination of the copy or move
        IconButton {
            width: Theme.itemSizeSmall
            height: parent.height
            visible: root.picking && model.hasChildren
            icon.source: "image://theme/icon-m-enter-accept"
            onClicked: root.picked()
        }
    }

    // current folder of a device: each level can be clicked to restart the tree from it
    Flickable {
        id: crumbs
        anchors {
            top: content.bottom
            left: parent.left
            leftMargin: content.x + Theme.iconSizeSmall + Theme.paddingSmall
            right: parent.right
            rightMargin: Theme.horizontalPageMargin
        }
        height: Theme.itemSizeExtraSmall
        visible: root.isDevice
        clip: true
        flickableDirection: Flickable.HorizontalFlick
        boundsBehavior: Flickable.StopAtBounds
        contentWidth: levels.width
        contentHeight: height
        // a long path shows its end, where the current folder is
        onContentWidthChanged: contentX = Math.max(0, contentWidth - width)
        onWidthChanged: contentX = Math.max(0, contentWidth - width)

        Row {
            id: levels
            height: parent.height

            Repeater {
                id: levelRepeater
                model: root.isDevice ? Logic.crumbs(root.currentPath) : []

                MouseArea {
                    width: separator.width + level.width + Theme.paddingMedium
                    height: levels.height
                    onClicked: root.tree.setCurrentFolder(root.row, modelData.path)

                    Label {
                        id: separator
                        anchors.verticalCenter: parent.verticalCenter
                        width: index > 1 ? implicitWidth + Theme.paddingMedium : 0
                        visible: index > 1
                        text: "/"
                        font.pixelSize: Theme.fontSizeSmall
                        color: Theme.secondaryColor
                    }
                    Label {
                        id: level
                        anchors { left: separator.right; verticalCenter: parent.verticalCenter }
                        text: modelData.name
                        font.pixelSize: Theme.fontSizeSmall
                        color: parent.pressed || index === levelRepeater.count - 1
                               ? Theme.highlightColor : Theme.primaryColor
                    }
                }
            }
        }
    }
}
