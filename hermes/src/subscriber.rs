use std::{marker::PhantomData, sync::Arc};

use futures::{Stream, StreamExt};
use tokio::sync::mpsc::{Receiver, Sender};
use tokio_stream::wrappers::ReceiverStream;
use uuid::Uuid;

use crate::message::{DynMessage, TypeErasedMessage};

pub struct Subscription<T> {
    _subscriber_id: Uuid,
    message_type: PhantomData<T>,
    receiver: Receiver<TypeErasedMessage>,
    // This is a workaround to be able to return references to the last message received
    // (alternative, return the arc)
    last_msg: Option<Arc<T>>,
}

impl<T: DynMessage> Subscription<T> {
    pub fn from_receiver(_subscriber_id: Uuid, receiver: Receiver<TypeErasedMessage>) -> Self {
        Subscription {
            _subscriber_id,
            message_type: PhantomData,
            receiver,
            last_msg: None,
        }
    }

    pub async fn recv(&mut self) -> Option<Arc<T>> {
        let msg = self.receiver.recv().await;
        if msg.is_some() {
            self.last_msg = None;
        }
        msg.map(|msg| msg.try_cast_into().expect("Failed to cast message"))
    }

    pub async fn recv_ref(&mut self) -> Option<&'_ T> {
        let msg = self.receiver.recv().await;
        if let Some(msg) = msg {
            self.last_msg = Some(msg.try_cast_into().expect("Failed to cast message"));
        }
        self.last_msg.as_deref()
    }

    pub fn into_stream(self) -> impl Stream<Item = Arc<T>> {
        ReceiverStream::new(self.receiver)
            .map(|msg| msg.try_cast_into::<T>().expect("Failed to cast message"))
    }
}

impl<T: DynMessage + Clone> Subscription<T> {
    pub async fn recv_owned(&mut self) -> Option<T> {
        let msg = self.receiver.recv().await;
        if msg.is_none() {
            self.last_msg = None;
        }
        msg.map(|msg| {
            let arc = msg.try_cast_into().expect("Failed to cast message");
            Arc::unwrap_or_clone(arc)
        })
    }
}

pub struct SubscriberRef {
    pub(crate) subscriber_id: Uuid,
    pub(crate) subscriber_name: String,
    pub(crate) sender: Sender<TypeErasedMessage>,
}

pub trait Criteria: Send + Sync {
    fn matches(&self, subscriber: &SubscriberRef) -> bool;
}
pub struct SubscriberNameCriteria {
    name: &'static str,
}

impl Criteria for SubscriberNameCriteria {
    fn matches(&self, subscriber: &SubscriberRef) -> bool {
        subscriber.subscriber_name == self.name
    }
}

pub struct SubscriberIdCriteria {
    subscriber_id: Uuid,
}

impl Criteria for SubscriberIdCriteria {
    fn matches(&self, subscriber: &SubscriberRef) -> bool {
        subscriber.subscriber_id == self.subscriber_id
    }
}
