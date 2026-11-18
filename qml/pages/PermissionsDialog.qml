import QtQuick 2.0
import Sailfish.Silica 1.0
import "../js/logic.js" as Logic

// edits the read / write / execute permissions of a file or folder
Dialog {
    id: dialog

    // name of the edited item
    property string name
    // permission bits: set before opening, read after acceptance
    property int mode

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height

        Column {
            id: column
            width: parent.width

            DialogHeader {
                title: dialog.name
                acceptText: qsTr("Apply")
            }

            Repeater {
                model: [qsTr("Owner"), qsTr("Group"), qsTr("Others")]

                Column {
                    id: section
                    readonly property int who: index
                    width: parent.width

                    SectionHeader {
                        text: modelData
                    }

                    Repeater {
                        model: [qsTr("Read"), qsTr("Write"), qsTr("Execute")]

                        TextSwitch {
                            text: modelData
                            automaticCheck: false
                            checked: Logic.hasPermission(dialog.mode, section.who, index)
                            onClicked: dialog.mode = Logic.withPermission(dialog.mode, section.who, index, !checked)
                        }
                    }
                }
            }
        }

        VerticalScrollDecorator { }
    }
}
