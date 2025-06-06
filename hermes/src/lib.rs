use itertools::Itertools;
use message::DynMessage;
use message::MessageMeta;
use message::TypeErasedMessage;
use spells::hashmap_ext::HashMapExtError;
use spells::hashmap_ext::HashmapExt;
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use subscriber::Criteria;
use subscriber::ExclusiveSubscription;
use subscriber::SubscriberRef;
use subscriber::Subscription;
use thiserror::Error;
use tokio::sync;
use tokio::sync::mpsc::Receiver;
use tokio::sync::mpsc::Sender;
use tokio::sync::oneshot::Sender as OneshotSender;
use tracing::debug;
use tracing::error;
use tracing::trace;
use tracing::warn;
use uuid::Uuid;

mod message;
pub mod subscriber;

#[derive(Error, Debug)]
pub enum HermesInternalError {
    #[error("Failed to send opaque message through oneshot channel")]
    OneShotSendError,
    #[error("Failed to send message through mpsc channel {0}")]
    SendError(String),
    #[error("Subscriber not found")]
    SubscriberNotFound,
}

#[derive(Debug)]
pub struct Hermes {
    channel_capacity: usize,
    // Subscribers to messages that can only have one recipient
    single_subscriber_by_message_meta: HashMap<MessageMeta, SubscriberRef>,
    // A subscriber is subscribed to a single message type
    multi_subscribers_by_id: HashMap<Uuid, SubscriberRef>,
    // Many subscribers can subscribe to the same message type
    multi_subscriber_id_by_message_meta: HashMap<MessageMeta, Vec<Uuid>>,
}

impl Hermes {
    pub fn new(channel_capacity: usize) -> Self {
        Hermes {
            channel_capacity,
            multi_subscribers_by_id: HashMap::new(),
            multi_subscriber_id_by_message_meta: HashMap::new(),
            single_subscriber_by_message_meta: HashMap::new(),
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
        trace!("{:?}", hermes);
        // Process the message here
        match msg {
            ToHermesMsg::SubscribeTo {
                name,
                message_meta,
                responder,
                exclusive: false,
            } => {
                // Create a reference to the subscriber and store it. Respond with a channel to
                // receive Hermes messages.
                let (sender, receiver) = sync::mpsc::channel(hermes.channel_capacity);
                let subscriber_ref = SubscriberRef {
                    subscriber_id: Uuid::new_v4(),
                    subscriber_name: name.clone(),
                    sender,
                };

                match hermes
                    .multi_subscriber_id_by_message_meta
                    .entry(message_meta.clone())
                {
                    Entry::Occupied(mut entry) => {
                        let subscriber_ids = entry.get_mut();
                        if subscriber_ids.contains(&subscriber_ref.subscriber_id) {
                            warn!(
                                "Subscriber {} is already subscribed to message type: {:?}",
                                name, message_meta,
                            );
                            let r = responder.send(Err(SubscribeToError::AlreadySubscribed));
                            if r.is_err() {
                                return Err(HermesInternalError::OneShotSendError);
                            }
                            continue;
                        } else {
                            subscriber_ids.push(subscriber_ref.subscriber_id);
                        }
                    }
                    Entry::Vacant(entry) => {
                        entry.insert(vec![subscriber_ref.subscriber_id]);
                    }
                }

                // Finally insert the subscriber reference into the multi-subscriber map.
                let res = hermes
                    .multi_subscribers_by_id
                    .fallible_insert(subscriber_ref.subscriber_id, subscriber_ref);

                match res {
                    Ok(_) => {
                        responder
                            .send(Ok(receiver))
                            .map_err(|_| HermesInternalError::OneShotSendError)?;
                    }
                    Err(HashMapExtError::KeyAlreadyExists) => {
                        responder
                            .send(Err(SubscribeToError::AlreadySubscribed))
                            .map_err(|_| HermesInternalError::OneShotSendError)?;
                        continue;
                    }
                }
            }
            ToHermesMsg::SubscribeTo {
                name,
                message_meta,
                responder,
                exclusive: true,
            } => {
                // First, check if an exclusive subscriber already exists for this message type.
                if hermes
                    .single_subscriber_by_message_meta
                    .contains_key(&message_meta)
                {
                    responder
                        .send(Err(SubscribeToError::ExclusiveSubscriberAlreadyExists))
                        .map_err(|_| HermesInternalError::OneShotSendError)?;
                    continue;
                }

                // Then check if a subscriber exists for this messag  type in the multi-subscriber map.
                if hermes
                    .multi_subscriber_id_by_message_meta
                    .contains_key(&message_meta)
                {
                    responder
                        .send(Err(SubscribeToError::SubscriberFound))
                        .map_err(|_| HermesInternalError::OneShotSendError)?;
                    continue;
                }
                // Create a reference to the subscriber and store it. Respond with a channel to
                // receive Hermes messages.
                let (sender, receiver) = sync::mpsc::channel(hermes.channel_capacity);
                let subscriber_ref = SubscriberRef {
                    subscriber_id: Uuid::new_v4(),
                    subscriber_name: name.clone(),
                    sender,
                };
                hermes
                    .single_subscriber_by_message_meta
                    .insert(message_meta, subscriber_ref);

                responder
                    .send(Ok(receiver))
                    .map_err(|_| HermesInternalError::OneShotSendError)?;
            }
            // This method overloads two types of deliveries:
            //     - Single (exclusive) subscriber delivery
            //     - Multi subscriber delivery (can have an optional criteria)
            ToHermesMsg::Deliver { msg, criteria } => {
                if let Some(exclusive) = try_find_exclusive_subscriber_for_meta(&hermes, &msg.meta)
                {
                    exclusive
                        .sender
                        .send(msg.clone())
                        .await
                        .map_err(|e| HermesInternalError::SendError(format!("{e:?}")))?;
                } else {
                    let subscriber_ids_matching_type =
                        filter_subscribers_by_matching_type(&hermes, &msg);
                    let subscriber_ids_matching_criteria: Vec<&Uuid> =
                        if let Some(criteria) = criteria {
                            subscriber_ids_matching_type
                                .into_iter()
                                .filter(|subscriber_id| {
                                    hermes
                                        .multi_subscribers_by_id
                                        .get(subscriber_id)
                                        .is_some_and(|subscriber| criteria.matches(subscriber))
                                })
                                .collect()
                        } else {
                            subscriber_ids_matching_type.collect()
                        };
                    if !subscriber_ids_matching_criteria.is_empty() {
                        send_msg_to_subscribers(&hermes, msg, subscriber_ids_matching_criteria)
                            .await?;
                    } else {
                        warn!(
                            "No subscribers matching criteria for message: {}",
                            msg.type_name()
                        );
                    }
                }
            }
            ToHermesMsg::Unsubscribe {
                _subscriber_id,
                _message_meta,
            } => todo!(),
            ToHermesMsg::Terminate => break,
            //ToHermesMsg::SubscribeTo {
            //    name,
            //    message_meta,
            //    responder,
            //    exclusive,
            //} => {
            //    // Create a reference to the exclusive subscriber and store it.
            //    let (sender, receiver) = sync::mpsc::channel(hermes.channel_capacity);
            //    let subscriber_ref = SubscriberRef {
            //        subscriber_id: Uuid::new_v4(),
            //        subscriber_name: name,
            //        sender,
            //    };

            //    if hermes
            //        .single_subscriber_by_message_meta
            //        .contains_key(&message_meta)
            //    {
            //        warn!(
            //            "Exclusive subscriber already exists for message type: {:?}",
            //            message_meta
            //        );
            //        let r = responder.send(Err(SubscribeToError::ExclusiveSubscriberAlreadyExists));
            //        if r.is_err() {
            //            return Err(HermesInternalError::OneShotSendError);
            //        }
            //        continue;
            //    }

            //    hermes
            //        .single_subscriber_by_message_meta
            //        .insert(message_meta, subscriber_ref);

            //    responder
            //        .send(Ok(receiver))
            //        .map_err(|_| HermesInternalError::OneShotSendError)?;
            //}
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
            .multi_subscribers_by_id
            .get(subscriber_id)
            .ok_or(HermesInternalError::SubscriberNotFound)?
            .sender
            .send(type_erased_msg.clone())
            .await
            .map_err(|e| HermesInternalError::SendError(format!("{e:?}")))?;
    }
    Ok(())
}

//async fn send_msg_to_exclusive_subscriber(
//    hermes: &Hermes,
//    type_erased_msg: ExclusiveTypeErasedMessage,
//) -> Result<(), HermesInternalError> {
//    hermes
//        .single_subscriber_by_message_meta
//        .get(&type_erased_msg.meta)
//        .ok_or(HermesInternalError::SubscriberNotFound)?
//        .sender
//        .send(type_erased_msg)
//        .await
//        .map_err(|e| HermesInternalError::SendError(format!("{e:?}")))?;
//
//    Ok(())
//}

//fn filter_subscribers_by_matching_type<'a>(
//    hermes: &'a Hermes,
//    type_erased_msg: &TypeErasedMessage,
//) -> impl Iterator<Item = &'a Uuid> {
//    let type_id = type_erased_msg.type_id();
//    hermes
//        .multi_subscriber_id_by_message_meta
//        .iter()
//        .filter_map(move |(message_meta, subscriber_ids)| {
//            if message_meta.type_id() == type_id {
//                Some(subscriber_ids)
//            } else {
//                None
//            }
//        })
//        .flatten()
//        .unique()
//}

fn try_find_exclusive_subscriber_for_meta<'a>(
    hermes: &'a Hermes,
    meta: &MessageMeta,
) -> Option<&'a SubscriberRef> {
    hermes.single_subscriber_by_message_meta.get(meta)
}

fn filter_subscribers_by_matching_type<'a>(
    hermes: &'a Hermes,
    type_erased_msg: &TypeErasedMessage,
) -> impl Iterator<Item = &'a Uuid> {
    let type_id = type_erased_msg.type_id();

    hermes
        .multi_subscriber_id_by_message_meta
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
                exclusive: false,
            })
            .await
            .map_err(|e| SubscribeToError::SendError { source: e })?;

        let generic_receiver = receiver.await??;

        Ok(Subscription::<T>::from_receiver(
            Uuid::new_v4(),
            generic_receiver,
        ))
    }

    pub async fn exclusive_subscribe_to<T: DynMessage>(
        &self,
        actor_name: &str,
    ) -> Result<ExclusiveSubscription<T>, SubscribeToError> {
        let subscriber_id = Uuid::new_v4();
        let (responder, receiver) = sync::oneshot::channel();
        self.to_hermes
            .send(ToHermesMsg::SubscribeTo {
                name: actor_name.to_owned(),
                message_meta: MessageMeta::of::<T>(),
                responder,
                exclusive: true,
            })
            .await
            .map_err(|e| SubscribeToError::SendError { source: e })?;

        let exclusive_receiver = receiver.await??;

        Ok(ExclusiveSubscription::from_receiver(
            subscriber_id,
            exclusive_receiver,
        ))
    }

    pub async fn deliver<T: DynMessage>(&self, msg: T) -> Result<(), HermesError> {
        let type_erased_msg = TypeErasedMessage::from(msg);
        self.to_hermes
            .send(ToHermesMsg::Deliver {
                msg: type_erased_msg,
                criteria: None,
            })
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
            .send(ToHermesMsg::Deliver {
                msg: type_erased_msg,
                criteria: Some(Box::new(criteria)),
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
        responder: OneshotSender<Result<Receiver<TypeErasedMessage>, SubscribeToError>>,
        exclusive: bool,
    },
    Deliver {
        msg: TypeErasedMessage,
        criteria: Option<Box<dyn subscriber::Criteria>>,
    },
    Unsubscribe {
        _subscriber_id: Uuid,
        _message_meta: MessageMeta,
    },
    Terminate,
}

#[derive(Debug, Error)]
pub enum SubscribeToError {
    #[error("Already subscribed to this message type")]
    AlreadySubscribed,
    #[error("Message type can only have one subscriber")]
    ExclusiveSubscriberAlreadyExists,
    #[error(
        "A regular subscriber already exists for this message type, cannot create an exclusive subscriber"
    )]
    SubscriberFound,
    #[error("Failed to receive response from Hermes")]
    FailedToReceiveResponse,
    #[error("Failed to send message through Hermes channel")]
    SendError {
        #[source]
        source: tokio::sync::mpsc::error::SendError<ToHermesMsg>,
    },
    #[error("Failed to receive response from Hermes")]
    RecvOneshotError(#[from] tokio::sync::oneshot::error::RecvError),
}

#[derive(Debug, Error)]
pub enum HermesError {
    #[error("Failed to send message through Hermes channel")]
    SendError(#[from] tokio::sync::mpsc::error::SendError<ToHermesMsg>),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone)]
    enum ClonableTestMessage {
        Hello(String),
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

        if let Some(msg) = subscription.recv_cloned().await {
            match msg {
                ClonableTestMessage::Hello(name) => assert_eq!(name, "World"),
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

    #[derive(Debug)]
    enum NonClonableTestMessage {
        Hello(String),
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

    #[tokio::test]
    async fn test_hermes_exclusive_subscribe_and_deliver() {
        let hermes = Hermes::new(10);
        let (handle, hermes_handle) = hermes.start().await;

        let mut subscription = hermes_handle
            .exclusive_subscribe_to::<NonClonableTestMessage>("test_actor")
            .await
            .expect("Failed to subscribe");

        hermes_handle
            .deliver(NonClonableTestMessage::Hello("World".to_string()))
            .await
            .expect("Failed to deliver message");

        if let Some(msg) = subscription.recv().await {
            match msg {
                NonClonableTestMessage::Hello(name) => assert_eq!(name, "World"),
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
