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

// kinds of files the theme has an icon for, by extension
var iconKinds = {
    image: ["jpg", "jpeg", "png", "gif", "bmp", "webp", "tif", "tiff", "svg", "heic"],
    audio: ["mp3", "ogg", "oga", "flac", "wav", "m4a", "aac", "opus", "wma"],
    video: ["mp4", "mkv", "avi", "mov", "webm", "m4v", "3gp", "mpg", "mpeg", "ogv"],
    pdf: ["pdf"],
    document: ["doc", "docx", "odt", "rtf"],
    spreadsheet: ["xls", "xlsx", "ods", "csv"],
    presentation: ["ppt", "pptx", "odp"],
    formatted: ["html", "htm", "xml", "json", "md"],
    note: ["txt", "log"],
    compressed: ["zip", "tar", "gz", "tgz", "bz2", "xz", "7z", "rar"],
    apk: ["apk"],
    rpm: ["rpm"],
    vcard: ["vcf"]
}
// kinds whose icon exists in a variant for dark and for light backgrounds
var schemedIcons = ["pdf", "document", "spreadsheet", "presentation", "formatted", "note", "other"]

// kind of a file from its name, "other" when unknown
function fileKind(name) {
    var dot = name.lastIndexOf(".")
    if (dot <= 0)
        return "other"
    var extension = name.slice(dot + 1).toLowerCase()
    for (var kind in iconKinds)
        if (iconKinds[kind].indexOf(extension) >= 0)
            return kind
    return "other"
}

// theme icon of a file. lightOnDark: the ambience has light text on a dark background
function fileIcon(name, lightOnDark) {
    var kind = fileKind(name)
    var scheme = schemedIcons.indexOf(kind) >= 0 ? (lightOnDark ? "-dark" : "-light") : ""
    return "image://theme/icon-m-file-" + kind + scheme
}

// what a running transfer shows: "Copying photo.jpg", with the number of waiting ones
function transferLabel(moving, name, waiting) {
    var text = (moving ? "Moving " : "Copying ") + name
    return waiting > 0 ? text + " (+" + waiting + ")" : text
}

// capitalized category name for section headers
function title(text) {
    return text.length > 0 ? text.charAt(0).toUpperCase() + text.slice(1) : text
}
