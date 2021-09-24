use actix::prelude::*;
use url::Url;

use crate::gateway::MsgRef;

#[derive(Default)]
pub struct UrlQueue;
impl Actor for UrlQueue {
    type Context = Context<Self>;
}
impl Handler<UrlQueueData> for UrlQueue {
    type Result = ();

    fn handle(
        &mut self,
        UrlQueueData { url, from, to }: UrlQueueData,
        _: &mut Self::Context,
    ) -> Self::Result {
        unimplemented!()
    }
}
impl Supervised for UrlQueue {}
impl ArbiterService for UrlQueue {}

pub struct UrlQueueData {
    pub url: Url,
    pub from: u64,
    pub to: MsgRef,
}
impl Message for UrlQueueData {
    type Result = ();
}
