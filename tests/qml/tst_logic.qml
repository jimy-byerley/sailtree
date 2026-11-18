import QtQuick 2.0
import QtTest 1.0
import "../../qml/js/logic.js" as Logic

TestCase {
    name: "logic"

    function test_indent() {
        compare(Logic.indent(0, 6), 0)
        compare(Logic.indent(3, 6), 3)
        compare(Logic.indent(6, 6), 6)
        compare(Logic.indent(42, 6), 6)
        compare(Logic.indent(-1, 6), 0)
    }

    function test_formatSize_data() {
        return [
            { bytes: 0, text: "0 B" },
            { bytes: 1, text: "1 B" },
            { bytes: 999, text: "999 B" },
            { bytes: 1000, text: "1 kB" },
            { bytes: 1500, text: "1.5 kB" },
            { bytes: 9999, text: "9.9 kB" },
            { bytes: 15300, text: "15 kB" },
            { bytes: 999999, text: "999 kB" },
            { bytes: 1000000, text: "1 MB" },
            { bytes: 2560000, text: "2.5 MB" },
            { bytes: 3000000000, text: "3 GB" },
            { bytes: 7200000000000, text: "7.2 TB" },
            { bytes: 5000000000000000, text: "5000 TB" }
        ]
    }

    function test_formatSize(data) {
        compare(Logic.formatSize(data.bytes), data.text)
    }

    function test_formatDate() {
        // built from local time, so the test does not depend on the timezone
        compare(Logic.formatDate(new Date(2026, 10, 18, 20, 25, 59).getTime()), "2026-11-18 20:25")
        compare(Logic.formatDate(new Date(2001, 0, 2, 3, 4, 5).getTime()), "2001-01-02 03:04")
    }

    function test_details() {
        var date = new Date(2026, 10, 18, 20, 25).getTime()
        compare(Logic.details("rw-r--r--", "user", "user", date, "", 1500),
                "rw-r--r--  user  2026-11-18 20:25  1.5 kB")
        // a folder whose content is not counted yet
        compare(Logic.details("rwxr-xr-x", "root", "wheel", date, "", -1),
                "rwxr-xr-x  root:wheel  2026-11-18 20:25")
        compare(Logic.details("rwxrwxrwx", "user", "user", date, "../target", 0),
                "rwxrwxrwx  user  2026-11-18 20:25  → ../target  0 B")
    }

    function test_validName() {
        verify(Logic.validName("file.txt"))
        verify(Logic.validName(".hidden"))
        verify(Logic.validName("with space"))
        verify(Logic.validName("..."))
        verify(!Logic.validName(""))
        verify(!Logic.validName("."))
        verify(!Logic.validName(".."))
        verify(!Logic.validName("a/b"))
        verify(!Logic.validName("/"))
    }

    function test_permissionBits() {
        compare(Logic.permissionBit(0, 0), 256) // owner read, 0400
        compare(Logic.permissionBit(0, 2), 64)  // owner execute, 0100
        compare(Logic.permissionBit(1, 1), 16)  // group write, 0020
        compare(Logic.permissionBit(2, 2), 1)   // others execute, 0001
    }

    function test_hasPermission() {
        var mode = 492 // 0754
        var expected = [[true, true, true], [true, false, true], [true, false, false]]
        for (var who = 0; who < 3; who++)
            for (var what = 0; what < 3; what++)
                compare(Logic.hasPermission(mode, who, what), expected[who][what], who + "/" + what)
    }

    function test_withPermission() {
        compare(Logic.withPermission(420, 0, 2, true), 484)  // 0644 + owner execute = 0744
        compare(Logic.withPermission(420, 2, 0, false), 416) // 0644 - others read = 0640
        compare(Logic.withPermission(420, 0, 0, true), 420)  // already set
        compare(Logic.withPermission(420, 2, 1, false), 420) // already clear
    }

    function test_permissionRoundTrip() {
        // rebuilding a mode switch by switch gives the same mode back
        for (var mode = 0; mode < 512; mode += 7) {
            var rebuilt = 0
            for (var who = 0; who < 3; who++)
                for (var what = 0; what < 3; what++)
                    rebuilt = Logic.withPermission(rebuilt, who, what, Logic.hasPermission(mode, who, what))
            compare(rebuilt, mode)
        }
    }

    function test_fileUrl() {
        compare(Logic.fileUrl("/home/user/notes.txt"), "file:///home/user/notes.txt")
        compare(Logic.fileUrl("/home/user/my file #1?.txt"), "file:///home/user/my%20file%20%231%3F.txt")
        compare(Logic.fileUrl("/"), "file:///")
    }

    function test_crumbs() {
        var crumbs = Logic.crumbs("/home/user/My Documents")
        compare(crumbs.length, 4)
        compare(crumbs[0].name, "/")
        compare(crumbs[0].path, "/")
        compare(crumbs[1].name, "home")
        compare(crumbs[1].path, "/home")
        compare(crumbs[3].name, "My Documents")
        compare(crumbs[3].path, "/home/user/My Documents")

        compare(Logic.crumbs("/").length, 1)
        compare(Logic.crumbs("").length, 1)
        // doubled and trailing separators do not make levels
        crumbs = Logic.crumbs("/srv//files/")
        compare(crumbs.length, 3)
        compare(crumbs[2].path, "/srv/files")
    }

    function test_title() {
        compare(Logic.title("connected"), "Connected")
        compare(Logic.title(""), "")
    }
}
