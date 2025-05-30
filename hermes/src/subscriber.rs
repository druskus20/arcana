use std::marker::PhantomData;

use futures::{Stream, StreamExt};
use tokio::sync::mpsc::{Receiver, Sender};
use tokio_stream::wrappers::ReceiverStream;
use uuid::Uuid;

use crate::message::{DynClonableMessage, TypeErasedMessage};

pub struct Subscription<T> {
    _subscriber_id: Uuid,
    message_type: PhantomData<T>,
    receiver: Receiver<TypeErasedMessage>,
}

impl<T: DynClonableMessage> Subscription<T> {
    pub fn from_receiver(_subscriber_id: Uuid, receiver: Receiver<TypeErasedMessage>) -> Self {
        Subscription {
            _subscriber_id,
            message_type: PhantomData,
            receiver,
        }
    }

    pub async fn recv(&mut self) -> Option<T> {
        let msg = self.receiver.recv().await;
        msg.map(|msg| msg.try_cast_into().expect("Failed to cast message"))
    }

    pub fn into_stream(self) -> impl Stream<Item = T> {
        ReceiverStream::new(self.receiver)
            .map(|msg| msg.try_cast_into::<T>().expect("Failed to cast message"))
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
