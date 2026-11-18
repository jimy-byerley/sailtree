import QtQuick 2.0
import QtTest 1.0
import "../../qml/pages"

// the dialogs, driven without a screen
TestCase {
    name: "dialogs"

    Component {
        id: nameDialog
        NameDialog { }
    }

    Component {
        id: conflictDialog
        ConflictDialog { }
    }

    function test_conflictDialogSkipsByDefault() {
        var dialog = conflictDialog.createObject(null, { name: "notes.txt" })
        compare(dialog.answer, "skip")
        compare(dialog.all, false)
        verify(!dialog.canAccept)
        dialog.all = true
        compare(dialog.all, true)
        dialog.destroy()
    }

    Component {
        id: deviceDialog
        DeviceDialog { }
    }

    Component {
        id: loginDialog
        LoginDialog { }
    }

    function test_deviceDialogNeedsAHostAndAPort() {
        var dialog = deviceDialog.createObject(null, {})
        verify(dialog.adding)
        verify(!dialog.canAccept)
        compare(dialog.port, 22)
        dialog.host = "nas.local"
        verify(dialog.canAccept)
        dialog.host = "my nas"
        verify(!dialog.canAccept)
        dialog.destroy()

        dialog = deviceDialog.createObject(null, { adding: false, host: "10.0.0.2", initialPort: "2222", folder: "/srv" })
        verify(dialog.canAccept)
        compare(dialog.port, 2222)
        compare(dialog.folder, "/srv")
        dialog.destroy()

        // changing the protocol changes the port, unless another one was typed
        dialog = deviceDialog.createObject(null, { host: "10.0.0.2" })
        compare(dialog.protocol, "sftp")
        dialog.choose("ftp")
        compare(dialog.protocol, "ftp")
        compare(dialog.port, 21)
        dialog.choose("sftp")
        compare(dialog.port, 22)
        dialog.destroy()
        dialog = deviceDialog.createObject(null, { host: "10.0.0.2", initialPort: "2121" })
        dialog.choose("ftp")
        compare(dialog.port, 2121)
        dialog.destroy()

        dialog = deviceDialog.createObject(null, { host: "10.0.0.2", initialPort: "port" })
        verify(!dialog.canAccept)
        compare(dialog.port, 0)
        dialog.destroy()
    }

    function test_loginDialogNeedsAUser() {
        var dialog = loginDialog.createObject(null, { device: "NAS" })
        verify(!dialog.canAccept)
        dialog.user = "me"
        verify(dialog.canAccept)
        compare(dialog.savePassword, false)
        dialog.destroy()
    }

    Component {
        id: permissionsDialog
        PermissionsDialog { }
    }

    function test_nameDialogValidation() {
        var dialog = nameDialog.createObject(null, { initialName: "notes.txt" })
        compare(dialog.name, "notes.txt")
        verify(dialog.canAccept)
        dialog.destroy()

        dialog = nameDialog.createObject(null, { initialName: "" })
        verify(!dialog.canAccept, "empty name")
        dialog.destroy()

        dialog = nameDialog.createObject(null, { initialName: "a/b" })
        verify(!dialog.canAccept, "name with a slash")
        dialog.destroy()
    }

    function test_permissionsDialogKeepsItsMode() {
        var dialog = permissionsDialog.createObject(null, { name: "file", mode: 420 })
        compare(dialog.mode, 420)
        dialog.destroy()
    }
}
