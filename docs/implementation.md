# implementation choices

## tree view

- the device/folder tree is flattened into a single rust `QAbstractListModel` (qmetaobject-rs), one row per visible node
- roles: `depth`, `name`, `kind` (device/folder/file), `expanded`, `hasChildren`, `selected`, `category`
- expanding inserts the children rows right after the node, with a temporary "loading" row while the listing runs asynchronously
- collapsing removes the contiguous range of descendant rows
- QML side is a single `SilicaListView` with `ListItem` delegates, indented by `depth` (capped visually), `section.property: "category"` for the connected/online/offline headers
- a device changing category moves its whole row block (`begin_move_rows`)
- selection is stored in rust as a set of `(device, path)`, never as row indices

## discovery

- no ARP: raw sockets need `CAP_NET_RAW`, unavailable to a regular app
- `mdns-sd` crate browses `_sftp-ssh._tcp`, `_ssh._tcp`, `_ftp._tcp` continuously, so advertising hosts show up immediately with their `.local` name and port
- a periodic TCP connect sweep of ports 22 and 21 over the local /24 completes the list (short timeout ~300ms, bounded concurrency), rerun on network change
- hosts found only by the sweep get their name from the system reverse lookup (`getnameinfo` via `dns-lookup` crate), falling back to the IP
- results from both sources are merged per IP address

## transfers

- SFTP is the only transfer path for ssh devices; interrupted transfers resume by stat + seek on the partial destination file
- longer term: optional rsync acceleration when `rsync` is available on both ends, run in an exec channel of the same ssh session (`rsync --server`). no rsync daemon (`rsync://`) support planned

## concurrency and backends

- tokio runtime running in background threads; QML-invoked methods spawn onto it, results come back to the Qt thread with `qmetaobject::queued_callback`
- SFTP via `russh` + `russh-sftp`: one shared session per device, operations run as concurrent tasks (browsing stays responsive during transfers)
- FTP via `suppaftp` async: one operation at a time per control connection, transfers use a separate connection
- local filesystem calls run in `spawn_blocking`
- backends are dispatched through `enum Fs { Local, Sftp, Ftp }`, not a dyn trait
- cancellation through task abort / cancellation token, network calls wrapped in `tokio::time::timeout`
- discovery (mdns-sd receiver, port sweep with bounded concurrency, reverse lookup in `spawn_blocking`) runs on the same runtime

## repository structure

guideline for where new files go, not an exhaustive list

```
sailtree/
├── Cargo.toml                single crate, no workspace
├── build.rs                  compiles the C++ shim (cpp crate) for SailfishApp::application()
├── Cargo.lock                committed, crate versions building with the SDK's rust
├── .cargo/config.toml        cargo settings (rust-version aware resolution, later vendored sources for offline Chum/OBS builds)
├── scripts/                  development helpers (cargo in the SDK target, emulator run)
├── src/
│   ├── main.rs               startup, type registration, main QML loading
│   ├── sailfishapp.rs        rust side of the SailfishApp shim
│   ├── runtime.rs            tokio runtime, bridge to the Qt thread (queued_callback)
│   ├── config.rs             recents, manual devices, settings (~/.config/sailtree)
│   ├── model/                QML-facing models
│   │   ├── tree.rs           flattened QAbstractListModel
│   │   ├── node.rs           node data
│   │   └── selection.rs      (device, path) selection and grouped actions
│   ├── fs/                   filesystem backends
│   │   ├── mod.rs            `enum Fs` and common operations
│   │   ├── local.rs
│   │   ├── sftp.rs           russh + russh-sftp
│   │   └── ftp.rs            suppaftp
│   ├── devices/
│   │   ├── mod.rs            device identity, connected/online/offline state, merging discovery results
│   │   └── auth.rs           credentials, ssh host key verification
│   ├── discovery/
│   │   ├── mdns.rs           mdns-sd browsing
│   │   ├── scan.rs           TCP sweep on ports 22/21
│   │   └── resolve.rs        system reverse lookup
│   └── transfer/
│       ├── queue.rs          copy/move jobs across devices, progress, cancellation
│       └── conflict.rs       overwrite / skip / rename
├── qml/                      installed to /usr/share/sailtree/qml, not embedded
│   ├── sailtree.qml          ApplicationWindow
│   ├── pages/                full pages and dialogs (MainPage, AddDeviceDialog, LoginDialog, RenameDialog, PermissionsDialog)
│   ├── components/           reusable items (TreeDelegate, SelectionCircle, SelectionActions)
│   └── cover/                CoverPage
├── icons/<size>/sailtree.png sizes 86x86, 108x108, 128x128, 172x172
├── sailtree.desktop          with [X-Sailjail] Sandboxing=Disabled
├── rpm/sailtree.spec         offline cargo build and install
├── translations/             optional .ts files
└── docs/
```

- `vendor/` is generated at packaging time (`cargo vendor`), not committed
- startup goes through a C++ shim calling `SailfishApp::application()` / `createView()` / `pathToMainQml()`

## build and test

- crate versions are limited to those building with the rust of the Sailfish SDK target (`rust-version` in `Cargo.toml`); `Cargo.lock` is regenerated on the host with `cargo generate-lockfile`
- `scripts/target-cargo.sh <cargo args>` runs cargo inside the SDK target, eg. `scripts/target-cargo.sh test`. unit tests only run there, the host has no Qt/Silica
- `sfdk -c target=SailfishOS-5.1.0.11-i486 build` produces `RPMS/sailtree-*.rpm`
- `scripts/emulator-run.sh [--no-build] [file.png]` builds, installs in the SDK emulator, starts the app, prints its output and saves a screenshot. it never targets a real device
- only the i486 target (emulator) is built for now
- the desktop file uses `X-Nemo-Application-Type=no-invoker`: the rust binary is started directly, without the booster
