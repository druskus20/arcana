use std::{fmt::Display, sync::Arc};

use thiserror::Error;
use tokio_util::sync::CancellationToken;
use tracing::error;

use crate::ToGladosMsg;

#[derive(Debug, Error)]
pub enum TaskError {
    #[error("Async Task finished with unexpected failure")]
    TaskFailed(Box<dyn Send + std::fmt::Debug>),
    #[error("Failed to receive oneshot message")]
    OneShotRecvError(#[from] tokio::sync::oneshot::error::RecvError),
    #[error("Error sending message")]
    TrySendError(#[from] tokio::sync::mpsc::error::TrySendError<ToGladosMsg>),
    #[error("Error in cancellation function")]
    CancellationError(Box<dyn std::fmt::Debug + Send>),
}

#[derive(Debug, Error)]
pub enum JoinHandleError {
    #[error("Failed to join tokio task: {0}")]
    TokioJoin(#[from] tokio::task::JoinError),
    #[error("Opaque OS thread join error")]
    OSThreadJoin(Box<dyn std::any::Any + Send>),
    #[error("Failed to receive result from OS thread join")]
    OneShotRecv(#[from] tokio::sync::oneshot::error::RecvError),
}

#[derive(Debug)]
pub struct TaskHandle {
    name: Arc<str>,
    pub(crate) id: uuid::Uuid,
    kind: TaskKind,
    /// Not unique, an id of a previous task might be reused (at least with Tokio)
    platform_id: _TaskPlatformId,
    pub(crate) join_handle: JoinHandle,
    cancellation_token: CancellationToken,
}

impl TaskHandle {
    pub fn new(name: &str, join_handle: JoinHandle, cancellation_token: CancellationToken) -> Self {
        let kind = match &join_handle {
            JoinHandle::Tokio(_) => TaskKind::Async,
            JoinHandle::OSThread(_) => TaskKind::OSThread,
        };
        let platform_id = match &join_handle {
            JoinHandle::Tokio(handle) => _TaskPlatformId::_Tokio(handle.id()),
            JoinHandle::OSThread(handle) => _TaskPlatformId::_OS(handle.thread().id()),
        };

        Self {
            name: Arc::from(name),
            id: uuid::Uuid::new_v4(),
            kind,
            platform_id,
            join_handle,
            cancellation_token,
        }
    }

    pub(crate) fn cancel(&self) {
        self.cancellation_token.cancel()
    }
}

impl Display for TaskHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "TaskHandle {{ name: {}, id: {}, kind: {:?}, platform_id: {:?} }}",
            self.name, self.id, self.kind, self.platform_id
        )
    }
}

#[derive(Debug)]
pub enum TaskKind {
    Async,
    _Blocking,
    OSThread,
}

#[allow(dead_code)]
#[derive(Debug)]
enum _TaskPlatformId {
    _Tokio(tokio::task::Id),
    _OS(std::thread::ThreadId),
}

#[derive(Debug)]
pub enum JoinHandle {
    Tokio(tokio::task::JoinHandle<Result<(), TaskError>>),
    OSThread(std::thread::JoinHandle<Result<(), TaskError>>),
}

impl JoinHandle {
    pub async fn try_join(self: JoinHandle) -> Result<Result<(), TaskError>, JoinHandleError> {
        match self {
            JoinHandle::Tokio(join_handle) => join_handle.await.map_err(JoinHandleError::TokioJoin),
            // Start a thread, join the handle, send the result through a tokio oneshot channel
            // The thread should be aborted if it takes to long to join
            JoinHandle::OSThread(join_handle) => {
                let (tx, rx) = tokio::sync::oneshot::channel();
                std::thread::spawn(move || {
                    let r = join_handle.join().map_err(JoinHandleError::OSThreadJoin);
                    tx.send(r).expect("Failed to send result");
                });
                rx.await?
            }
        }
    }
}
