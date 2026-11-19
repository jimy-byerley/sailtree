# implementation choices

## tree view

- the device/folder tree is flattened into a single rust `QAbstractListModel` (qmetaobject-rs), one row per visible node
- roles: `depth`, `name`, `kind` (device/folder/file), `expanded`, `hasChildren`, `selected`, `category`
- expanding inserts the children rows right after the node, with a temporary "loading" row while the listing runs asynchronously
- collapsing removes the contiguous range of descendant rows
- QML side is a single `SilicaListView` with `ListItem` delegates, indented by `depth` (capped visually), `section.property: "category"` for the connected/online/offline headers
- a device changing category moves its whole row block (`begin_move_rows`)
- selection is stored in rust as a set of `(device, path)`, never as row indices
- more roles for the details: `selectable`, `path`, `size`, `date` (milliseconds), `owner`, `group`, `permissions` (`rwxr-xr-x`), `mode`, `link` (symlink target), `ratio` (see "size ring")
- row changes are planned by Qt-free code (`model/node.rs`) as insert / remove / update steps, that the Qt model applies one by one between the matching begin/end notifications
- only the visible rows are kept: collapsing a folder forgets what was expanded inside it
- inside a folder: folders always first, then the chosen sort, then names ignoring case
  - by date: most recently modified first
  - by size: biggest files first; folders have no size of their own and stay ordered by name
- hidden items (leading dot) are filtered in rust; toggling them or changing the sort resets the model
- a folder that cannot be listed shows an error row under it
- symlinks are shown as what they point to (a link to a folder expands), broken links as files
- what stops being displayed (collapsed folder, hidden items turned off, refreshed listing, change of current folder) is removed from the selection, so no action applies to something the user does not see
- a selected device stays selected through collapsing and changes of its current folder
- the current folder of a device is the path of its row; the folder is not repeated as a row of its own

## main page

- top bar without selection: menu button, app name, sort button. each opens a drop-down under the bar that pushes the list down
  - menu: "show details" and "show hidden files" switches
  - sort: by name, date, size; the current one is highlighted
- top bar with a selection: clear, count, then rename, permissions, copy, move, delete, `+`, set as current folder (rightmost)
  - rename and permissions: a single folder or file
  - copy, move, delete: apply to the selected folders and files, selected devices are left alone
  - `+`: a single item of any kind. it opens a drop-down with folder and file
  - set as current folder: a single folder
- copy, move, `+`, set as current folder and sort have their own icons (`qml/icons/*.svg`, white strokes colored by Silica like the theme's icons), the theme having none that fit. they need the Qt svg image plugin at run time
- delete starts after the usual Silica remorse countdown, which lets the user cancel
- `NameDialog` serves rename and creation; `PermissionsDialog` has read/write/execute switches for owner, group, others
- a device row has a second line with its current folder, one clickable label per level starting at `/`. a long path scrolls horizontally and shows its end
- changing the current folder of a device expands it and brings it to the top of the view; clicking the last level lists the folder again
- "add device" is the entry of a Silica pull-down menu on the list; it opens the connection settings of a new device
- a long press on a remote device opens its connection settings; nothing happens on the local device, which has none
- details are a second line under the name: permissions, owner (and group when different), date, link target, and for files the size last
- file icons follow the file extension, among the kinds the theme has an icon for (image, audio, video, pdf, document, spreadsheet, presentation, formatted text, note, archive, apk, rpm, vcard)
- a click on a file opens it with the application the system associates to it (`Qt.openUrlExternally`), see "opening files"
- rows are indented by depth up to 6 levels, deeper ones stay at the same indentation
- failed operations show a Silica notice with the error message
- the page expands the local device when it opens

## size ring

- a small ring on the right of a file, before the selection circle, shows its share of the files directly in the folder it is in (not of the device's current folder)
- the sizes come from the listing of the folder, so the ring is immediate. the content of folders is not counted: folders have no ring and weigh nothing in the total
- hidden files are part of the total even when they are not shown, so the rings of what is shown may not add up to a full one
- only for the local device (`ratio` is negative when there is none)

## copy and move

- copy or move in the selection actions keeps the selected files and folders aside and clears the selection. the top bar then shows "Copy/Move n items to…" with a cancel button, and device and folder rows show a button instead of the selection circle: it starts the transfer into that folder, or into the current folder of that device
- the items kept aside do not depend on what is displayed, so the tree can be browsed and the current folder changed to reach the destination
- transfers run one at a time, the next ones wait (`transfer/job.rs` does one, the model holds the queue). items coming from several devices make one transfer per device
- files travel by pieces of 256 KiB through the common `Fs` operations, so the same code serves inside a device and between two. inside a device a move is first tried as a rename, which is immediate on the same filesystem
- a row under the top bar shows the progress in bytes, the item being transferred, the number of waiting transfers, and a button that cancels the running transfer and the waiting ones. a cancelled file leaves nothing half written; what was already transferred stays
- when the name is taken in the destination, `ConflictDialog` asks: keep both (numbered name, `photo (2).jpg`), replace, or skip, optionally for all the next conflicts of the transfer. going back skips. replacing a folder by a folder merges them, asking again for what conflicts inside
- a copy to the folder the item is already in makes a numbered duplicate without asking; a move there does nothing; a folder cannot go inside itself
- a moved folder is only removed from its source when everything in it went; with a skip inside, what was not transferred stays there
- a copy keeps symbolic links as links and the permissions
- the destination is expanded to show what arrived

## settings

- kept in `~/.config/sailtree/config.ini` (sections of `key=value` lines, private to the user), written at each change: show details, show hidden files, sort, and the current folder of the local device
- a saved current folder that does not exist anymore falls back to the home folder

## local device

- always present and connected from the start (no connection step), listed first, named after the hostname
- its current folder is the home folder the first time, then the one it had when the app was last used
- creating a folder or file targets the selected item: inside a folder, in the current folder of a device, next to a file. a collapsed target is expanded to show the new item
- rename refuses a name already taken instead of overwriting
- deleting a folder deletes its content; deleting a symlink never touches its target

## remote devices

- a remote device is reached through ssh (sftp) or FTP. its connection settings (`DeviceDialog`): protocol (the port follows it unless another one was typed), host, port, user, password with a "remember the password" switch, name shown in the list (the host by default), current folder, and the fingerprint of its key once trusted. the dialog also disconnects and forgets the device
- clicking a device that is not connected connects it, then expands it. `LoginDialog` asks the user and password when no password is known, or after a refused one. a password not remembered is kept for the run only
- the key of a device is checked: at the first connection, or when the key is not the trusted one anymore, `HostKeyDialog` shows its fingerprint (`SHA256:…`, as ssh tools print it) and connects only if the user trusts it
- FTP: plain FTP only, nothing is encrypted, which the settings say when FTP is chosen. no key to trust. passive mode (`EPSV`, then `PASV` ignoring the address it announces). listings use `MLSD` when the server has it, asking for the unix permissions, owner and group, and fall back to parsing `LIST` (no dates then). each file being transferred has a connection of its own, browsing keeps the first one. links cannot be created through FTP; permissions a server does not tell are shown empty, and a file copied from there gets the usual `rw-r--r--`
- servers asking the password as a question (keyboard-interactive) are answered with it; a server accepting no password is reported as such. key based login is not supported
- a device starts in its remembered current folder, the home folder of the user the first time
- categories: connected; online when the device answers on its port (checked when the app starts, then every 30 seconds while it is in use, and after a disconnection); offline otherwise. a device changing category moves, with what is shown under it, to the end of its new category
- connected devices are checked at the same pace: a connection attempt on their port, then a harmless request on the open session (sftp `realpath`, ftp `NOOP`). a device failing the first is closed and listed offline, one failing the second is closed and listed online, with a notice. a cut network is otherwise not told by the system before long
- a session is never dropped on the Qt thread: closing it tells the device and waits for its answer (2 seconds at most for ssh)
- devices are remembered in the configuration (`[device N]` sections), with the password only if asked. at most 20: beyond, the least recently connected one is forgotten
- owner and group of remote items are numbers: the protocol gives no names
- copies and moves between devices go through the same transfer jobs. a remote file is opened by first fetching it to the folder used for temporary links, with the usual progress
- the size ring is not shown on remote devices

## opening files

- viewers are sandboxed (Sailjail) and only see `Documents`, `Downloads`, `Music`, `Pictures`, `Videos`, `Public` of the home folder and removable media (`/run/media`). a file there is opened in place
- any other file is opened through a temporary hard link, `~/Downloads/.sailtree-open/<number>/<file name>`: no copy, and changes made by the viewer reach the file
- the links are removed when the app exits, and when it starts in case it was killed; never the files they point to. a viewer still showing a linked file after that cannot reload it
- when no link can be made (file on another filesystem than the home folder, file owned by another user) the original path is opened anyway and the viewer reports that it cannot read it
- empty files: the system picks the application from the content of a file and has none for an empty one. the application is then chosen from the file name (`/usr/share/mime/globs2`), plain text by default, and started with `lca-tool --triggerdesktop`. the text viewer also judges an empty file by its name, so unless it ends with `.txt` it is opened through a link named `<file name>.txt`

## discovery

- no ARP: raw sockets need `CAP_NET_RAW`, unavailable to a regular app
- `mdns-sd` crate browses `_sftp-ssh._tcp`, `_ssh._tcp` and `_ftp._tcp` continuously once the app looked at the network, so advertising hosts show up with their `.local` name and port
- a TCP connect sweep of ports 22 then 21 over each local IPv4 network completes the list (300 ms per host, 32 at a time; a network bigger than a /24 is only swept around the address of the phone). it runs when the app starts and every 2 minutes while the app is in use
- hosts found only by the sweep get their name from the system reverse lookup (`getnameinfo`), falling back to the address
- a device offering both ssh and FTP is listed once, through ssh
- results from both sources are merged per address, the addresses of the phone left out, as well as the devices already remembered (by host name, or by an address their host resolves to)
- a discovered device is listed online, reached by its address, with no user: connecting asks for the login. it is not remembered until connected or edited, and leaves the list when it is not seen anymore

## transfers

- SFTP is the only transfer path for ssh devices; interrupted transfers resume by stat + seek on the partial destination file
- longer term: optional rsync acceleration when `rsync` is available on both ends, run in an exec channel of the same ssh session (`rsync --server`). no rsync daemon (`rsync://`) support planned

## concurrency and backends

- tokio runtime running in background threads; QML-invoked methods spawn onto it, results come back to the Qt thread with `qmetaobject::queued_callback`
- SFTP via the `ssh2` crate (libssh2, built with the app and linked statically on the system openssl): one session per device, blocking calls run in `spawn_blocking`, one at a time on the session. transfers go by pieces, so browsing gets its turn between two of them
- FTP by a client of our own on blocking sockets (`fs/ftp.rs`, no crate): one operation at a time on the control connection, transfers use a separate connection
- local filesystem calls run in `spawn_blocking`
- backends are dispatched through `enum Fs { Local, Sftp, Ftp }`, not a dyn trait
- cancellation through task abort / cancellation token, network calls wrapped in `tokio::time::timeout`
- discovery runs as one blocking task of the same runtime: the sweep uses its own short lived threads, the mdns-sd daemon and its listeners their own

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
│   ├── main.rs               startup, type registration (QML module `Sailtree 1.0`)
│   ├── testing.rs            helpers shared by the rust unit tests
│   ├── sailfishapp.rs        rust side of the SailfishApp shim
│   ├── runtime.rs            tokio runtime, bridge to the Qt thread (queued_callback)
│   ├── staging.rs            makes files visible to the sandboxed applications opening them
│   ├── opener.rs             opens the files the system default cannot (empty files)
│   ├── config.rs             settings and devices remembered between runs (~/.config/sailtree/config.ini)
│   ├── model/                QML-facing models
│   │   ├── tree.rs           flattened QAbstractListModel
│   │   ├── node.rs           node data
│   │   └── selection.rs      (device, path) selection and grouped actions
│   ├── fs/                   filesystem backends
│   │   ├── mod.rs            `enum Fs` and common operations
│   │   ├── local.rs
│   │   ├── sftp.rs           ssh2 (libssh2)
│   │   └── ftp.rs            own client on blocking sockets
│   ├── devices/
│   │   └── mod.rs            device identity, connected/online/offline state, how devices are remembered
│   ├── discovery/
│   │   ├── mdns.rs           mdns-sd browsing
│   │   ├── scan.rs           TCP sweep on ports 22/21
│   │   └── resolve.rs        system reverse lookup
│   └── transfer/
│       ├── job.rs            one copy or move, inside a device or between two: progress, cancellation
│       └── conflict.rs       overwrite / skip / keep both
├── qml/                      installed to /usr/share/sailtree/qml, not embedded
│   ├── sailtree.qml          ApplicationWindow
│   ├── pages/                full pages and dialogs (MainPage, DeviceDialog, LoginDialog, HostKeyDialog, NameDialog, PermissionsDialog, ConflictDialog)
│   ├── components/           reusable items (TreeDelegate, SelectionCircle, SelectionActions, ActionButton)
│   ├── icons/                action icons missing from the theme, svg
│   ├── js/logic.js           interface logic without visual items (formatting, validation, permission bits)
│   └── cover/                CoverPage
├── tests/qml/                QML unit tests (QtTest), rust unit tests stay next to the code
├── icons/sailtree.svg        launcher icon, the reference drawing
├── icons/<size>/sailtree.png sizes 86x86, 108x108, 128x128, 172x172, drawn by scripts/make-icons.py
├── sailtree.desktop          with [X-Sailjail] Sandboxing=Disabled
├── rpm/sailtree.spec         offline cargo build and install
├── translations/             optional .ts files
└── docs/
```

- the launcher icon follows the Sailfish app icon guide: its fully rounded base shape (a circle), drawn on 86 px with no padding but a 0.3 px gap, a greyscale gradient as background and a simple white picture inside, here a small tree of items
- the cover shows the full path of the current folder of the local device, its last level in the accent color as in the tree (over three lines at most: first levels are replaced by an ellipsis until it fits), and its first items in the display order of the tree (same sort and hidden files setting, folders ending with a `/`), as many as fit, the last line telling how many are left out, and a refresh button at the bottom. behind them, the picture of the launcher icon, made as the cover graphics of the system's apps (`graphic-cover-*.png` of the theme, shown by Settings with `anchors.fill`): an image of the size of a cover, white only and mostly transparent, the picture large and cut by the edges (`qml/icons/cover.png`, drawn by `scripts/make-icons.py`). the model lists that folder on its own (`refreshCover`, at most 20 names kept) when the cover appears, when the current folder changes, when a transfer ends and on the button, so it does not depend on the device being expanded in the tree. the cover gets the model of the main page through the application window
- `vendor/` is generated at packaging time (`cargo vendor`), not committed
- startup goes through a C++ shim calling `SailfishApp::application()` / `createView()` / `pathToMainQml()`

## build and test

- crate versions are limited to those building with the rust of the Sailfish SDK target (`rust-version` in `Cargo.toml`); `Cargo.lock` is regenerated on the host with `cargo generate-lockfile`
- `scripts/target-cargo.sh <cargo args>` runs cargo inside the SDK target, eg. `scripts/target-cargo.sh test`. unit tests only run there, the host has no Qt/Silica
- `scripts/target-qmltest.sh` runs the QML unit tests of `tests/qml` inside the SDK target (`qmltestrunner`, `minimal` Qt platform)
- every step comes with its unit tests; filesystem tests work in temporary folders, the tree model is tested against a temporary folder without a GUI
- `sfdk -c target=SailfishOS-5.1.0.11-i486 build` produces `RPMS/sailtree-*.rpm`
- `scripts/emulator-run.sh [--no-build] [file.png]` builds, installs in the SDK emulator (rpm over ssh), starts the app, prints its output and saves a screenshot. it never targets a real device
- only the i486 target (emulator) is built for now
- the desktop file uses `X-Nemo-Application-Type=no-invoker`: the rust binary is started directly, without the booster
