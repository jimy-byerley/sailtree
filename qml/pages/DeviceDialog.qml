import QtQuick 2.0
import Sailfish.Silica 1.0
import "../js/logic.js" as Logic

// connection settings of a remote device, to add one or change an existing one
Dialog {
    id: dialog

    // a new device: there is nothing to disconnect or forget, and no current folder yet
    property bool adding: true
    property bool connected: false
    // "sftp" or "ftp"
    property string protocol: "sftp"
    property alias name: nameField.text
    property alias host: hostField.text
    property string initialPort: "22"
    readonly property int port: Logic.validPort(portField.text) ? parseInt(portField.text, 10) : 0
    property alias user: userField.text
    property alias password: passwordField.text
    property alias savePassword: saveSwitch.checked
    property string fingerprint
    property alias folder: folderField.text

    // asked with the buttons at the bottom; the dialog is then left without accepting
    signal disconnectRequested()
    signal forgetRequested()

    canAccept: Logic.validHost(hostField.text) && Logic.validPort(portField.text)

    // the port follows the protocol, unless another one was typed
    function choose(chosen) {
        if (portField.text === Logic.defaultPort(protocol))
            portField.text = Logic.defaultPort(chosen)
        protocol = chosen
    }

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height + Theme.paddingLarge

        Column {
            id: column
            width: parent.width

            DialogHeader {
                acceptText: dialog.adding ? qsTr("Add") : qsTr("Save")
            }
            ComboBox {
                id: protocolBox
                label: qsTr("Protocol")
                description: dialog.protocol === "ftp" ? qsTr("FTP sends the password and the files without encryption") : ""
                currentIndex: dialog.protocol === "ftp" ? 1 : 0
                menu: ContextMenu {
                    MenuItem { text: "SFTP (ssh)"; onClicked: dialog.choose("sftp") }
                    MenuItem { text: "FTP"; onClicked: dialog.choose("ftp") }
                }
            }
            TextField {
                id: hostField
                width: parent.width
                label: qsTr("Host name or address")
                placeholderText: label
                inputMethodHints: Qt.ImhNoAutoUppercase | Qt.ImhNoPredictiveText | Qt.ImhUrlCharactersOnly
                EnterKey.iconSource: "image://theme/icon-m-enter-next"
                EnterKey.onClicked: portField.focus = true
            }
            TextField {
                id: portField
                width: parent.width
                label: qsTr("Port")
                placeholderText: label
                text: dialog.initialPort
                inputMethodHints: Qt.ImhDigitsOnly
                EnterKey.iconSource: "image://theme/icon-m-enter-next"
                EnterKey.onClicked: userField.focus = true
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
                placeholderText: qsTr("Password (asked when connecting if empty)")
                EnterKey.iconSource: "image://theme/icon-m-enter-next"
                EnterKey.onClicked: nameField.focus = true
            }
            TextSwitch {
                id: saveSwitch
                text: qsTr("Remember the password")
                description: qsTr("Kept in the configuration of the app, readable by this user only")
            }
            TextField {
                id: nameField
                width: parent.width
                label: qsTr("Name shown in the list")
                placeholderText: qsTr("Name (the host by default)")
                EnterKey.iconSource: "image://theme/icon-m-enter-close"
                EnterKey.onClicked: focus = false
            }
            TextField {
                id: folderField
                width: parent.width
                visible: !dialog.adding
                label: qsTr("Current folder")
                placeholderText: qsTr("Current folder (home folder by default)")
                inputMethodHints: Qt.ImhNoAutoUppercase | Qt.ImhNoPredictiveText
                EnterKey.iconSource: "image://theme/icon-m-enter-close"
                EnterKey.onClicked: focus = false
            }
            DetailItem {
                visible: dialog.fingerprint.length > 0
                label: qsTr("Key")
                value: dialog.fingerprint
            }
            Item {
                width: 1
                height: Theme.paddingLarge
            }
            Button {
                anchors.horizontalCenter: parent.horizontalCenter
                visible: dialog.connected
                text: qsTr("Disconnect")
                onClicked: { dialog.disconnectRequested(); pageStack.pop() }
            }
            Item {
                width: 1
                height: Theme.paddingLarge
            }
            Button {
                anchors.horizontalCenter: parent.horizontalCenter
                visible: !dialog.adding
                text: qsTr("Forget this device")
                onClicked: { dialog.forgetRequested(); pageStack.pop() }
            }
        }
    }
}
