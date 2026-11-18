import QtQuick 2.0
import Sailfish.Silica 1.0
import "../js/logic.js" as Logic

// asks for the name of a file or folder: to rename one, or to create one
Dialog {
    id: dialog

    property string title
    property string acceptText: qsTr("Accept")
    // name proposed when the dialog opens
    property string initialName
    // the accepted name
    readonly property string name: field.text

    canAccept: Logic.validName(field.text)

    Column {
        width: parent.width

        DialogHeader {
            title: dialog.title
            acceptText: dialog.acceptText
        }

        TextField {
            id: field
            width: parent.width
            text: dialog.initialName
            label: qsTr("Name")
            placeholderText: label
            focus: true
            inputMethodHints: Qt.ImhNoAutoUppercase | Qt.ImhNoPredictiveText
            errorHighlight: text.length > 0 && !Logic.validName(text)
            EnterKey.enabled: dialog.canAccept
            EnterKey.iconSource: "image://theme/icon-m-enter-accept"
            EnterKey.onClicked: dialog.accept()
        }
    }
}
