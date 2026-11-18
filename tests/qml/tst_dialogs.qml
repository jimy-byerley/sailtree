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
