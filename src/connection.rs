use alloc::sync::Arc;

use actix::prelude::*;
use songbird::input::Input;
use songbird::tracks::Track;
use songbird::{Call, Songbird};
use tokio::sync::Mutex;
use url::Url;

use crate::gateway;
use crate::util::{reply, token, Pipe};

#[derive(Default)]
pub struct UrlQueue;
impl Actor for UrlQueue {
    type Context = Context<Self>;
}
impl Handler<UrlQueueData> for UrlQueue {
    type Result = ResponseActFuture<Self, ()>;

    fn handle(
        &mut self,
        UrlQueueData { url, from, guild }: UrlQueueData,
        _: &mut Self::Context,
    ) -> Self::Result {
        songbird::ytdl(url)
            .into_actor(self)
            .map(move |res, _, _| {
                let input = try_handle!(res; to = from);

                Connector::from_registry().do_send(QueueData {
                    guild,
                    kind: QueueDataKind::Source(input),
                    from,
                });
            })
            .boxed_local()
    }
}
impl Supervised for UrlQueue {}
impl ArbiterService for UrlQueue {}

pub struct UrlQueueData {
    pub url: Url,
    pub from: gateway::MsgRef,
    pub guild: u64,
}
impl Message for UrlQueueData {
    type Result = ();
}

#[derive(Default)]
pub struct Connector;
impl Actor for Connector {
    type Context = Context<Self>;
}
impl Handler<Action> for Connector {
    type Result = ResponseFuture<()>;

    fn handle(
        &mut self,
        Action { kind, from, guild }: Action,
        _: &mut Self::Context,
    ) -> Self::Result {
        async move {
            let arc = Caller::from_registry()
                .send(CallRequest { guild })
                .await
                .unwrap();
            let mut guard = arc.lock().await;

            use ActionKind::*;
            match kind {
                Join { channel } => {
                    let join_fut = try_handle!(guard.join(channel.into()).await; to = from);
                    drop(guard);

                    try_handle!(join_fut.await; to = from);

                    reply("joined", from)
                },
                Leave => {
                    try_handle!(guard.leave().await; to = from);

                    reply("leaved", from)
                },
            }
        }
        .pipe(Box::pin)
    }
}
impl Handler<QueueData> for Connector {
    type Result = ResponseFuture<()>;

    fn handle(
        &mut self,
        QueueData { guild, kind, from }: QueueData,
        _: &mut Self::Context,
    ) -> Self::Result {
        async move {
            let arc = try_handle!(Caller::from_registry()
            .send(CallRequest { guild }).await; to = from);
            let mut guard = arc.lock().await;

            use QueueDataKind::*;
            match kind {
                Source(input) => guard.enqueue_source(input),
                Track(track) => guard.enqueue(track),
            }

            reply("queued", from)
        }
        .pipe(Box::pin)
    }
}
impl Supervised for Connector {}
impl ArbiterService for Connector {}

#[non_exhaustive]
struct Action {
    kind: ActionKind,
    from: gateway::MsgRef,
    guild: u64,
}

enum ActionKind {
    Join { channel: u64 },
    Leave,
}
impl Message for Action {
    type Result = ();
}

struct QueueData {
    guild: u64,
    kind: QueueDataKind,
    from: gateway::MsgRef,
}

enum QueueDataKind {
    Source(Input),
    Track(Track),
}
impl Message for QueueData {
    type Result = ();
}

#[derive(Default)]
struct Caller {
    songbird: Option<Songbird>,
}
impl Caller {
    async fn init() -> Songbird {
        let cluster =
            twilight_gateway::Cluster::new(token::<String>(), twilight_gateway::Intents::empty())
                .await
                .unwrap()
                .0;

        cluster.up().await;

        let user_id = twilight_http::Client::new(token())
            .current_user()
            .exec()
            .await
            .unwrap()
            .model()
            .await
            .unwrap()
            .id;

        Songbird::twilight(cluster, user_id)
    }
}
impl Actor for Caller {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        async { Self::init().await }
            .into_actor(self)
            .map(|sb, this, _| {
                this.songbird = sb.pipe(Some);
            })
            .pipe(|f| ctx.wait(f))
    }
}
impl Handler<CallRequest> for Caller {
    type Result = Arc<Mutex<Call>>;

    fn handle(
        &mut self,
        CallRequest { guild }: CallRequest,
        _: &mut Self::Context,
    ) -> Self::Result {
        self.songbird.as_ref().unwrap().get_or_insert(guild.into())
    }
}
impl Supervised for Caller {}
impl ArbiterService for Caller {}

struct CallRequest {
    guild: u64,
}
impl Message for CallRequest {
    type Result = Arc<Mutex<Call>>;
}
