import QtQuick 2.0
import Sailfish.Silica 1.0
import "pages"
import "cover"

ApplicationWindow {
    id: app

    // the model of the main page, also shown by the cover
    property var tree

    initialPage: Component { MainPage { Component.onCompleted: app.tree = model } }
    cover: Component { CoverPage { tree: app.tree } }
    allowedOrientations: defaultAllowedOrientations
}
