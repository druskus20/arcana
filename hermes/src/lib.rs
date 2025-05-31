use itertools::Itertools;
use message::DynMessage;
use message::MessageMeta;
use message::TypeErasedMessage;
use spells::hashmap_ext::HashmapExt;
use std::collections::HashMap;
use subscriber::Criteria;
use subscriber::SubscriberRef;
use subscriber::Subscription;
use thiserror::Error;
use tokio::sync;
use tokio::sync::mpsc::Receiver;
use tokio::sync::mpsc::Sender;
use tokio::sync::oneshot::Sender as OneshotSender;
use tracing::debug;
use tracing::warn;
use uuid::Uuid;

mod message;
pub mod subscriber;

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
    // A subscriber is subscribed to a single message type
    subscribers_by_id: HashMap<Uuid, SubscriberRef>,
    // Many subscribers can subscribe to the same message type
    subscriber_id_by_message_meta: HashMap<MessageMeta, Vec<Uuid>>,
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

                // TODO: clear this. But basically, if the message type is already subscribed to,
                // then we push a new subscriber ID to the existing list.
                // If the message type is not subscribed to, we create a new entry in the map.
                if hermes
                    .subscriber_id_by_message_meta
                    .contains_key(&message_meta)
                {
                    let subscriber_ids = hermes
                        .subscriber_id_by_message_meta
                        .get_mut(&message_meta)
                        .unwrap();
                    if subscriber_ids.contains(&subscriber_ref.subscriber_id) {
                        warn!(
                            "Subscriber {} is already subscribed to message type: {:?}",
                            name, message_meta,
                        );
                        responder.send(SubscribeToResponse::AlreadySubscribed).ok();
                        continue;
                    } else {
                        subscriber_ids.push(subscriber_ref.subscriber_id);
                    }
                } else {
                    hermes
                        .subscriber_id_by_message_meta
                        .insert(message_meta, vec![subscriber_ref.subscriber_id]);
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
                _subscriber_id,
                _message_meta,
            } => todo!(),
            ToHermesMsg::Terminate => break,
        }
    }

    debug!("Hermes finished");
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
        .filter_map(move |(message_meta, subscriber_ids)| {
            if message_meta.type_id() == type_id {
                Some(subscriber_ids)
            } else {
                None
            }
        })
        .flatten()
        .unique()
}

#[derive(Debug, Clone)]
pub struct HermesHandle {
    to_hermes: Sender<ToHermesMsg>,
}

impl HermesHandle {
    pub async fn subscribe_to<T: DynMessage>(
        &self,
        actor_name: &str,
    ) -> Result<Subscription<T>, SubscribeToError> {
        let (responder, receiver) = sync::oneshot::channel();
        self.to_hermes
            .send(ToHermesMsg::SubscribeTo {
                name: actor_name.to_owned(),
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
    pub async fn deliver<T: DynMessage>(&self, msg: T) -> Result<(), HermesError> {
        let type_erased_msg = TypeErasedMessage::from(msg);
        self.to_hermes
            .send(ToHermesMsg::Deliver(type_erased_msg))
            .await?;
        Ok(())
    }

    pub async fn deliver_with_criteria<T: DynMessage>(
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
    pub async fn unsubscribe<T>(&self, _s: Subscription<T>) -> Result<(), HermesError> {
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
        _subscriber_id: Uuid,
        _message_meta: MessageMeta,
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::subscriber::SubscriberIdCriteria;
    use crate::subscriber::SubscriberNameCriteria;
    use tokio::sync::mpsc;

    #[derive(Debug, Clone)]
    enum ClonableTestMessage {
        Hello(String),
        Goodbye(String),
    }

    #[tokio::test]
    async fn test_hermes_subscribe_and_deliver_clonable() {
        let hermes = Hermes::new(10);
        let (handle, hermes_handle) = hermes.start().await;

        let mut subscription = hermes_handle
            .subscribe_to::<ClonableTestMessage>("test_actor")
            .await
            .expect("Failed to subscribe");

        hermes_handle
            .deliver(ClonableTestMessage::Hello("World".to_string()))
            .await
            .expect("Failed to deliver message");

        if let Some(msg) = subscription.recv_owned().await {
            match msg {
                ClonableTestMessage::Hello(name) => assert_eq!(name, "World"),
                _ => panic!("Unexpected message type received"),
            }
        } else {
            panic!("No message received");
        }

        hermes_handle
            .terminate()
            .await
            .expect("Failed to terminate Hermes");
        handle
            .await
            .expect("Hermes task failed")
            .expect("Error joining Hermes task");
    }

    #[derive(Debug, Clone)]
    enum NonClonableTestMessage {
        Hello(String),
        Goodbye(String),
    }

    #[tokio::test]
    async fn test_hermes_subscribe_and_deliver_non_clonable() {
        let hermes = Hermes::new(10);
        let (handle, hermes_handle) = hermes.start().await;

        let mut subscription = hermes_handle
            .subscribe_to::<NonClonableTestMessage>("test_actor")
            .await
            .expect("Failed to subscribe");

        hermes_handle
            .deliver(NonClonableTestMessage::Hello("World".to_string()))
            .await
            .expect("Failed to deliver message");

        if let Some(msg) = subscription.recv_ref().await {
            match msg {
                NonClonableTestMessage::Hello(name) => assert_eq!(name, "World"),
                _ => panic!("Unexpected message type received"),
            }
        } else {
            panic!("No message received");
        }

        hermes_handle
            .terminate()
            .await
            .expect("Failed to terminate Hermes");
        handle
            .await
            .expect("Hermes task failed")
            .expect("Error joining Hermes task");
    }
}
