import QtQuick 2.0
import QtTest 1.0
import "../../qml/components"

// the action buttons, driven without a screen
TestCase {
    name: "actions"

    Component {
        id: actionButton
        ActionButton { }
    }

    Component {
        id: selectionActions
        SelectionActions { width: 720; height: 100 }
    }

    function test_actionButtonUsesTheAppIcons() {
        var button = actionButton.createObject(null, { name: "copy" })
        verify(button !== null)
        var source = "" + button.icon.source
        verify(source.indexOf("/qml/icons/copy.svg") === source.length - "/qml/icons/copy.svg".length, source)
        button.destroy()
    }

    function test_selectionActionsShareTheWidth() {
        var bar = selectionActions.createObject(null, { count: 2, itemCount: 1, kind: "" })
        verify(bar !== null)
        // clear, count and seven actions
        verify(bar.slot * 9 <= bar.width)
        verify(!bar.singleItem)
        bar.kind = "folder"
        verify(bar.singleItem)
        bar.kind = "device"
        verify(!bar.singleItem)
        bar.destroy()
    }
}
