use std::{any::Any, hash::Hash, sync::Arc};

use thiserror::Error;

#[derive(Debug, Clone)]
pub struct MessageMeta {
    type_id: std::any::TypeId,
    type_name: &'static str,
}

impl MessageMeta {
    pub fn of<T: 'static>() -> Self {
        MessageMeta {
            type_id: std::any::TypeId::of::<T>(),
            type_name: std::any::type_name::<T>(),
        }
    }

    pub fn type_id(&self) -> std::any::TypeId {
        self.type_id
    }
}

impl PartialEq for MessageMeta {
    fn eq(&self, other: &Self) -> bool {
        self.type_id == other.type_id
    }
}

impl Eq for MessageMeta {}

impl Hash for MessageMeta {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.type_id.hash(state);
    }
}

#[derive(Debug)]
pub struct TypeErasedMessage {
    pub(super) message: Arc<dyn DynMessage>,
    pub(super) meta: MessageMeta,
    // TODO: Should we have a "origin" field to track where the message came from? - optionally
    // also useful for authentication
}

#[derive(Error, Debug)]
pub enum CastError {
    #[error("Type mismatch when casting TypeErasedMessage")]
    TypeMismatch(String),
    #[error("Failed to downcast TypeErasedMessage")]
    FailureToDowncast(String),
}
impl TypeErasedMessage {
    pub fn from<T: DynMessage>(message: T) -> Self {
        TypeErasedMessage {
            message: Arc::new(message),
            meta: MessageMeta::of::<T>(),
        }
    }

    pub fn try_cast_into<T: DynMessage>(self) -> Result<Arc<T>, CastError> {
        if self.meta.type_id != std::any::TypeId::of::<T>() {
            return Err(CastError::TypeMismatch(format!(
                "TypeErasedMessage type_id mismatch: expected {}, got {}",
                std::any::type_name::<T>(),
                self.meta.type_name
            )));
        }

        let msg = self.message.as_any_arc().downcast::<T>().map_err(|_| {
            CastError::TypeMismatch(format!(
                "Failed to downcast TypeErasedMessage from {} to {}",
                self.meta.type_name,
                std::any::type_name::<T>()
            ))
        })?;

        Ok(msg)
    }
}

pub trait DynMessage: std::fmt::Debug + Send + Sync + 'static {
    //fn clone_box(&self) -> Box<dyn DynMessage>;
    fn as_any_arc(self: Arc<Self>) -> Arc<dyn Any + Send + Sync>;
}

impl<T: std::fmt::Debug + Send + Sync + 'static> DynMessage for T {
    fn as_any_arc(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }

    //fn clone_box(&self) -> Box<dyn DynMessage> {
    //    Box::new(self.clone())
    //}
}

impl TypeErasedMessage {
    pub fn type_id(&self) -> std::any::TypeId {
        self.meta.type_id
    }
    pub fn type_name(&self) -> &'static str {
        self.meta.type_name
    }

    pub fn shared(self) -> Arc<TypeErasedMessage> {
        Arc::new(self)
    }
}

impl Clone for TypeErasedMessage {
    fn clone(&self) -> Self {
        TypeErasedMessage {
            message: Arc::clone(&self.message),
            meta: self.meta.clone(),
        }
    }
}
