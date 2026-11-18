import QtQuick 2.0
import Sailfish.Silica 1.0

// asks whether to trust the key a device presents: accepting trusts it
Dialog {
    id: dialog

    // name of the device
    property string device
    property string fingerprint
    // the device presented another key before
    property bool changed: false

    Column {
        width: parent.width
        spacing: Theme.paddingLarge

        DialogHeader {
            acceptText: qsTr("Trust")
            title: dialog.device
        }
        Label {
            x: Theme.horizontalPageMargin
            width: parent.width - 2 * x
            wrapMode: Text.Wrap
            color: Theme.highlightColor
            text: dialog.changed
                  ? qsTr("The key of this device is not the one trusted before. It may have been reinstalled, or another machine may be answering in its place.")
                  : qsTr("First connection to this device. Check that this is the key it shows, to be sure of who answers.")
        }
        Label {
            x: Theme.horizontalPageMargin
            width: parent.width - 2 * x
            wrapMode: Text.WrapAnywhere
            font.family: Theme.fontFamilyHeading
            text: dialog.fingerprint
        }
    }
}
