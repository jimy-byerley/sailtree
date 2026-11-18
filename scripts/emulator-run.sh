#!/bin/bash
# build the rpm, install it in the SDK emulator, start the app and take a screenshot
#   scripts/emulator-run.sh [--no-build] [screenshot.png]
# only ever talks to the emulator (ssh on 127.0.0.1:2223), never to a real device
set -e
cd "$(dirname "$0")/.."
SDK=${SDK:-$HOME/jolla/sdk}
TARGET=${SFDK_TARGET:-SailfishOS-5.1.0.11-i486}
EMULATOR=${SFDK_EMULATOR:-Sailfish OS Emulator 5.1.0.11}
SSH_OPTIONS="-i $SDK/vmshare/ssh/private_keys/sdk -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o BatchMode=yes"
HOST=defaultuser@127.0.0.1

if [ "$1" = "--no-build" ]; then shift; else
    "$SDK/bin/sfdk" -c target="$TARGET" build
fi
SHOT=${1:-target/screenshot.png}
"$SDK/bin/sfdk" -c target="$TARGET" -c device="$EMULATOR" deploy --sdk

# the screenshot service only answers to the privileged group
ssh -p 2223 $SSH_OPTIONS root@127.0.0.1 'id -Gn defaultuser | grep -qw privileged || usermod -a -G privileged defaultuser'
ssh -p 2223 $SSH_OPTIONS $HOST '
    export XDG_RUNTIME_DIR=/run/user/$(id -u) WAYLAND_DISPLAY=../../display/wayland-0
    export DBUS_SESSION_BUS_ADDRESS=unix:path=$XDG_RUNTIME_DIR/dbus/user_bus_socket
    pkill -x sailtree; rm -f shot.png
    setsid sailtree > sailtree.log 2>&1 < /dev/null &
    sleep 6
    sg privileged -c "dbus-send --session --print-reply --dest=org.nemomobile.lipstick /org/nemomobile/lipstick/screenshot org.nemomobile.lipstick.saveScreenshot string:$HOME/shot.png" > /dev/null
    sleep 1
    pgrep -x sailtree > /dev/null || echo "sailtree is not running anymore"
    echo "--- app output"; cat sailtree.log'
mkdir -p "$(dirname "$SHOT")"
ssh -p 2223 $SSH_OPTIONS $HOST 'cat shot.png' > "$SHOT"
echo "screenshot saved to $SHOT"
