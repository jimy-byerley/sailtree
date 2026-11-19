# Sailtree

A local and remote file explorer for Sailfish OS, written in Rust with a QML (Silica) interface.

Devices are listed in one tree: the phone itself, and remote devices reached through SFTP (ssh) or FTP. Each device expands into its folders and files.

- browse, open with the system's default application, rename, delete, create, change permissions
- copy and move inside a device or between devices, with progress, cancellation and a dialog for name conflicts
- selection across folders and devices for grouped actions
- details (permissions, owner, date, size), hidden files, sort by name, date or size
- remote devices found on the local network (mDNS and a scan of the ssh and FTP ports), or added by hand

Design documents are in `docs/`:

| File | Content |
|---|---|
| `docs/specifications.md` | what the app must do |
| `docs/implementation.md` | design choices and where things are in the code |
| `docs/journal.md` | dated issues, workarounds and decision changes |

## Quickstart

### Requirements

- the [Sailfish SDK](https://docs.sailfishos.org/Tools/Sailfish_SDK/) (developed with 3.13.5), with a build target for Sailfish OS 5.1
- `sfdk` reachable, either in the `PATH` or at `~/jolla/sdk/bin/sfdk` (what the scripts assume; override with the `SFDK` or `SDK` variables)

Rust is not needed on the development machine: the build uses the Rust toolchain of the SDK target (1.75).

### Build

From the project folder, the first time:

```sh
sfdk -c target=SailfishOS-5.1.0.11-i486 build-requires pull   # installs rust, cargo, Qt and openssl headers in the target
```

then:

```sh
sfdk -c target=SailfishOS-5.1.0.11-i486 build
```

The package lands in `RPMS/sailtree-<version>.i486.rpm`. A release build from scratch takes a few minutes.

For a phone, use the target of its architecture, for instance `SailfishOS-5.1.0.11-aarch64` for a Jolla C2. The `aarch64` package builds, but only the `i486` one (the SDK emulator) has been run and tested so far; `armv7hl` has not been tried.

### Install

In the SDK emulator, build, install, start the app and save a screenshot in one command:

```sh
scripts/emulator-run.sh                      # screenshot in target/screenshot.png
scripts/emulator-run.sh --no-build shot.png  # reuse the last package
```

On a device, copy the rpm matching its architecture and install it as root:

```sh
devel-su rpm -U --force sailtree-*.rpm
```

The app runs without the Sailjail sandbox (`Sandboxing=Disabled`), as it needs the whole filesystem.

## Development

### Setup

1. Install the Sailfish SDK with its build engine, the `i486` target and the emulator.
2. Pull the build requirements as shown in the quickstart.
3. Start the emulator from the SDK (`sfdk emulator start` or the Qt Creator of the SDK).

Things to know about the SDK, detailed in `docs/journal.md`:

- VirtualBox needs hardware virtualization; on recent kernels the `kvm` modules may have to be unloaded first (`sudo modprobe -r kvm_amd kvm`).
- The build engine's disk is small by default (8 GB). Builds put their output on that disk, in `/home/mersdk/cargo-target/sailtree`, because the shared project folder cannot hold it.
- Cargo runs one job at a time with a few `SB2_RUST_*` settings: without them it can freeze under the SDK's sandbox. A frozen cargo keeps the build locked and has to be killed inside the build engine.
- Crate versions are limited to those building with Rust 1.75. After changing `Cargo.toml`, update `Cargo.lock` on the development machine with:

  ```sh
  CARGO_RESOLVER_INCOMPATIBLE_RUST_VERSIONS=fallback cargo metadata --format-version 1 > /dev/null
  ```

### Code layout

```
src/            Rust: startup, tokio runtime, configuration
  fs/           filesystem backends: local, sftp, ftp, behind one `Fs` enum
  devices/      devices, their state and how they are remembered
  discovery/    devices found on the local network
  transfer/     copy and move jobs, name conflicts
  model/        the list model shown by QML, and its Qt-free tree logic
qml/            interface: pages, components, icons, logic.js
tests/qml/      QML unit tests
rpm/            package spec
scripts/        development helpers
```

Logic is kept out of the Qt types where possible, so it can be unit tested without a screen. `docs/implementation.md` has the guideline for where new files go.

### Launcher icon

`icons/sailtree.svg` is the reference drawing, on one of the base shapes of the [Sailfish app icon guide](https://sailfishos.org/design/icons/). The png files of each size are drawn from the same description by `python3 scripts/make-icons.py` (needs Pillow); change both together. The same script draws the picture shown behind the cover, `qml/icons/cover.png`.

### Tests

Unit tests run inside the SDK target, as the development machine has no Qt or Silica:

```sh
scripts/target-cargo.sh test --locked   # Rust unit tests
scripts/target-qmltest.sh               # QML unit tests (QtTest)
```

`scripts/target-cargo.sh` passes its arguments to cargo in the target, so `scripts/target-cargo.sh build` or `clippy` work the same way. The QML tests print "Unsupported image format" warnings for the app's svg icons: the build target has no svg plugin, which is harmless.

Rust tests must not start other programs: under the SDK's sandbox that can freeze the run.

### Trying it in the emulator

`scripts/emulator-run.sh` only ever talks to the emulator (ssh on `127.0.0.1:2223`), never to a real device. The app's log is in `~/sailtree.log` of the emulator's user, and in the system journal.

Opening files needs viewers, which the emulator image does not have:

```sh
# as root in the emulator
pkcon install jolla-gallery                                              # images
ssu ar jolla-apps https://releases.jolla.com/jolla-apps/5.1.0.11/i486/   # Jolla apps repository
pkcon refresh && pkcon install sailfish-office                           # text and documents
```

Remote devices need servers to talk to. The emulator's own ssh server only accepts keys, so tests used:

- **SFTP**: a second ssh server in the emulator, with a test user allowed to log in with a password:

  ```sh
  # as root in the emulator
  useradd -m sailtest && echo "sailtest:sailpw" | chpasswd
  printf 'Port 2200\nPasswordAuthentication yes\nUsePAM no\nStrictModes no\nSubsystem sftp internal-sftp\nPidFile /run/sshd-test.pid\n' > /tmp/sshd_test_config
  /usr/sbin/sshd -D -e -f /tmp/sshd_test_config &
  ```

  then add a device in the app: SFTP, host `127.0.0.1`, port `2200`, user `sailtest`.

- **FTP**: a throwaway server on the development machine, reachable from the emulator as `10.0.2.2`:

  ```sh
  python3 -m venv ftpenv && ftpenv/bin/pip install pyftpdlib
  ftpenv/bin/python -m pyftpdlib -i 127.0.0.1 -p 2121 -u ftptest -P ftppw -w -d /some/folder
  ```

  then add a device in the app: FTP, host `10.0.2.2`, port `2121`, user `ftptest`.

Both are for tests only: weak passwords, and the FTP server gives write access to the folder it serves.

## Status

Working: the local device, SFTP and FTP devices, discovery, transfers between devices.

Not done yet: key based ssh login, FTP over TLS, resuming interrupted transfers, tests on a real phone, packaging for the Chum repository, and the choice of a license (the rpm spec has a placeholder).
