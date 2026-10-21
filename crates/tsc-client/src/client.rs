use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use dashmap::DashMap;
use tokio::process::{ChildStdin, ChildStdout};
use tokio::sync::{mpsc, oneshot, Mutex as AsyncMutex, Notify};
use tokio_stream::StreamExt;
use tokio_util::codec::FramedRead;

use crate::error::TscClientError;
use crate::protocol::{self, IncomingMessage, Notification, RequestId, RequestMessage};
use crate::transport::{self, ContentLengthDecoder};

/// Whole frames queued for the writer task. Bounds memory when the child stops reading stdin: past this, callers wait
/// in `request()` under their own deadline instead of buffering without limit. The bound counts frames, not bytes.
const WRITE_QUEUE_FRAMES: usize = 16;

type PendingSender = oneshot::Sender<Result<TaggedResponse, TscClientError>>;

/// A response's decoded `result`, tagged with `seq`, the wire-arrival order it was read in. A caller can tell whether
/// an answer could reflect an update it already saw acknowledged (`answer.seq > update_ack.seq`).
#[derive(Debug, Clone)]
pub struct TaggedResponse {
    pub value: serde_json::Value,
    pub seq: u64,
}

/// State shared with the reader and writer tasks. Kept apart from [`ClientInner`] so the tasks never hold the write
/// queue's `Sender`: when the last `Client` clone drops the queue closes and the writer releases stdin.
struct Shared {
    pending: DashMap<u64, PendingSender>,
    /// Wire-arrival order, stamped by the reader task as each frame is decoded — not
    /// issue order, not wall clock. Lets a caller mechanically check whether a query's
    /// answer could reflect an update it already saw acknowledged
    /// (`answer.seq > update_ack.seq`), the concrete slice of freshness tracking this
    /// crate can build without the real (still-churning) `updateSnapshot` wire shape.
    next_seq: AtomicU64,
    /// Why the session ended. Set once; the first closer wins.
    closed: OnceLock<TscClientError>,
    /// Wakes the writer so it drops `ChildStdin` even mid-write and while `Client` clones are alive.
    stop_writer: Notify,
}

impl Shared {
    fn close(&self, reason: TscClientError) {
        if self.closed.set(reason.clone()).is_err() {
            return;
        }
        self.stop_writer.notify_one();
        self.fail_all_pending(&reason);
    }

    fn dispatch(&self, body: &[u8], seq: u64, notif_tx: &mpsc::UnboundedSender<Notification>) {
        match protocol::parse_incoming(body) {
            Ok(IncomingMessage::Response(response)) => {
                if let Some((_, tx)) = self.pending.remove(&response.id.0) {
                    let result = if let Some(error) = response.error {
                        Err(TscClientError::RpcError { code: error.code, message: error.message, data: error.data })
                    } else {
                        Ok(TaggedResponse { value: response.result.unwrap_or(serde_json::Value::Null), seq })
                    };
                    // A dropped receiver means the caller already gave up (e.g. timed
                    // out) — nothing left to deliver to.
                    tx.send(result).ok();
                }
            }
            Ok(IncomingMessage::Notification(notification)) => {
                notif_tx.send(Notification { method: notification.method, params: notification.params }).ok();
            }
            Err(source) => {
                // A malformed *body* doesn't desync the stream — the declared
                // Content-Length was honored — so the session stays alive; this one
                // frame is just unusable.
                tracing::warn!("dropping unparsable tsc frame: {source}");
            }
        }
    }

    /// Delivers the session-ending failure to every caller still waiting on a response.
    /// `TscClientError` is cheaply `Clone`, so every waiter gets its own full copy.
    fn fail_all_pending(&self, err: &TscClientError) {
        let ids: Vec<u64> = self.pending.iter().map(|entry| *entry.key()).collect();
        for id in ids {
            if let Some((_, tx)) = self.pending.remove(&id) {
                tx.send(Err(err.clone())).ok();
            }
        }
    }
}

struct ClientInner {
    shared: Arc<Shared>,
    /// The only path from a caller to the child's stdin: whole request bodies for the writer task.
    write_tx: mpsc::Sender<Vec<u8>>,
    next_id: AtomicU64,
    notifications: AsyncMutex<Option<mpsc::UnboundedReceiver<Notification>>>,
}

/// Removes a request's pending entry on every exit path, including a dropped future.
struct PendingEntry<'a> {
    pending: &'a DashMap<u64, PendingSender>,
    id: u64,
}

impl Drop for PendingEntry<'_> {
    fn drop(&mut self) {
        self.pending.remove(&self.id);
    }
}

/// Held by each I/O task so that any exit, including a panic, ends the session. A no-op once a reason is set.
struct CloseOnDrop(Arc<Shared>);

impl Drop for CloseOnDrop {
    fn drop(&mut self) {
        self.0.close(TscClientError::Crashed { status: None });
    }
}

/// Sole owner of `ChildStdin`. Writes one whole frame per queued body, so a request future dropped mid-flight can
/// never leave a partial frame on the pipe.
async fn write_loop(mut stdin: ChildStdin, mut frames: mpsc::Receiver<Vec<u8>>, shared: Arc<Shared>) {
    let _close = CloseOnDrop(Arc::clone(&shared));
    loop {
        let body = tokio::select! {
            () = shared.stop_writer.notified() => return,
            body = frames.recv() => match body {
                Some(body) => body,
                None => return,
            },
        };
        tokio::select! {
            () = shared.stop_writer.notified() => return,
            result = transport::write_frame(&mut stdin, &body) => {
                if let Err(source) = result {
                    shared.close(TscClientError::Write(Arc::new(source)));
                    return;
                }
            }
        }
    }
}

async fn read_loop(
    stdout: ChildStdout,
    max_frame_bytes: usize,
    shared: Arc<Shared>,
    notif_tx: mpsc::UnboundedSender<Notification>,
) {
    let _close = CloseOnDrop(Arc::clone(&shared));
    let mut framed = FramedRead::new(stdout, ContentLengthDecoder::new(max_frame_bytes));
    let end = loop {
        match framed.next().await {
            Some(Ok(body)) => {
                let seq = shared.next_seq.fetch_add(1, Ordering::SeqCst);
                shared.dispatch(&body, seq, &notif_tx);
            }
            Some(Err(err)) => break err,
            // The decoder's `decode_eof` distinguishes a clean end (`Ok(None)`,
            // stream ends here) from an end mid-frame (`Err`, handled above) —
            // reaching `None` from the stream itself is always the clean case.
            None => break TscClientError::Crashed { status: None },
        }
    };
    shared.close(end);
}

/// A handle to a running `tsc --api` session's JSON-RPC transport. Cheap to `Clone`
/// (an `Arc` underneath) so multiple concurrent callers can share one connection.
///
/// The session owns its I/O: a writer task alone holds the child's stdin, a reader task owns stdout, and the session
/// closes exactly once with a reason ([`Client::closed`]).
///
/// This is a raw request/response client, not a typed wrapper over TypeScript's
/// checker methods — implementing the actual algorithm
/// (`docs/type-checker-integration.md`, "The algorithm") is a separate, later pass;
/// this crate only has to prove the transport and process lifecycle are sound.
#[derive(Clone)]
pub struct Client(Arc<ClientInner>);

impl Client {
    /// Moves `stdin` into the writer task and `stdout` into the reader task. Each closes the session with the
    /// reason it ended on; liveness is read from [`Client::closed`].
    pub(crate) fn connect(stdin: ChildStdin, stdout: ChildStdout, max_frame_bytes: usize) -> Self {
        let (notif_tx, notif_rx) = mpsc::unbounded_channel();
        let (write_tx, write_rx) = mpsc::channel(WRITE_QUEUE_FRAMES);
        let shared = Arc::new(Shared {
            pending: DashMap::new(),
            next_seq: AtomicU64::new(1),
            closed: OnceLock::new(),
            stop_writer: Notify::new(),
        });

        tokio::spawn(write_loop(stdin, write_rx, Arc::clone(&shared)));
        tokio::spawn(read_loop(stdout, max_frame_bytes, Arc::clone(&shared), notif_tx));

        Self(Arc::new(ClientInner {
            shared,
            write_tx,
            next_id: AtomicU64::new(1),
            notifications: AsyncMutex::new(Some(notif_rx)),
        }))
    }

    /// `Some(reason)` once the session has ended, `None` while it is live. Set once — every later call returns a
    /// clone of the same reason.
    pub fn closed(&self) -> Option<TscClientError> {
        self.0.shared.closed.get().cloned()
    }

    /// Ends the session with [`TscClientError::SessionClosed`] unless it already ended: fails every pending request
    /// and makes the writer drop stdin, so the child sees EOF even while other clones are alive.
    pub(crate) fn close(&self) {
        self.0.shared.close(TscClientError::SessionClosed);
    }

    /// Sends a raw JSON-RPC request and awaits its response. The whole call — waiting for queue space and for the
    /// response — is bounded by `deadline`, after which it returns `Timeout`. A session that closes first fails the
    /// call with the reason it closed with. A response that arrives after the deadline is a no-op send into a
    /// dropped receiver — it never gets attributed to a later, unrelated call.
    pub async fn request(
        &self,
        method: &str,
        params: serde_json::Value,
        deadline: Duration,
    ) -> Result<TaggedResponse, TscClientError> {
        match tokio::time::timeout(deadline, self.enqueue_and_wait(method, params)).await {
            Ok(result) => result,
            Err(_elapsed) => Err(TscClientError::Timeout { method: method.to_string(), after: deadline }),
        }
    }

    async fn enqueue_and_wait(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<TaggedResponse, TscClientError> {
        let shared = &self.0.shared;
        if let Some(reason) = self.closed() {
            return Err(reason);
        }

        let id = self.0.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        shared.pending.insert(id, tx);
        let _entry = PendingEntry { pending: &shared.pending, id };

        // A close between the first check and the insert already drained `pending` without this entry.
        if let Some(reason) = self.closed() {
            return Err(reason);
        }

        let message = RequestMessage { id: RequestId(id), method: method.to_string(), params };
        let body = serde_json::to_vec(&message).map_err(|source| TscClientError::Serialize(Arc::new(source)))?;

        if self.0.write_tx.send(body).await.is_err() {
            return Err(self.closed().unwrap_or(TscClientError::SessionClosed));
        }

        match rx.await {
            Ok(result) => result,
            Err(_sender_dropped) => Err(self.closed().unwrap_or(TscClientError::SessionClosed)),
        }
    }

    /// Single-consumer: returns `Some` exactly once, `None` on every later call. Keeps
    /// `Client` itself cheaply `Clone` without also fanning notifications out to every
    /// clone.
    pub fn take_notifications(&self) -> Option<mpsc::UnboundedReceiver<Notification>> {
        self.0.notifications.try_lock().ok()?.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shared() -> Arc<Shared> {
        Arc::new(Shared {
            pending: DashMap::new(),
            next_seq: AtomicU64::new(1),
            closed: OnceLock::new(),
            stop_writer: Notify::new(),
        })
    }

    #[tokio::test]
    async fn dropping_a_task_guard_closes_the_session_and_fails_waiters() {
        let shared = shared();
        let (tx, rx) = oneshot::channel();
        shared.pending.insert(1, tx);

        drop(CloseOnDrop(Arc::clone(&shared)));

        assert!(matches!(shared.closed.get(), Some(TscClientError::Crashed { status: None })));
        assert!(matches!(rx.await, Ok(Err(TscClientError::Crashed { status: None }))));
    }

    #[test]
    fn dropping_a_pending_entry_removes_it_from_the_map() {
        let shared = shared();
        let (tx, _rx) = oneshot::channel();
        shared.pending.insert(7, tx);
        let (other_tx, _other_rx) = oneshot::channel();
        shared.pending.insert(8, other_tx);

        drop(PendingEntry { pending: &shared.pending, id: 7 });

        assert!(!shared.pending.contains_key(&7));
        assert!(shared.pending.contains_key(&8), "only the guarded id is removed");
    }

    #[test]
    fn a_guard_dropped_after_a_reason_is_set_keeps_that_reason() {
        let shared = shared();
        shared.close(TscClientError::SessionClosed);
        drop(CloseOnDrop(Arc::clone(&shared)));
        assert!(matches!(shared.closed.get(), Some(TscClientError::SessionClosed)));
    }
}
