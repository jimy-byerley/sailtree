import QtQuick 2.0
import Sailfish.Silica 1.0

// small circle on the right of a row, filled when the row is selected
MouseArea {
    id: root
    property bool selected: false

    width: Theme.itemSizeSmall
    height: parent.height

    Rectangle {
        anchors.centerIn: parent
        width: Theme.iconSizeSmall
        height: width
        radius: width / 2
        color: "transparent"
        border.width: Math.max(2, Math.round(Theme.pixelRatio * 2))
        border.color: root.selected ? Theme.highlightColor : Theme.secondaryColor

        Rectangle {
            anchors.centerIn: parent
            width: parent.width / 2
            height: width
            radius: width / 2
            color: Theme.highlightColor
            visible: root.selected
        }
    }
}
