use std::hash::Hash;

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
    pub(super) message: Box<dyn DynClonableMessage>,
    pub(super) meta: MessageMeta,
}

impl TypeErasedMessage {
    pub fn from<T: DynClonableMessage>(message: T) -> Self {
        TypeErasedMessage {
            message: Box::new(message),
            meta: MessageMeta::of::<T>(),
        }
    }
}

pub trait DynClonableMessage: std::fmt::Debug + Send + Sync + 'static {
    fn clone_box(&self) -> Box<dyn DynClonableMessage>;
}

impl<T: std::fmt::Debug + Send + Sync + 'static + Clone> DynClonableMessage for T {
    fn clone_box(&self) -> Box<dyn DynClonableMessage> {
        Box::new(self.clone())
    }
}

impl Clone for TypeErasedMessage {
    fn clone(&self) -> Self {
        TypeErasedMessage {
            message: self.message.clone_box(),
            meta: self.meta.clone(),
        }
    }
}

impl TypeErasedMessage {
    pub fn type_id(&self) -> std::any::TypeId {
        self.meta.type_id
    }
    pub fn type_name(&self) -> &'static str {
        self.meta.type_name
    }
}
