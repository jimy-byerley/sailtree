//! tokio runtime shared by the whole app, and the bridge back to the Qt thread

use std::future::Future;
use std::sync::OnceLock;

use tokio::runtime::{Builder, Runtime};
use tokio::task::JoinHandle;

static RUNTIME: OnceLock<Runtime> = OnceLock::new();

/// the shared runtime, started on first use
pub fn runtime() -> &'static Runtime {
    RUNTIME.get_or_init(|| {
        Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("sailtree-worker")
            .enable_all()
            .build()
            .expect("failed to start the tokio runtime")
    })
}

/// run `task` on the runtime, then give its result to `deliver` from a worker thread
pub fn spawn_then<T, F, D>(task: F, deliver: D) -> JoinHandle<()>
where
    T: Send + 'static,
    F: Future<Output = T> + Send + 'static,
    D: FnOnce(T) + Send + 'static,
{
    runtime().spawn(async move { deliver(task.await) })
}

/// run `task` on the runtime, then give its result to `callback` on the Qt thread.
///
/// must be called from the Qt thread. the callback is dropped without being called
/// if the task is aborted or panics.
#[allow(dead_code)] // first used by the tree model
pub fn spawn_to_qt<T, F, C>(task: F, callback: C) -> JoinHandle<()>
where
    T: Send + 'static,
    F: Future<Output = T> + Send + 'static,
    C: FnOnce(T) + 'static,
{
    let mut callback = Some(callback);
    let queued = qmetaobject::queued_callback(move |result: T| {
        if let Some(callback) = callback.take() {
            callback(result);
        }
    });
    spawn_then(task, move |result| queued(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    const TIMEOUT: Duration = Duration::from_secs(5);

    #[test]
    fn result_reaches_the_callback() {
        let (sender, receiver) = mpsc::channel();
        spawn_then(async { 6 * 7 }, move |result| sender.send(result).unwrap());
        assert_eq!(receiver.recv_timeout(TIMEOUT), Ok(42));
    }

    #[test]
    fn task_runs_on_a_worker_thread() {
        let (sender, receiver) = mpsc::channel();
        spawn_then(
            async { std::thread::current().name().map(str::to_owned) },
            move |name| sender.send(name).unwrap(),
        );
        let name = receiver.recv_timeout(TIMEOUT).unwrap();
        assert_eq!(name.as_deref(), Some("sailtree-worker"));
    }

    #[test]
    fn panicking_task_does_not_stop_the_runtime() {
        let (sender, receiver) = mpsc::channel::<()>();
        let handle = spawn_then(async { panic!("expected test panic") }, move |()| {
            sender.send(()).unwrap()
        });
        let joined = runtime().block_on(handle);
        assert!(joined.unwrap_err().is_panic());
        // the callback was dropped, not called
        assert!(receiver.recv_timeout(TIMEOUT).is_err());

        // and the runtime still accepts work
        let (sender, receiver) = mpsc::channel();
        spawn_then(async { "alive" }, move |result| sender.send(result).unwrap());
        assert_eq!(receiver.recv_timeout(TIMEOUT), Ok("alive"));
    }

    #[test]
    fn aborted_task_never_calls_back() {
        let (sender, receiver) = mpsc::channel();
        let handle = spawn_then(
            async { tokio::time::sleep(Duration::from_secs(60)).await },
            move |()| sender.send(()).unwrap(),
        );
        handle.abort();
        assert!(runtime().block_on(handle).unwrap_err().is_cancelled());
        assert!(receiver.try_recv().is_err());
    }
}
