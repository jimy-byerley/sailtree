//! startup through libsailfishapp, so the app gets the Silica theme, scaling and paths

use std::ffi::CString;
use std::os::raw::{c_char, c_int};

use cpp::cpp;

cpp! {{
    #include <sailfishapp.h>
    #include <QtGui/QGuiApplication>
    #include <QtQuick/QQuickView>
}}

/// create the application, call `setup`, show the main QML file and run the event loop.
///
/// the main QML file is `/usr/share/<binary name>/qml/<binary name>.qml`.
/// returns the exit code of the event loop.
pub fn run(setup: impl FnOnce()) -> i32 {
    // Qt keeps references to argc and argv for the whole application life
    let args: Vec<CString> = std::env::args()
        .filter_map(|arg| CString::new(arg).ok())
        .collect();
    let mut argv: Vec<*mut c_char> = args.iter().map(|arg| arg.as_ptr() as *mut c_char).collect();
    argv.push(std::ptr::null_mut());
    let argc: &'static mut c_int = Box::leak(Box::new(args.len() as c_int));
    let argv: *mut *mut c_char = Box::leak(argv.into_boxed_slice()).as_mut_ptr();
    std::mem::forget(args);

    let argc: *mut c_int = argc;
    cpp!(unsafe [argc as "int*", argv as "char**"] {
        SailfishApp::application(*argc, argv);
    });

    setup();

    cpp!(unsafe [] -> i32 as "int" {
        QQuickView *view = SailfishApp::createView();
        view->setSource(SailfishApp::pathToMainQml());
        view->show();
        return QGuiApplication::exec();
    })
}
