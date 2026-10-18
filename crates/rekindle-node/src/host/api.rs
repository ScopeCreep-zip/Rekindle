//! The host's side of [`rekindle_ipc::HostApi`] (plan E1.1): the trait,
//! [`Caller`] and [`ConnId`] live in `rekindle-ipc`, so frontends name them
//! without the node.

use rekindle_events::EventStream;
use rekindle_ipc::error::IpcError;
use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_ipc::{Caller, HostApi};
use rekindle_types::subscription_events::Cursor;

#[async_trait::async_trait]
impl HostApi for super::Host {
    async fn call(&self, caller: Caller, req: IpcRequest) -> IpcResponse {
        self.ctx.call(req, &caller).await
    }

    async fn subscribe(&self, from: Option<Cursor>) -> Result<EventStream, IpcError> {
        Ok(self.ctx.journal.subscribe(from))
    }
}
