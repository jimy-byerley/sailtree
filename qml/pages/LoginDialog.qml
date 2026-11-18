import QtQuick 2.0
import Sailfish.Silica 1.0

// user and password asked when connecting to a device
Dialog {
    id: dialog

    // name of the device
    property string device
    property alias user: userField.text
    property alias password: passwordField.text
    property alias savePassword: saveSwitch.checked

    canAccept: userField.text.length > 0

    Column {
        width: parent.width

        DialogHeader {
            acceptText: qsTr("Connect")
            title: dialog.device
        }
        TextField {
            id: userField
            width: parent.width
            label: qsTr("User")
            placeholderText: label
            inputMethodHints: Qt.ImhNoAutoUppercase | Qt.ImhNoPredictiveText
            EnterKey.iconSource: "image://theme/icon-m-enter-next"
            EnterKey.onClicked: passwordField.focus = true
        }
        PasswordField {
            id: passwordField
            width: parent.width
            label: qsTr("Password")
            placeholderText: label
            EnterKey.iconSource: "image://theme/icon-m-enter-accept"
            EnterKey.onClicked: dialog.accept()
        }
        TextSwitch {
            id: saveSwitch
            text: qsTr("Remember the password")
            description: qsTr("To connect without being asked again")
        }
    }
}
