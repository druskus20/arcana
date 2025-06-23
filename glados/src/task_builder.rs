
use crate::SpawnTaskError;

// Marker types
pub struct NoFunct;
pub struct HasFunct;
pub struct NoCancelFunct;
pub struct HasCancelFunct;
pub struct NoCtx;
pub struct HasCtx;

pub struct TaskBuilder<F, C, X, FunctState, CancelState, CtxState> {
    name: String,
    funct: Option<F>,
    cancel_funct: Option<C>,
    ctx: Option<X>,
    _funct_state: FunctState,
    _cancel_state: CancelState,
    _ctx_state: CtxState,
}

impl TaskBuilder<(), (), (), NoFunct, NoCancelFunct, NoCtx> {
    pub fn new(name: &str) -> Self {
        TaskBuilder {
            name: name.to_string(),
            funct: None,
            cancel_funct: None,
            ctx: None,
            _funct_state: NoFunct,
            _cancel_state: NoCancelFunct,
            _ctx_state: NoCtx,
        }
    }
}

impl<C, X> TaskBuilder<(), C, X, NoFunct, NoCancelFunct, NoCtx> {
    pub fn with_funct<NF>(
        self,
        funct: NF,
    ) -> TaskBuilder<NF, C, X, HasFunct, NoCancelFunct, NoCtx> {
        TaskBuilder {
            name: self.name,
            funct: Some(funct),
            cancel_funct: self.cancel_funct,
            ctx: self.ctx,
            _funct_state: HasFunct,
            _cancel_state: self._cancel_state,
            _ctx_state: self._ctx_state,
        }
    }
}

impl<F, X> TaskBuilder<F, (), X, HasFunct, NoCancelFunct, HasCtx> {
    pub fn with_cancel_funct<NC>(
        self,
        cancel_funct: NC,
    ) -> TaskBuilder<F, NC, X, HasFunct, HasCancelFunct, HasCtx> {
        TaskBuilder {
            name: self.name,
            funct: self.funct,
            cancel_funct: Some(cancel_funct),
            ctx: self.ctx,
            _funct_state: self._funct_state,
            _cancel_state: HasCancelFunct,
            _ctx_state: self._ctx_state,
        }
    }
}

impl<F, C> TaskBuilder<F, C, (), HasFunct, NoCancelFunct, NoCtx> {
    pub fn with_ctx<NX>(self, ctx: NX) -> TaskBuilder<F, C, NX, HasFunct, NoCancelFunct, HasCtx> {
        TaskBuilder {
            name: self.name,
            funct: self.funct,
            cancel_funct: self.cancel_funct,
            ctx: Some(ctx),
            _funct_state: self._funct_state,
            _cancel_state: self._cancel_state,
            _ctx_state: HasCtx,
        }
    }
}

impl<F, Ctx, CF, Fut, Fut2, E, E2> TaskBuilder<F, CF, Ctx, HasFunct, HasCancelFunct, HasCtx>
where
    F: 'static + Send + FnOnce(Ctx) -> Fut,
    Fut: Send + std::future::Future<Output = Result<(), E>>,
    Ctx: 'static + Send,
    E: Send + 'static + std::fmt::Debug,
    CF: 'static + Send + FnOnce() -> Fut2,
    Fut2: Send + std::future::Future<Output = Result<(), E2>>,
    E2: Send + 'static + std::fmt::Debug,
{
    pub fn spawn_async(self, glados: &crate::GladosHandle) -> Result<(), SpawnTaskError> {
        let funct = self.funct.expect("Function must be provided");
        let cancel_funct = self.cancel_funct.expect("Cancel function must be provided");
        let ctx = self.ctx.expect("Context must be provided");

        glados.spawn_async::<F, Fut, Ctx, E, CF, E2, Fut2>(&self.name, ctx, funct, cancel_funct)
    }
}
impl<F, Ctx, CF, E, Fut2> TaskBuilder<F, CF, Ctx, HasFunct, HasCancelFunct, HasCtx>
where
    F: 'static + FnOnce(Ctx) -> Result<(), E> + Send,
    Ctx: 'static + Send,
    E: 'static + Send + std::fmt::Debug,
    CF: 'static + Send + FnOnce() -> Fut2,
    Fut2: Send + std::future::Future<Output = ()>,
{
    pub fn spawn_thread(self, glados: &crate::GladosHandle) -> Result<(), SpawnTaskError> {
        let funct = self.funct.expect("Function must be provided");
        let ctx = self.ctx.expect("Context must be provided");
        let cancel_funct = self.cancel_funct.expect("Cancel function must be provided");

        glados.spawn_thread::<F, Ctx, E, CF, Fut2>(&self.name, ctx, funct, cancel_funct)
    }
}

#[cfg(test)]
mod test {
    use super::*;
    type Result<T> = std::result::Result<T, ()>;

    #[tokio::test]
    async fn test_task_builder() {
        let builder = TaskBuilder::new("test")
            .with_funct(|_ctx: ()| async { Result::Ok(()) })
            .with_ctx(())
            .with_cancel_funct(|| async { Result::Ok(()) });

        assert!(builder.funct.is_some());
        assert!(builder.cancel_funct.is_some());
        assert!(builder.ctx.is_some());
    }
}
