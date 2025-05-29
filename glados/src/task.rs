use std::{fmt::Display, sync::Arc};

use thiserror::Error;
use tracing::{error, info};

use crate::{GladosHandle, ToGladosMsg, sync_to_async};

#[derive(Debug, Error)]
pub enum TaskError {
    #[error("Task finished with unexpected failure")]
    TaskFailed(Box<dyn std::any::Any + Send>),
    #[error("Failed to receive oneshot message")]
    OneShotRecvError(#[from] tokio::sync::oneshot::error::RecvError),
    #[error("Error sending message")]
    TrySendError(#[from] tokio::sync::mpsc::error::TrySendError<ToGladosMsg>),
}

#[derive(Debug, Error)]
pub enum JoinHandleError {
    #[error("Failed to join tokio task: {0}")]
    TokioJoinError(#[from] tokio::task::JoinError),
    #[error("Opaque OS thread join error")]
    OSThreadJoinError(Box<dyn std::any::Any + Send>),
}

#[derive(Debug)]
pub struct TaskHandle {
    name: Arc<str>,
    pub(crate) id: uuid::Uuid,
    kind: TaskKind,
    /// Not unique, an id of a previous task might be reused (at least with Tokio)
    platform_id: _TaskPlatformId,
    pub(crate) join_handle: JoinHandle,
    pub(crate) cancellation_token: tokio_util::sync::CancellationToken,
}

impl TaskHandle {
    pub fn new(name: &str, join_handle: JoinHandle) -> Self {
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
            cancellation_token: tokio_util::sync::CancellationToken::new(),
        }
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
            JoinHandle::Tokio(join_handle) => {
                join_handle.await.map_err(JoinHandleError::TokioJoinError)
            }
            JoinHandle::OSThread(join_handle) => {
                let tokio = tokio::runtime::Handle::current();
                sync_to_async::sync_to_async!(tokio, async { join_handle.join() })
                    .map_err(|e| JoinHandleError::OSThreadJoinError(Box::new(e)))
            }
        }
    }
}

trait TaskCtx {
    fn glados(&self) -> &GladosHandle;
    fn glados_cloned(&self) -> GladosHandle {
        self.glados().clone()
    }
}

pub struct AsyncTaskCtx<E> {
    pub glados_handle: GladosHandle,
    pub extra_ctx: E,
}

impl<E> TaskCtx for AsyncTaskCtx<E> {
    fn glados(&self) -> &GladosHandle {
        &self.glados_handle
    }
}
