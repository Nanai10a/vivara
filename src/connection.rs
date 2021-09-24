use alloc::sync::Arc;

use actix::prelude::*;
use songbird::input::Input;
use songbird::tracks::Track;
use songbird::{Call, Songbird};
use tokio::sync::Mutex;
use url::Url;

use crate::gateway;
use crate::util::{token, Pipe};

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
                    kind: Kind::Source(input),
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
    type Result = ();

    fn handle(&mut self, _: Action, _: &mut Self::Context) -> Self::Result { unimplemented!() }
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

            use Kind::*;
            match kind {
                Source(input) => guard.enqueue_source(input),
                Track(track) => guard.enqueue(track),
            }

            gateway::get_reply_recipient().do_send(gateway::Reply {
                msg: "queued".to_string(),
                kind: gateway::Kind::Ok,
                to: from,
            }).unwrap();
        }
        .pipe(Box::pin)
    }
}
impl Supervised for Connector {}
impl ArbiterService for Connector {}

#[non_exhaustive]
enum Action {
    // Join,
    // Leave,
}
impl Message for Action {
    type Result = ();
}

struct QueueData {
    guild: u64,
    kind: Kind,
    from: gateway::MsgRef,
}

enum Kind {
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
}
impl Handler<CallRequest> for Caller {
    type Result = ResponseActFuture<Self, Arc<Mutex<Call>>>;

    fn handle(
        &mut self,
        CallRequest { guild }: CallRequest,
        _: &mut Self::Context,
    ) -> Self::Result {
        let is_initialized = self.songbird.is_some();
        async move {
            if !is_initialized {
                Self::init().await.pipe(Some)
            } else {
                None
            }
        }
        .into_actor(self)
        .map(move |maybe_init, this, _| {
            if let Some(sb) = maybe_init {
                this.songbird = sb.pipe(Some);
            }

            this.songbird.as_ref().unwrap().get_or_insert(guild.into())
        })
        .boxed_local()
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
