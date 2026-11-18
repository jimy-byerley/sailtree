mod runtime;
mod sailfishapp;

fn main() {
    // start the worker threads before the gui so the first request doesn't pay for it
    runtime::runtime();

    let code = sailfishapp::run(|| {
        // QML types get registered here
    });
    std::process::exit(code);
}
