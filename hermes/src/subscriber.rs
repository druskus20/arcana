use std::marker::PhantomData;

use tokio::sync::mpsc::{Receiver, Sender};
use uuid::Uuid;

use crate::message::TypeErasedMessage;

pub struct Subscription<T> {
    subscriber_id: Uuid,
    message_type: PhantomData<T>,
    receiver: Receiver<TypeErasedMessage>,
}

impl<T> Subscription<T> {
    pub fn from_receiver(subscriber_id: Uuid, receiver: Receiver<TypeErasedMessage>) -> Self {
        Subscription {
            subscriber_id,
            message_type: PhantomData,
            receiver,
        }
    }

    pub async fn recv(&mut self) -> Option<TypeErasedMessage> {
        self.receiver.recv().await
    }
}

pub struct SubscriberRef {
    pub(crate) subscriber_id: Uuid,
    pub(crate) subscriber_name: String,
    pub(crate) sender: Sender<TypeErasedMessage>,
}

pub trait Criteria: Send {
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
