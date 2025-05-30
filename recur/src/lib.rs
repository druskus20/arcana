use std::{
    any::{Any, TypeId},
    collections::HashMap,
    pin::Pin,
};

// Event loop structure
pub struct EventLoop {
    handlers: HashMap<TypeId, Vec<BoxedHandler>>,
}

impl EventLoop {
    pub fn new() -> Self {
        Self {
            handlers: HashMap::new(),
        }
    }

    /// Add a handler for a specific event type
    /// This is the key method that supports different handler signatures
    pub fn add_handler<T, H>(&mut self, handler: H) -> &mut Self
    where
        T: Event + Clone,
        H: Handler<T>,
    {
        // Create a sample event to get the event type
        // In a real implementation, you'd want a different way to map types to event types
        let event_type = TypeId::of::<T>();

        // Type-erase the handler
        let boxed_handler: BoxedHandler = Box::new(move |event: Box<dyn Event>| {
            let handler = handler.clone();
            // Convert to Any and then downcast
            let fut = handler.call(*event);
            Box::pin(fut) as Pin<Box<dyn Future<Output = ()> + Send>>
        });

        self.handlers
            .entry(event_type)
            .or_insert_with(Vec::new)
            .push(boxed_handler);

        self
    }
}

// Handler trait - similar to Axum's Handler trait
pub trait Handler<T>: Clone + Send + Sync + 'static {
    type Future: Future<Output = ()> + Send + 'static;

    fn call(self, event: T) -> Self::Future;
}

// Implementation for function pointers and closures
impl<F, Fut, T> Handler<T> for F
where
    F: FnOnce(T) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send + 'static,
    T: Send + 'static,
{
    type Future = Fut;

    fn call(self, event: T) -> Self::Future {
        self(event)
    }
}

// Base event trait
pub trait Event: Send + Sync + 'static {
    fn event_type(&self) -> TypeId;
}

type BoxedHandler =
    Box<dyn Fn(Box<dyn Event>) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;
//type BoxedHandler =
//    Box<dyn Fn(Box<dyn Event>) -> Box<dyn Future<Output = ()> + Send> + Send + Sync>;

// Builder pattern for fluent configuration
pub struct EventLoopBuilder {
    event_loop: EventLoop,
}

impl EventLoopBuilder {
    pub fn new() -> Self {
        Self {
            event_loop: EventLoop::new(),
        }
    }

    pub fn add_handler<T, H>(mut self, handler: H) -> Self
    where
        T: Event + Clone,
        H: Handler<T>,
    {
        self.event_loop.add_handler::<T, H>(handler);
        self
    }

    pub fn build(self) -> EventLoop {
        self.event_loop
    }
}
