import QtQuick 2.0
import Sailfish.Silica 1.0
import "../js/logic.js" as Logic

// what shows in the home screen while the app is in the background:
// the current folder of the local device and its first items
CoverBackground {
    id: cover

    // the TreeModel of the main page
    property var tree

    readonly property bool shown: status === Cover.Active || status === Cover.Activating
    // lines of the list fitting between the title and the refresh button
    readonly property int room: Math.floor((height - list.y - Theme.itemSizeSmall) / lineHeight.height)

    function refresh() {
        if (tree && shown)
            tree.refreshCover()
    }
    onShownChanged: refresh()
    onTreeChanged: refresh()

    Connections {
        target: cover.tree ? cover.tree : null
        // a new current folder, or a transfer ending while in the background
        onCoverFolderChanged: cover.refresh()
        onBusyChanged: cover.refresh()
    }

    // the picture of the launcher icon behind the text, made and shown as on the covers of the
    // system's apps: an image of the size of the cover, with its own transparency
    Image {
        anchors.fill: parent
        source: "../icons/cover.png"
    }

    Label {
        id: lineHeight
        visible: false
        font.pixelSize: Theme.fontSizeExtraSmall
        text: "X"
    }

    // full path of the folder, its last level in the accent color as in the tree. over three lines at
    // most: first levels are left out until it fits
    Label {
        id: title

        readonly property string path: cover.tree ? cover.tree.coverFolder : ""
        property int skipped: 0
        function fit() {
            if (truncated && skipped < path.split("/").length)
                skipped++
        }
        onPathChanged: skipped = 0
        onTextChanged: fit()
        onTruncatedChanged: fit()

        x: Theme.paddingMedium
        y: Theme.paddingMedium
        width: parent.width - 2 * x
        text: cover.tree ? Logic.pathMarkup(path, Theme.highlightColor, skipped) : "Sailtree"
        textFormat: Text.StyledText
        font.pixelSize: Theme.fontSizeExtraSmall
        wrapMode: Text.WrapAnywhere
        maximumLineCount: 3
        elide: Text.ElideRight
    }

    Column {
        id: list
        x: Theme.paddingMedium
        y: title.y + title.height + Theme.paddingSmall
        width: parent.width - 2 * x

        Repeater {
            id: lines
            readonly property int total: cover.tree ? cover.tree.coverCount : 0
            model: cover.tree ? Logic.coverLines(cover.tree.coverItems, total, cover.room) : []

            Label {
                // the last line tells how many items are left out
                readonly property bool rest: index === lines.count - 1 && lines.count < lines.total
                width: list.width
                text: modelData
                color: rest ? Theme.secondaryColor : Theme.primaryColor
                font.pixelSize: Theme.fontSizeExtraSmall
                truncationMode: TruncationMode.Fade
            }
        }
    }

    CoverActionList {
        CoverAction {
            iconSource: "image://theme/icon-cover-refresh"
            onTriggered: if (cover.tree) cover.tree.refreshCover()
        }
    }
}
