use message::DynClonableMessage;
use message::MessageMeta;
use message::TypeErasedMessage;
use spells::HashmapExt;
use std::collections::HashMap;
use subscriber::Criteria;
use subscriber::SubscriberRef;
use subscriber::Subscription;
use thiserror::Error;
use tokio::sync;
use tokio::sync::mpsc::Receiver;
use tokio::sync::mpsc::Sender;
use tokio::sync::oneshot::Sender as OneshotSender;
use tracing::info;
use tracing::warn;
use uuid::Uuid;

mod message;
mod subscriber;

#[derive(Error, Debug)]
pub enum HermesInternalError {
    #[error("Failed to send opaque message through oneshot channel")]
    OneShotSendError,
    #[error("Failed to send message through mpsc channel")]
    SendError {
        #[source]
        source: tokio::sync::mpsc::error::SendError<TypeErasedMessage>,
    },
    #[error("Subscriber not found")]
    SubscriberNotFound,
}

pub struct Hermes {
    channel_capacity: usize,
    subscribers_by_id: HashMap<Uuid, SubscriberRef>,
    subscriber_id_by_message_meta: HashMap<MessageMeta, Uuid>,
}

impl Hermes {
    pub fn new(channel_capacity: usize) -> Self {
        Hermes {
            channel_capacity,
            subscribers_by_id: HashMap::new(),
            subscriber_id_by_message_meta: HashMap::new(),
        }
    }

    pub async fn start(
        self,
    ) -> (
        tokio::task::JoinHandle<Result<(), HermesInternalError>>,
        HermesHandle,
    ) {
        let (to_hermes, from_hermes_handle) = sync::mpsc::channel(self.channel_capacity);
        let hermes_task_handle =
            tokio::task::spawn(async move { hermes_loop(self, from_hermes_handle).await });
        (hermes_task_handle, HermesHandle { to_hermes })
    }
}

async fn hermes_loop(
    mut hermes: Hermes,
    mut from_handle: sync::mpsc::Receiver<ToHermesMsg>,
) -> Result<(), HermesInternalError> {
    while let Some(msg) = from_handle.recv().await {
        // Process the message here
        match msg {
            ToHermesMsg::SubscribeTo {
                name,
                message_meta,
                responder,
            } => {
                // Create a reference to the subscriber and store it. Respond with a channel to
                // receive Hermes messages.
                let (sender, receiver) = sync::mpsc::channel(hermes.channel_capacity);
                let subscriber_ref = SubscriberRef {
                    subscriber_id: Uuid::new_v4(),
                    subscriber_name: name.clone(),
                    sender,
                };
                let res = hermes
                    .subscriber_id_by_message_meta
                    .fallible_insert(message_meta, subscriber_ref.subscriber_id);
                if let Err(e) = res {
                    warn!("Failed to insert subscriber: {e}");
                    responder.send(SubscribeToResponse::AlreadySubscribed).ok();
                    continue;
                }

                let res = hermes
                    .subscribers_by_id
                    .fallible_insert(subscriber_ref.subscriber_id, subscriber_ref);
                if let Err(e) = res {
                    warn!("Failed to insert subscriber: {e}");
                    responder.send(SubscribeToResponse::AlreadySubscribed).ok();
                    continue;
                }

                responder
                    .send(SubscribeToResponse::Ok(receiver))
                    .map_err(|_| HermesInternalError::OneShotSendError)?;
            }
            ToHermesMsg::Deliver(msg) => {
                let subscriber_ids_matching_type =
                    filter_subscribers_by_matching_type(&hermes, &msg).collect::<Vec<_>>();

                if subscriber_ids_matching_type.is_empty() {
                    warn!(
                        "No subscribers matching type for message: {}",
                        msg.type_name()
                    );
                } else {
                    send_msg_to_subscribers(&hermes, msg, subscriber_ids_matching_type).await?;
                }
            }
            ToHermesMsg::DeliverWithCriteria { msg, criteria } => {
                let subscriber_ids_matching_type =
                    filter_subscribers_by_matching_type(&hermes, &msg);

                let subscriber_ids_matching_criteria = subscriber_ids_matching_type
                    .filter(|&subscriber_id| {
                        hermes
                            .subscribers_by_id
                            .get(subscriber_id)
                            .is_some_and(|subscriber| criteria.matches(subscriber))
                    })
                    .collect::<Vec<_>>();

                if subscriber_ids_matching_criteria.is_empty() {
                    warn!(
                        "No subscribers matching criteria for message: {}",
                        msg.type_name()
                    );
                } else {
                    send_msg_to_subscribers(&hermes, msg, subscriber_ids_matching_criteria).await?;
                }
            }
            ToHermesMsg::Unsubscribe {
                subscriber_id,
                message_meta,
            } => todo!(),
            ToHermesMsg::Terminate => break,
        }
    }

    info!("Hermes finished");
    Ok(())
}

async fn send_msg_to_subscribers(
    hermes: &Hermes,
    type_erased_msg: TypeErasedMessage,
    subscriber_ids_matching_type: Vec<&Uuid>,
) -> Result<(), HermesInternalError> {
    for subscriber_id in subscriber_ids_matching_type {
        hermes
            .subscribers_by_id
            .get(subscriber_id)
            .ok_or(HermesInternalError::SubscriberNotFound)?
            .sender
            .send(type_erased_msg.clone())
            .await
            .map_err(|e| HermesInternalError::SendError { source: e })?;
    }
    Ok(())
}

fn filter_subscribers_by_matching_type<'a>(
    hermes: &'a Hermes,
    type_erased_msg: &TypeErasedMessage,
) -> impl Iterator<Item = &'a Uuid> {
    let type_id = type_erased_msg.type_id();
    hermes
        .subscriber_id_by_message_meta
        .iter()
        .filter_map(move |(message_meta, subscriber_id)| {
            if message_meta.type_id() == type_id {
                Some(subscriber_id)
            } else {
                None
            }
        })
}

#[derive(Debug, Clone)]
pub struct HermesHandle {
    to_hermes: Sender<ToHermesMsg>,
}

impl HermesHandle {
    pub async fn subscribe_to<T: DynClonableMessage>(
        &self,
        actor_name: String,
    ) -> Result<Subscription<T>, SubscribeToError> {
        let (responder, receiver) = sync::oneshot::channel();
        self.to_hermes
            .send(ToHermesMsg::SubscribeTo {
                name: actor_name.clone(),
                message_meta: MessageMeta::of::<T>(),
                responder,
            })
            .await
            .map_err(|e| SubscribeToError::SendError { source: e })?;

        match receiver.await {
            Ok(SubscribeToResponse::Ok(generic_receiver)) => Ok(Subscription::<T>::from_receiver(
                Uuid::new_v4(),
                generic_receiver,
            )),
            Ok(SubscribeToResponse::AlreadySubscribed) => Err(SubscribeToError::AlreadySubscribed),
            Err(_) => Err(SubscribeToError::FailedToReceiveResponse),
        }
    }
    pub async fn deliver<T: DynClonableMessage>(&self, msg: T) -> Result<(), HermesError> {
        let type_erased_msg = TypeErasedMessage::from(msg);
        self.to_hermes
            .send(ToHermesMsg::Deliver(type_erased_msg))
            .await?;
        Ok(())
    }

    pub async fn deliver_with_criteria<T: DynClonableMessage>(
        &self,
        msg: T,
        criteria: impl Criteria + 'static,
    ) -> Result<(), HermesError> {
        let type_erased_msg = TypeErasedMessage::from(msg);
        self.to_hermes
            .send(ToHermesMsg::DeliverWithCriteria {
                msg: type_erased_msg,
                criteria: Box::new(criteria),
            })
            .await?;
        Ok(())
    }
    /// Gives away the subscription
    pub async fn unsubscribe<T>(&self, s: Subscription<T>) -> Result<(), HermesError> {
        todo!()
    }
    pub async fn terminate(&self) -> Result<(), HermesError> {
        self.to_hermes.send(ToHermesMsg::Terminate).await?;
        Ok(())
    }
}

pub enum ToHermesMsg {
    SubscribeTo {
        name: String,
        message_meta: MessageMeta,
        responder: OneshotSender<SubscribeToResponse>,
    },
    Deliver(TypeErasedMessage),
    DeliverWithCriteria {
        msg: TypeErasedMessage,
        criteria: Box<dyn subscriber::Criteria>,
    },
    Unsubscribe {
        subscriber_id: Uuid,
        message_meta: MessageMeta,
    },
    Terminate,
}

pub enum SubscribeToResponse {
    Ok(Receiver<TypeErasedMessage>),
    AlreadySubscribed,
}

#[derive(Debug, Error)]
pub enum SubscribeToError {
    #[error("Already subscribed to this message type")]
    AlreadySubscribed,
    #[error("Failed to receive response from Hermes")]
    FailedToReceiveResponse,
    #[error("Failed to send message through Hermes channel")]
    SendError {
        #[source]
        source: tokio::sync::mpsc::error::SendError<ToHermesMsg>,
    },
}

#[derive(Debug, Error)]
pub enum HermesError {
    #[error("Failed to send message through Hermes channel")]
    SendError(#[from] tokio::sync::mpsc::error::SendError<ToHermesMsg>),
}
