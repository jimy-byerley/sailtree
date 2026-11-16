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

# ui definition

main page is a list of devices in 3 categories

- connected
- online
- offline

clicking a device expand/collapses. same for a folder under it

# connection settings

## sorting

online devices are shown

- local device always
- sftp and ftp are discovered using ARP requests

after online ones, a list of last 20 accessed devices is kept

clicking one online or not tries to connect to it, starting on remote user home folder. on connection success a offline one switches connected

any item starts collapsed, and every expanded item immediate childs is cached in RAM is scrolling is smooth

## item management

devices, folders, files can be selected by clicking a small circle on the right of the screen. files can be selected from different folders and devices

devices, folders, files can be expanded or opened by clicking their line

screen top bar:

- a menu button to 
  - enable the display of details (owner, permissions, date) for items
  - show hidden files
- a button '+' with a drop-down for adding device, folder, file

when there is a selection, the above buttons are replaced by actions on selection

- move, copy, rename, delete ...

long press on a device opens connection settings

## adding 

- remote devices can be added manually with '+/device'
- periodic ARP scanning reveals detected hosts, shown by name if there is a DNS on the network, automatically adding them to the online category

when connecting on a new or existing device, login is asked and a checkbox propose to save it to avoid prompting again