use actix::prelude::{
    Actor, ArbiterService, AsyncContext, Context, ContextFutureSpawner, Handler, Message,
    Supervised, WrapFuture,
};
use futures_util::StreamExt;
use twilight_http::Client;
use twilight_model::gateway::event::Event;

use crate::cmd::CommandParser;
use crate::util::{token, Pipe};

#[derive(Debug, Clone, Copy)]
pub struct MessageRef {
    message: u64,
    channel: u64,
}

#[derive(Debug, Clone)]
pub struct RawCommand {
    pub content: String,
    pub from: MessageRef,
    pub guild: Option<u64>,
    pub user: u64,
}
impl Message for RawCommand {
    type Result = ();
}

#[derive(Default)]
pub struct Gateway;
impl Actor for Gateway {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        let addr = ctx.address();

        async move {
            let (cluster, mut events) = loop {
                match twilight_gateway::Cluster::new(
                    token::<String>(),
                    twilight_gateway::Intents::GUILD_VOICE_STATES,
                )
                .await
                {
                    Ok(t) => break t,
                    Err(e) => tracing::warn!("failed initializing cluster: {}", e),
                }
            };

            cluster.up().await;

            while let Some((_, e)) = events.next().await {
                if let Event::MessageCreate(mc) = e {
                    GatewayMessage {
                        content: mc.0.content,
                        from: MessageRef {
                            message: mc.0.id.0,
                            channel: mc.0.channel_id.0,
                        },
                        user: mc.0.author.id.0,
                        guild: mc.0.guild_id.map(|i| i.0),
                    }
                    .pipe(|m| addr.try_send(m).expect("failed sending"));
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

struct GatewayMessage {
    content: String,
    from: MessageRef,
    guild: Option<u64>,
    user: u64,
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
