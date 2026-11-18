#!/bin/bash
# run the QML unit tests (tests/qml) inside the Sailfish SDK build target
set -e
cd "$(dirname "$0")/.."
SFDK=${SFDK:-$HOME/jolla/sdk/bin/sfdk}
TARGET=${SFDK_TARGET:-SailfishOS-5.1.0.11-i486}
exec "$SFDK" -c target="$TARGET" build-shell env QT_QPA_PLATFORM=minimal XDG_RUNTIME_DIR=/tmp qmltestrunner -input tests/qml "$@"
