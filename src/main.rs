use std::ffi::CStr;

mod config;
mod devices;
mod discovery;
mod fs;
mod model;
mod opener;
mod runtime;
mod sailfishapp;
mod staging;
mod transfer;
#[cfg(test)]
mod testing;

fn main() {
    // start the worker threads before the gui so the first request doesn't pay for it
    runtime::runtime();
    // links made for other applications, left by a run that did not exit normally
    let home = devices::Device::local().home;
    staging::clean(&home);

    let code = sailfishapp::run(|| {
        // usable from QML with `import Sailtree 1.0`
        let module = CStr::from_bytes_with_nul(b"Sailtree\0").expect("valid C string");
        let name = CStr::from_bytes_with_nul(b"TreeModel\0").expect("valid C string");
        qmetaobject::qml_register_type::<model::tree::TreeModel>(module, 1, 0, name);
    });
    staging::clean(&home);
    std::process::exit(code);
}
