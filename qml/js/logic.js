.pragma library

// logic of the interface that does not depend on visual items, so it can be unit tested

// number of indentation steps of a row: deep folders stop moving right to stay readable
function indent(depth, maximum) {
    return Math.max(0, Math.min(depth, maximum))
}

// human readable file size, eg. "512 B", "1.5 kB", "20 MB"
function formatSize(bytes) {
    var units = ["B", "kB", "MB", "GB", "TB"]
    var value = bytes
    var unit = 0
    while (value >= 1000 && unit < units.length - 1) {
        value /= 1000
        unit += 1
    }
    if (unit === 0)
        return value + " B"
    // one decimal below 10, none above: 1.5 kB, 15 kB
    var text = value < 10 ? (Math.floor(value * 10) / 10).toString() : Math.floor(value).toString()
    return text + " " + units[unit]
}

function pad(number) {
    return number < 10 ? "0" + number : "" + number
}

// local date and time from milliseconds since the epoch, eg. "2026-11-18 20:25"
function formatDate(milliseconds) {
    var date = new Date(milliseconds)
    return date.getFullYear() + "-" + pad(date.getMonth() + 1) + "-" + pad(date.getDate())
            + " " + pad(date.getHours()) + ":" + pad(date.getMinutes())
}

// second line of a row when details are shown. the size comes last, and is left out while
// it is not known (negative)
function details(permissions, owner, group, date, link, size) {
    var parts = [permissions, owner === group ? owner : owner + ":" + group, formatDate(date)]
    if (link)
        parts.push("→ " + link)
    if (size >= 0)
        parts.push(formatSize(size))
    return parts.join("  ")
}

// whether a text can be the name of a file or folder
function validName(name) {
    return name.length > 0 && name !== "." && name !== ".." && name.indexOf("/") < 0
}

// permission bits: who is 0 for the owner, 1 for the group, 2 for others;
// what is 0 for read, 1 for write, 2 for execute
function permissionBit(who, what) {
    return 1 << ((2 - who) * 3 + (2 - what))
}

function hasPermission(mode, who, what) {
    return (mode & permissionBit(who, what)) !== 0
}

function withPermission(mode, who, what, allowed) {
    var bit = permissionBit(who, what)
    return allowed ? (mode | bit) : (mode & ~bit)
}

// url opening a local file with the application the system associates to it
function fileUrl(path) {
    return "file://" + path.split("/").map(encodeURIComponent).join("/")
}

// levels of the current folder of a device, from the root down: each has the name to show
// and the path it stands for
function crumbs(path) {
    var result = [{ name: "/", path: "/" }]
    var at = ""
    var parts = path.split("/")
    for (var i = 0; i < parts.length; i++) {
        if (parts[i].length > 0) {
            at += "/" + parts[i]
            result.push({ name: parts[i], path: at })
        }
    }
    return result
}

// capitalized category name for section headers
function title(text) {
    return text.length > 0 ? text.charAt(0).toUpperCase() + text.slice(1) : text
}
