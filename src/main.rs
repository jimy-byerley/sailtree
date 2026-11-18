use std::ffi::CStr;

mod devices;
mod fs;
mod model;
mod runtime;
mod sailfishapp;
#[cfg(test)]
mod testing;

fn main() {
    // start the worker threads before the gui so the first request doesn't pay for it
    runtime::runtime();

    let code = sailfishapp::run(|| {
        // usable from QML with `import Sailtree 1.0`
        let module = CStr::from_bytes_with_nul(b"Sailtree\0").expect("valid C string");
        let name = CStr::from_bytes_with_nul(b"TreeModel\0").expect("valid C string");
        qmetaobject::qml_register_type::<model::tree::TreeModel>(module, 1, 0, name);
    });
    std::process::exit(code);
}
