use alloc::sync::Arc;
use std::thread::spawn;

use actix::prelude::{
    Actor, ActorFutureExt, ArbiterService, Context, ContextFutureSpawner, Handler, Message,
    ResponseFuture, Supervised, WrapFuture,
};
use dashmap::{DashMap, Map};
use futures_util::StreamExt;
use songbird::error::JoinError;
use songbird::input::cached::Memory;
use songbird::input::Input;
use songbird::tracks::TrackHandle;
use songbird::{Call, Songbird};
use tokio::sync::Mutex;
use twilight_gateway::cluster::Events;
use twilight_gateway::{Cluster, Intents};
use url::Url;

use crate::gateway::MessageRef;
use crate::util::{reply, reply_err, token, Pipe};

#[derive(Default)]
pub struct Queuer;
impl Actor for Queuer {
    type Context = Context<Self>;
}
impl Handler<UrlQueueData> for Queuer {
    type Result = ();

    fn handle(
        &mut self,
        UrlQueueData { url, from, guild }: UrlQueueData,
        ctx: &mut Self::Context,
    ) -> Self::Result {
        songbird::ytdl(url)
            .into_actor(self)
            .map(move |res, _, _| {
                let result: Result<(), String> = try {
                    let input = res.map_err(|e| e.to_string())?;

                    let input = Memory::new(input)
                        .map(|m| m.try_into())
                        .flatten()
                        .map_err(|e| e.to_string())?;

                    Connector::from_registry()
                        .try_send(QueueData { guild, from, input })
                        .expect("failed sending")
                };

                if let Err(e) = result {
                    reply_err(e, from)
                }
            })
            .wait(ctx)
    }
}
impl Supervised for Queuer {}
impl ArbiterService for Queuer {}

pub struct UrlQueueData {
    pub url: Url,
    pub from: MessageRef,
    pub guild: u64,
}
impl Message for UrlQueueData {
    type Result = ();
}

#[derive(Default)]
pub struct Connector {
    queues: Arc<DashMap<usize, Vec<Input>>>,
    handles: Arc<DashMap<usize, TrackHandle>>,
}
impl Actor for Connector {
    type Context = Context<Self>;
}
impl Handler<Action> for Connector {
    type Result = ();

    fn handle(
        &mut self,
        Action { kind, from, guild }: Action,
        ctx: &mut Self::Context,
    ) -> Self::Result {
        let queues = self.queues.clone();
        let handles = self.handles.clone();

        async move {
            let arc = Caller::from_registry()
                .send(CallRequest { guild })
                .await
                .expect("failed sending");
            let key = Arc::as_ptr(&arc) as usize;
            let mut guard = arc.lock().await;

            let result: Result<&str, String> = try {
                use ActionKind::*;
                match kind {
                    Join { channel } => {
                        let join = guard
                            .join(channel.into())
                            .await
                            .map_err(|e| e.to_string())?;

                        drop(guard);

                        join.await.map_err(|e| e.to_string())?;

                        "joined"
                    },
                    Play => {
                        let input = match queues.get_mut(&key) {
                            None => Err("queue is empty")?,
                            Some(mut vec) => match vec.len() {
                                0 => Err("queue is empty")?,
                                _ => vec.remove(0),
                            },
                        };

                        let handle = guard.play_source(input);
                        handles.insert(key, handle);

                        "playing"
                    },
                    Stop => {
                        let handle = match handles.get(&key) {
                            Some(h) => h,
                            None => Err("not playing")?,
                        };

                        handle.stop().map_err(|e| e.to_string())?;

                        "stopped"
                    },
                    Leave => guard
                        .leave()
                        .await
                        .map_err(|e| e.to_string())?
                        .pipe(|()| "leaved"),
                }
            };

            match result {
                Ok(o) => reply(o, from),
                Err(e) => reply_err(e, from),
            }
        }
        .into_actor(self)
        .spawn(ctx);
    }
}
impl Handler<QueueData> for Connector {
    type Result = ();

    fn handle(
        &mut self,
        QueueData { guild, from, input }: QueueData,
        ctx: &mut Self::Context,
    ) -> Self::Result {
        let queues = self.queues.clone();

        async move {
            Caller::from_registry()
                .send(CallRequest { guild })
                .await
                .expect("failed sending")
                .pipe(|a| Arc::as_ptr(&a))
                .pipe(|p| p as usize)
                .pipe(Some)
        }
        .into_actor(self)
        .map(move |opt, _, _| {
            if let Some(key) = opt {
                let mut queue = queues._entry(key).or_default();

                queue.push(input);

                reply("queued", from)
            }
        })
        .wait(ctx);
    }
}
impl Supervised for Connector {}
impl ArbiterService for Connector {}

pub struct Action {
    pub kind: ActionKind,
    pub from: MessageRef,
    pub guild: u64,
}
pub enum ActionKind {
    Join { channel: u64 },
    Play,
    Stop,
    Leave,
}
impl Message for Action {
    type Result = ();
}

struct QueueData {
    input: Input,
    from: MessageRef,
    guild: u64,
}
impl Message for QueueData {
    type Result = ();
}

struct Caller {
    songbird: Arc<Songbird>,
    events: Option<Events>,
}
impl Caller {
    async fn init() -> (Songbird, Events) {
        let (cluster, events) = loop {
            match Cluster::new(token::<String>(), Intents::GUILD_VOICE_STATES).await {
                Ok(t) => break t,
                Err(e) => tracing::warn!("failed initializing cluster: {}", e),
            }
        };

        cluster.up().await;

        let user_id = loop {
            let result = twilight_http::Client::new(token())
                .current_user()
                .exec()
                .await;

            let response = match result {
                Ok(o) => o,
                Err(e) => {
                    tracing::warn!("failed getting current_user: {}", e);
                    continue;
                },
            };

            let user = match response.model().await {
                Ok(o) => o,
                Err(e) => {
                    tracing::warn!(
                        "failed deserialization to response of getting current_user: {}",
                        e
                    );
                    continue;
                },
            };

            break user.id;
        };

        (Songbird::twilight(cluster, user_id), events)
    }
}
impl Default for Caller {
    fn default() -> Self {
        let handle = tokio::runtime::Handle::current();
        let (songbird, events) = spawn(move || handle.block_on(async { Self::init().await }))
            .join()
            .expect("failed joining thread");

        Self {
            songbird: Arc::new(songbird),
            events: events.pipe(Some),
        }
    }
}
impl Actor for Caller {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        let arc = self.songbird.clone();
        let mut events = self.events.take().expect("must be found value");

        async move {
            while let Some((id, event)) = events.next().await {
                tracing::trace!("received event: ({}) {:?}", id, event);
                arc.process(&event).await;
            }
        }
        .into_actor(self)
        .spawn(ctx);
    }
}
impl Handler<GetCall> for Caller {
    type Result = Option<Arc<Mutex<Call>>>;

    fn handle(&mut self, GetCall { guild }: GetCall, ctx: &mut Self::Context) -> Self::Result {
        self.songbird.get(guild)
    }
}
impl Handler<DeleteCall> for Caller {
    type Result = ResponseFuture<Result<bool, String>>;

    fn handle(&mut self, DeleteCall { guild }: DeleteCall, _: &mut Self::Context) -> Self::Result {
        let songbird = self.songbird.clone();

        async move {
            match songbird.remove(guild).await {
                Ok(o) => Ok(true),
                Err(e) => match e {
                    JoinError::NoCall => Ok(false),
                    e => Err(e.to_string()),
                },
            }
        }.pipe(Box::pin)
    }
}
impl Supervised for Caller {}
impl ArbiterService for Caller {}

struct GetCall {
    guild: u64,
}
impl Message for GetCall {
    type Result = Option<Arc<Mutex<Call>>>;
}

struct DeleteCall {
    guild: u64,
}
impl Message for DeleteCall {
    type Result = Result<bool, String>;
}
