#!/bin/bash
# run cargo inside the Sailfish SDK build target, eg.
#   scripts/target-cargo.sh test
#   SFDK_TARGET=SailfishOS-5.1.0.11-aarch64 scripts/target-cargo.sh build
set -e
cd "$(dirname "$0")/.."
SFDK=${SFDK:-$HOME/jolla/sdk/bin/sfdk}
TARGET=${SFDK_TARGET:-SailfishOS-5.1.0.11-i486}
case "$TARGET" in
    *-i486) TRIPLE=i686-unknown-linux-gnu ;;
    *-aarch64) TRIPLE=aarch64-unknown-linux-gnu ;;
    *-armv7hl) TRIPLE=armv7-unknown-linux-gnueabihf ;;
    *) echo "unknown architecture in target $TARGET" >&2; exit 1 ;;
esac
# - the registry cache stays in the project, the build products go to the engine's own disk
#   because rustc needs mmap, which the engine's shared folders do not support
# - the SB2_RUST_* settings and the single job avoid cargo hanging between fork and exec under scratchbox
# - QMAKE is given explicitly because probing for a missing qmake6 fails with a bogus error under scratchbox
exec "$SFDK" -c target="$TARGET" build-shell bash -c '
    export SB2_RUST_TARGET_TRIPLE='$TRIPLE' CARGO_BUILD_TARGET='$TRIPLE'
    export CARGO_HOME=$PWD/target/cargo-home
    export CARGO_TARGET_DIR=$HOME/cargo-target/sailtree
    export CARGO_PROFILE_DEV_DEBUG=0
    export QMAKE=/usr/bin/qmake
    export SB2_RUST_EXECVP_SHIM="/usr/bin/env LD_PRELOAD=/usr/lib/libsb2/libsb2.so.1 /usr/bin/env"
    export SB2_RUST_USE_REAL_EXECVP=Yes SB2_RUST_USE_REAL_FN=Yes SB2_RUST_NO_SPAWNVP=Yes
    export CARGO_BUILD_JOBS=1
    exec cargo "$@"' cargo "$@"
