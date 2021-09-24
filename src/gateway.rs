use actix::prelude::*;
use futures_util::StreamExt;
use twilight_gateway::{Cluster, Intents};
use twilight_http::Client;
use twilight_model::gateway::event::Event;

use crate::cmd::CommandParser;
use crate::util::{token, Pipe};

#[derive(Debug)]
pub struct MsgRef {
    message: u64,
    channel: u64,
    guild: Option<u64>,
}
impl MsgRef {
    pub fn is_dm(&self) -> bool { self.guild.is_none() }

    pub fn guild(&self) -> Option<u64> { self.guild }
}

pub struct RawCommand {
    pub content: String,
    pub from: MsgRef,
    pub user: u64,
}
impl Message for RawCommand {
    type Result = ();
}

pub fn get_reply_recipient() -> Recipient<Reply> { unimplemented!() }

#[derive(Default)]
struct Gateway;
impl Actor for Gateway {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        let addr = ctx.address();

        async move {
            let (clu, mut eve) = Cluster::new(
                token::<String>(),
                Intents::GUILD_MESSAGES | Intents::DIRECT_MESSAGES,
            )
            .await
            .unwrap();

            clu.up().await;

            while let Some((_, e)) = eve.next().await {
                if let Event::MessageCreate(mc) = e {
                    GatewayMessage {
                        content: mc.0.content,
                        from: MsgRef {
                            message: mc.0.id.0,
                            channel: mc.0.channel_id.0,
                            guild: mc.0.guild_id.map(|i| i.0),
                        },
                        user: mc.0.author.id.0,
                    }
                    .pipe(|m| addr.do_send(m));
                }
            }
        }
        .into_actor(self)
        .spawn(ctx)
    }
}
impl Handler<GatewayMessage> for Gateway {
    type Result = ();

    fn handle(
        &mut self,
        GatewayMessage {
            content,
            from,
            user,
        }: GatewayMessage,
        _: &mut Self::Context,
    ) -> Self::Result {
        CommandParser::from_registry().do_send(RawCommand {
            content,
            from,
            user,
        })
    }
}
impl Supervised for Gateway {}
impl ArbiterService for Gateway {}

struct GatewayMessage {
    content: String,
    from: MsgRef,
    user: u64,
}

impl Message for GatewayMessage {
    type Result = ();
}

struct Responder {
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
            kind,
            to:
                MsgRef {
                    message,
                    channel,
                    guild,
                },
        }: Reply,
        ctx: &mut Self::Context,
    ) -> Self::Result {
        self.client
            .create_message(channel.into())
            .content(&format!("{}: {}", kind, msg))
            .unwrap()
            .reply(message.into())
            .exec()
            .pipe(|f| async {
                match f.await {
                    Ok(_) => (),
                    Err(e) => tracing::warn!("send error: {}", e),
                };
            })
            .into_actor(self)
            .spawn(ctx)
    }
}
impl Supervised for Responder {}
impl ArbiterService for Responder {}

#[derive(Debug)]
pub struct Reply {
    pub msg: String,
    pub kind: Kind,
    pub to: MsgRef,
}

#[derive(Debug)]
pub enum Kind {
    Ok,
    Err,
}
impl Message for Reply {
    type Result = ();
}

impl core::fmt::Display for Kind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Kind::Ok => write!(f, "ok"),
            Kind::Err => write!(f, "err"),
        }
    }
}
