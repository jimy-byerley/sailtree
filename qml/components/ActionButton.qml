import QtQuick 2.0
import Sailfish.Silica 1.0

// icon button showing one of the app's own icons (qml/icons), colored like the theme's
IconButton {
    // file name of the icon, without extension
    property string name

    icon.source: Qt.resolvedUrl("../icons/" + name + ".svg")
    icon.sourceSize.width: Theme.iconSizeMedium
    icon.sourceSize.height: Theme.iconSizeMedium
}
