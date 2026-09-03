//! Typed routing to instance and payload-module current-event hosts.

use crate::{
    CurrentEventHost, CurrentEventHostExecution, CurrentEventStartRequest, CurrentEventStopRequest,
    CurrentEventUpdateRequest, ExecutableEventId, FunctionAdapter, IntervalCallbackInvocation,
    LayerId, ModuleAdapter, ModuleId, RuntimeContext, RuntimeError, RuntimeExecutor,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CurrentEventRoute {
    Primary,
    Payload(ModuleId),
}

impl<O, E, M, F> RuntimeExecutor<'_, O, E, M, F>
where
    E: Clone,
    M: ModuleAdapter<O, E, F>,
    F: FunctionAdapter<O, E, M>,
{
    pub(crate) fn payload_current_event_host_available(&mut self, owner: ModuleId) -> bool {
        self.modules.current_event_host(owner).is_some()
    }

    pub(crate) fn invoke_current_event_callback(
        &mut self,
        invocation: IntervalCallbackInvocation<'_, E>,
        callback_guard_layer: Option<LayerId>,
        callback_registration_target: Option<crate::runtime::CallbackRegistrationTarget>,
        synchronous_stop_exit: bool,
    ) -> Result<(), RuntimeError<M::Error, F::Error>> {
        let (result, failure) = {
            let RuntimeExecutor {
                state,
                modules,
                functions,
            } = self;
            let mut context = RuntimeContext::<O, E, M, F>::new(
                state,
                callback_guard_layer,
                synchronous_stop_exit,
            );
            if let Some(target) = callback_registration_target {
                context = context.with_callback_registration_target(target);
            }
            let result = functions.execute_interval_callback(invocation, modules, &mut context);
            (result, context.take_failure())
        };
        if let Some(error) = failure {
            return Err(error);
        }
        if let Err(error) = result {
            return self.function_failure(error);
        }
        Ok(())
    }

    pub(crate) fn start_current_event(
        &mut self,
        route: CurrentEventRoute,
        request: CurrentEventStartRequest,
    ) -> Result<(), RuntimeError<M::Error, F::Error>> {
        if self.current_event_host_execution == CurrentEventHostExecution::Suppressed {
            return Ok(());
        }
        match route {
            CurrentEventRoute::Primary => {
                self.with_primary_current_event_host(|host| host.start_current_event(request))
            }
            CurrentEventRoute::Payload(owner) => self
                .with_payload_current_event_host(owner, |host| host.start_current_event(request))
                .map(|_| ()),
        }
    }

    pub(crate) fn stop_current_event(
        &mut self,
        route: CurrentEventRoute,
        request: CurrentEventStopRequest,
    ) -> Result<(), RuntimeError<M::Error, F::Error>> {
        if self.current_event_host_execution == CurrentEventHostExecution::Suppressed {
            return Ok(());
        }
        match route {
            CurrentEventRoute::Primary => {
                self.with_primary_current_event_host(|host| host.stop_current_event(request))
            }
            CurrentEventRoute::Payload(owner) => self
                .with_payload_current_event_host(owner, |host| host.stop_current_event(request))
                .map(|_| ()),
        }
    }

    pub(crate) fn update_current_event(
        &mut self,
        route: CurrentEventRoute,
        request: CurrentEventUpdateRequest,
    ) -> Result<(), RuntimeError<M::Error, F::Error>> {
        if self.current_event_host_execution == CurrentEventHostExecution::Suppressed {
            return Ok(());
        }
        match route {
            CurrentEventRoute::Primary => {
                self.with_primary_current_event_host(|host| host.update_current_event(request))
            }
            CurrentEventRoute::Payload(owner) => self
                .with_payload_current_event_host(owner, |host| host.update_current_event(request))
                .map(|_| ()),
        }
    }

    pub(crate) fn current_event_gate(
        &mut self,
        event_id: ExecutableEventId,
    ) -> Result<bool, RuntimeError<M::Error, F::Error>> {
        self.with_primary_current_event_host(|host| host.current_event_gate(event_id))
    }

    pub(crate) fn with_primary_current_event_host<T>(
        &mut self,
        call: impl FnOnce(&mut dyn CurrentEventHost<E, Error = F::Error>) -> Result<T, F::Error>,
    ) -> Result<T, RuntimeError<M::Error, F::Error>> {
        let Some(host) = self.functions.current_event_host() else {
            self.poisoned = true;
            return Err(RuntimeError::MissingCurrentEventHost);
        };
        match call(host) {
            Ok(value) => Ok(value),
            Err(error) => self.function_failure(error),
        }
    }

    fn with_payload_current_event_host<T>(
        &mut self,
        owner: ModuleId,
        call: impl FnOnce(&mut dyn CurrentEventHost<E, Error = M::Error>) -> Result<T, M::Error>,
    ) -> Result<Option<T>, RuntimeError<M::Error, F::Error>> {
        let Some(host) = self.modules.current_event_host(owner) else {
            return Ok(None);
        };
        match call(host) {
            Ok(value) => Ok(Some(value)),
            Err(error) => self.module_failure(error),
        }
    }
}
