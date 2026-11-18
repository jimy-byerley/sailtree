# goal

make a local and remote file explorer app

- explore local file tree
- explore remote devices tree
  - sftp
  - ftp
- very good and simple ergonomy for mainstream file management
  - copy, move, rename
  - permissions
  - open with system default app
  - selection for grouped actions
  - show size, date, owner, permissions

# non-goal

- support non-fs like transmission protocols
- abstract the platform
- provide file viewers
- impose a file management style or folder structure



# technical stack

- language: rust
- gui: QML with sailfish silica and qmetaobject-rs
- package: rpm, targetting chum repo

# layout

main page is a list of devices in 3 categories

- connected
- online
- offline

clicking a device expand/collapses. same for a folder under it

file items show
- icon (varying by file type)
- file name with extension
- details (permissions, user, date, size)
- ring for ratio of size in current folder (only on local device)

## connection settings

- hostname
- port
- password
- fingerprint (if any)
- current folder

## sorting

online devices are shown

- local device always
- sftp and ftp are discovered using ARP requests

after online ones, a list of last 20 accessed devices is kept

clicking one online or not tries to connect to it, starting on remote user home folder. on connection success a offline one switches connected

any item starts collapsed, and every expanded item immediate childs is cached in RAM is scrolling is smooth

files in any folder can be sorted either by name (defautl), date, size

## item management

devices, folders, files can be 
- selected by clicking a small circle on the right of the screen. 
- expanded or opened by clicking their line
- files can be selected from different folders and devices

a device item has a current folder shown by a path
- clicking a level in this path sets current folder to the clicked level
- tree under this device starts from this current folder

screen top bar has a menu button to 
  - enable the display of details (owner, permissions, date) for items
  - show hidden files
  - sort by name, date, size

when there is a selection, the top bar content is replaced by actions on selection
- set as device current folder
- new file, folder
- move, copy, rename, delete ...

long press on a device opens connection settings

## adding 

- adding a device is a button show by dragging the full page down
- periodic ARP scanning reveals detected hosts, shown by name if there is a DNS on the network, automatically adding them to the online category

when connecting on a new or existing device, login is asked and a checkbox propose to save it to avoid prompting again
