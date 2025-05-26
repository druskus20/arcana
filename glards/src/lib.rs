use task::{AsyncTaskCtx, JoinHandle, TaskError, TaskHandle};
use tokio::sync::{
    mpsc::{Receiver, Sender},
    oneshot::Sender as OneShotSender,
};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

mod sync_to_async;
mod task;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum GladosError {
    #[error("Unknown error")]
    Unknown,
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
    pub fn spawn_async<F, Fut, Ctx>(&self, name: &str, f: F) -> TaskHandle
    where
        F: 'static + Send + FnOnce(AsyncTaskCtx) -> Fut,
        Fut: Send + std::future::Future<Output = Result<(), TaskError>>,
    {
        debug!("Spawning async task: {}", name);

        let (notify_ready_to_start, notify_ready_to_start_receiver) =
            tokio::sync::oneshot::channel();
        // Spawn the task - which starts executing immediately, but will be paused until Glados
        // notifies it to start
        let join_handle = tokio::task::spawn({
            let ctx = AsyncTaskCtx {
                glados_handle: self.clone(),
            };
            let to_glados = self.to_glados.clone();
            async move {
                let uuid = notify_ready_to_start_receiver.await?;
                let uuid = match uuid {
                    Ok(uuid) => uuid,
                    Err(e) => match e {
                        AddTaskError::ShutdownInProgress => todo!(),
                    },
                };

                let r = f(ctx).await;
                to_glados
                    .try_send(ToGladosMsg::RemoveTask(uuid))
                    .expect("Failed to send task removal to Glados");
                r
            }
        });

        let task_handle = TaskHandle::new(name, JoinHandle::Tokio(join_handle));

        self.to_glados
            .try_send(ToGladosMsg::AddTask {
                task_handle,
                responder: notify_ready_to_start,
            })
            .expect("Failed to send task to Glados");

        todo!()
    }
}

impl Glados {
    pub fn new(channel_capacity: usize) -> Self {
        Self {
            active_tasks: Vec::new(),
            channel_capacity,
        }
    }

    pub async fn start(self) -> Result<(tokio::task::JoinHandle<()>, GladosHandle), GladosError> {
        let (to_glados, from_glados_handle) = tokio::sync::mpsc::channel(self.channel_capacity);
        let glados_task_handle =
            tokio::task::spawn(async move { glados_loop(self, from_glados_handle).await });
        Ok((glados_task_handle, GladosHandle { to_glados }))
    }
}

async fn glados_loop(mut glados: Glados, mut from_handle: Receiver<ToGladosMsg>) {
    let mut graceful_shutdown = false;
    loop {
        if graceful_shutdown {
            if !glados.active_tasks.is_empty() {
                info!("Waiting for {} tasks to finish", glados.active_tasks.len());
            } else {
                info!("All tasks finished, terminating");
                break;
            }
        }
        info!("Glados waiting for messages");
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
                        .expect("Failed to notify task");
                    continue;
                }
                // Normal behavior - add task
                info!("Spawning task: {}", task_handle);

                let uuid = task_handle.id;
                glados.active_tasks.push(task_handle);
                responder
                    .send(Ok(uuid))
                    .expect("Failed to notify task to start");
            }
            Some(ToGladosMsg::RemoveTask(task)) => {
                info!("Removing task: {}", task);
                // Find and remove the task in one step
                let task_position = glados
                    .active_tasks
                    .iter()
                    .position(|t| t.id == task)
                    .expect("Task not found");

                // Remove the task, preserving ownership
                let task = glados.active_tasks.swap_remove(task_position);

                // we need to wait on the handle so we Actually get the error trace /
                // result
                task.join_handle
                    .try_join()
                    .await
                    .expect("Failed to join task");
            }
            Some(ToGladosMsg::GracefulShutdown) => {
                info!("Graceful shutdown started");
                for task in glados.active_tasks.iter_mut() {
                    info!("Cancelling task: {}", task);
                    task.cancellation_token.cancel();
                }
                graceful_shutdown = true;
            }
            Some(ToGladosMsg::ForcedShutdown) => {
                info!("Forced shutdown");
                for task in glados.active_tasks.iter_mut() {
                    info!("Aborting task: {}", task);
                    match &mut task.join_handle {
                        JoinHandle::Tokio(handle) => handle.abort(),
                        JoinHandle::OSThread(handle) => handle.thread().unpark(), // ????
                    }
                }
                // finish immediately
                break;
            }
            None => {
                error!("Channel closed");
                break;
            }
        }
    }
    info!("Glados finished");
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
