use futures::FutureExt;
use task::{JoinHandle, TaskHandle};
use thiserror::Error;
use tokio::sync::{
    mpsc::{Receiver, Sender},
    oneshot::Sender as OneShotSender,
};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, warn};
use uuid::Uuid;

mod sync_to_async;
mod task;
pub mod task_builder;
pub use task::TaskError;

#[derive(Error, Debug)]
pub enum GladosInternalError {
    #[error("Task not found")]
    TaskNotFound,
    #[error("Task already exists")]
    AlreadyExists,
    #[error(transparent)]
    SendError(#[from] tokio::sync::mpsc::error::SendError<ToGladosMsg>),
    #[error("Failed to send a oneshot message: {0}")]
    OneShotSendError(String),
    #[error("Failed to join task: {0}")]
    JoinError(String),
    #[error("Glados channel closed unexpectedly")]
    ChannelClosed,
}

#[derive(Error, Debug)]
pub enum GladosError {
    #[error("Error sending message to Glados")]
    SendError(#[from] tokio::sync::mpsc::error::SendError<ToGladosMsg>),
}

pub struct Glados {
    active_tasks: Vec<task::TaskHandle>,
    channel_capacity: usize,
}

#[derive(Debug, Clone)]
pub struct GladosHandle {
    to_glados: Sender<ToGladosMsg>,
}

impl GladosHandle {
    pub fn new(to_glados: Sender<ToGladosMsg>) -> Self {
        Self { to_glados }
    }

    pub async fn graceful_shutdown(&self) -> Result<(), GladosError> {
        self.to_glados.send(ToGladosMsg::GracefulShutdown).await?;
        Ok(())
    }

    pub async fn forced_shutdown(&self) -> Result<(), GladosError> {
        self.to_glados.send(ToGladosMsg::ForcedShutdown).await?;
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum SpawnTaskError {
    #[error("Error sending message to Glados")]
    TrySendError(#[from] tokio::sync::mpsc::error::TrySendError<ToGladosMsg>),
}

impl GladosHandle {
    pub fn spawn_async<F, Fut, Ctx, E, CF, E2, Fut2>(
        &self,
        name: &str,
        ctx: Ctx,
        f: F,
        f_cancel: CF,
    ) -> Result<(), SpawnTaskError>
    where
        F: 'static + Send + FnOnce(Ctx) -> Fut,
        Fut: Send + std::future::Future<Output = Result<(), E>>,
        Ctx: 'static + Send,
        E: Send + 'static,
        CF: 'static + Send + FnOnce() -> Fut2,
        Fut2: Send + std::future::Future<Output = Result<(), E2>>,
        E2: Send + 'static,
    {
        debug!("Spawning async task: {}", name);

        let cancellation_token = CancellationToken::new();
        let (notify_ready_to_start, notify_ready_to_start_receiver) =
            tokio::sync::oneshot::channel();
        // Spawn the task - which starts executing immediately, but will be paused until Glados
        // notifies it to start
        let join_handle = tokio::task::spawn({
            let cancellation_token = cancellation_token.clone();
            let to_glados = self.to_glados.clone();
            async move {
                let uuid = notify_ready_to_start_receiver.await?;
                let uuid = match uuid {
                    Ok(uuid) => uuid,
                    Err(e) => match e {
                        // TODO: probably remove the task
                        AddTaskError::ShutdownInProgress => todo!(),
                    },
                };

                let f = f(ctx);
                let f_cancel = cancellation_token.cancelled().then(|_| async move {
                    f_cancel().await
                    //to_glados.try_send(ToGladosMsg::RemoveTask(uuid)).unwrap();
                });

                tokio::select! {
                    r = f => {
                    //to_glados.try_send(ToGladosMsg::RemoveTask(uuid)).unwrap();
                        r.map_err(|e| TaskError::TaskFailed(Box::new(e)))
                    },
                    r = f_cancel => {
                        r.map_err(|e| TaskError::CancellationError(Box::new(e)))
                    },
                }?;
                to_glados.try_send(ToGladosMsg::RemoveTask(uuid))?;
                Ok(())
            }
        });

        let task_handle = TaskHandle::new(name, JoinHandle::Tokio(join_handle), cancellation_token);

        self.to_glados.try_send(ToGladosMsg::AddTask {
            task_handle,
            responder: notify_ready_to_start,
        })?;

        Ok(())
    }
    pub fn spawn_thread<F, Ctx, E>(&self, name: &str, ctx: Ctx, f: F) -> Result<(), SpawnTaskError>
    where
        F: 'static + FnOnce((Ctx, CancellationToken)) -> Result<(), E> + Send,
        Ctx: 'static + Send,
        E: 'static + Send,
    {
        debug!("Spawning async task: {}", name);

        let cancellation_token = CancellationToken::new();
        let (notify_ready_to_start, notify_ready_to_start_receiver) =
            tokio::sync::oneshot::channel();
        // Spawn the task - which starts executing immediately, but will be paused until Glados
        // notifies it to start
        let join_handle = std::thread::spawn({
            let cancellation_token = cancellation_token.clone();
            let to_glados = self.to_glados.clone();
            move || {
                let uuid = notify_ready_to_start_receiver.blocking_recv()?;
                let uuid = match uuid {
                    Ok(uuid) => uuid,
                    Err(e) => match e {
                        // TODO: probably remove the task
                        AddTaskError::ShutdownInProgress => todo!(),
                    },
                };

                f((ctx, cancellation_token)).map_err(|e| TaskError::TaskFailed(Box::new(e)))?;

                to_glados.try_send(ToGladosMsg::RemoveTask(uuid))?;
                Ok(())
            }
        });

        let task_handle =
            TaskHandle::new(name, JoinHandle::OSThread(join_handle), cancellation_token);

        self.to_glados.try_send(ToGladosMsg::AddTask {
            task_handle,
            responder: notify_ready_to_start,
        })?;

        Ok(())
    }
}

impl Glados {
    pub fn new(channel_capacity: usize) -> Self {
        Self {
            active_tasks: Vec::new(),
            channel_capacity,
        }
    }

    pub async fn start(
        self,
    ) -> (
        tokio::task::JoinHandle<Result<(), GladosInternalError>>,
        GladosHandle,
    ) {
        let (to_glados, from_glados_handle) = tokio::sync::mpsc::channel(self.channel_capacity);
        let glados_task_handle =
            tokio::task::spawn(async move { glados_loop(self, from_glados_handle).await });
        (glados_task_handle, GladosHandle { to_glados })
    }
}

async fn glados_loop(
    mut glados: Glados,
    mut from_handle: Receiver<ToGladosMsg>,
) -> Result<(), GladosInternalError> {
    let mut graceful_shutdown = false;
    loop {
        if graceful_shutdown {
            if !glados.active_tasks.is_empty() {
                debug!("Waiting for {} tasks to finish", glados.active_tasks.len());
            } else {
                debug!("All tasks finished, terminating");
                break;
            }
        }
        debug!("Glados waiting for messages");
        match from_handle.recv().await {
            Some(ToGladosMsg::AddTask {
                task_handle,
                responder,
            }) => {
                // During shutdown, deny spawning new tasks
                if graceful_shutdown {
                    warn!("Ignoring task addition during graceful shutdown");
                    responder
                        .send(Err(AddTaskError::ShutdownInProgress))
                        .map_err(|value| {
                            GladosInternalError::OneShotSendError(format!("{value:?}"))
                        })?;

                    continue;
                }
                // Normal behavior - add task
                debug!("Spawning task: {}", task_handle);

                let uuid = task_handle.id;
                glados.active_tasks.push(task_handle);
                responder.send(Ok(uuid)).map_err(|value| {
                    GladosInternalError::OneShotSendError(format!(
                        "Failed to send response: {value:?}"
                    ))
                })?;
            }
            Some(ToGladosMsg::RemoveTask(task)) => {
                debug!("Removing task: {}", task);
                // Find and remove the task in one step
                let task_position = glados
                    .active_tasks
                    .iter()
                    .position(|t| t.id == task)
                    .ok_or(GladosInternalError::TaskNotFound)?;

                // Remove the task, preserving ownership
                let task = glados.active_tasks.swap_remove(task_position);

                // Here we first get a result indicating whether the join was successful or not,
                // then we get the actual result of the task.
                let r = task
                    .join_handle
                    .try_join()
                    .await
                    .map_err(|e| GladosInternalError::JoinError(format!("{e}")));

                match r {
                    Ok(v) => debug!("Task {} finished successfully: {:?}", task.id, v),
                    Err(e) => error!("Task {} finished with error: {:?}", task.id, e),
                }
            }
            Some(ToGladosMsg::GracefulShutdown) => {
                debug!("Graceful shutdown started");
                for task in glados.active_tasks.iter_mut() {
                    debug!("Cancelling task: {}", task);
                    task.cancel();
                }
                graceful_shutdown = true;
            }
            Some(ToGladosMsg::ForcedShutdown) => {
                debug!("Forced shutdown");
                for task in glados.active_tasks.iter_mut() {
                    debug!("Aborting task: {}", task);
                    match &mut task.join_handle {
                        JoinHandle::Tokio(handle) => handle.abort(),
                        JoinHandle::OSThread(handle) => handle.thread().unpark(), // ????
                    }
                }
                break;
            }
            None => {
                error!("Channel closed");
                return Err(GladosInternalError::ChannelClosed);
            }
        }
    }
    debug!("Glados finished");
    Ok(())
}

#[derive(Error, Debug)]
pub enum AddTaskError {
    #[error("Shuwdown in progress, adding new tasks is not allowed")]
    ShutdownInProgress,
}

#[derive(Debug)]
pub enum ToGladosMsg {
    AddTask {
        task_handle: TaskHandle,
        responder: OneShotSender<Result<Uuid, AddTaskError>>,
    },
    RemoveTask(Uuid),
    GracefulShutdown,
    ForcedShutdown,
}
