use actix::prelude::{
    Actor, ArbiterService, Context, ContextFutureSpawner, Handler, Message, Supervised, WrapFuture,
};
use twilight_http::Client;

use crate::command::CommandParser;
use crate::util::{token, Pipe};

#[derive(Debug, Clone, Copy)]
pub struct MessageRef {
    pub message: u64,
    pub channel: u64,
}

#[derive(Debug, Clone)]
pub struct RawCommand {
    pub content: String,
    pub user: u64,
    pub from: MessageRef,
    pub guild: Option<u64>,
}
impl Message for RawCommand {
    type Result = ();
}

#[derive(Default)]
pub struct Gateway;
impl Actor for Gateway {
    type Context = Context<Self>;
}
impl Handler<GatewayMessage> for Gateway {
    type Result = ();

    fn handle(
        &mut self,
        GatewayMessage {
            content,
            from,
            user,
            guild,
        }: GatewayMessage,
        _: &mut Self::Context,
    ) -> Self::Result {
        CommandParser::from_registry()
            .try_send(RawCommand {
                content,
                from,
                user,
                guild,
            })
            .expect("failed sending")
    }
}
impl Supervised for Gateway {}
impl ArbiterService for Gateway {}

pub struct GatewayMessage {
    pub content: String,
    pub user: u64,
    pub from: MessageRef,
    pub guild: Option<u64>,
}

impl Message for GatewayMessage {
    type Result = ();
}

pub struct Responder {
    client: Client,
}
impl Default for Responder {
    fn default() -> Self {
        Self {
            client: Client::new(token()),
        }
    }
}
impl Actor for Responder {
    type Context = Context<Self>;
}
impl Handler<Reply> for Responder {
    type Result = ();

    fn handle(
        &mut self,
        Reply {
            msg,
            to: MessageRef { message, channel },
        }: Reply,
        ctx: &mut Self::Context,
    ) -> Self::Result {
        self.client
            .create_message(channel.into())
            .content(&msg)
            .expect("illegal message")
            .reply(message.into())
            .exec()
            .pipe(|f| async {
                match f.await {
                    Ok(_) => (),
                    Err(e) => tracing::error!("failed sending response: {}", e),
                };
            })
            .into_actor(self)
            .spawn(ctx)
    }
}
impl Supervised for Responder {}
impl ArbiterService for Responder {}

#[derive(Debug, Clone)]
pub struct Reply {
    pub msg: String,
    pub to: MessageRef,
}
impl Message for Reply {
    type Result = ();
}
