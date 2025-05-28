/// Bridge between synchronous and asynchronous code by spawning a tokio task and a tokio channel
/// with one async end and one blocking end.
macro_rules! sync_to_async {
    ($tokio:ident, $f:expr) => {{
        let (result_sender, result_receiver) = tokio::sync::oneshot::channel();
        $tokio.spawn(async move {
            let r = $f.await;
            result_sender.send(r).expect("Failed to send result");
        });
        result_receiver
            .blocking_recv()
            .expect("Failed to receive result")
    }};
}
pub(crate) use sync_to_async;
