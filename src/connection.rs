use alloc::sync::Arc;
use core::ops::Bound;
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

use crate::gateway::MessageRef;
use crate::util::{reply, reply_err, token, Pipe};

#[derive(Default)]
pub struct Connector {
    queues: Arc<DashMap<usize, Vec<Input>>>,
    handles: Arc<DashMap<usize, TrackHandle>>,
}
impl Connector {
    async fn create_track(url: String) -> Result<Input, String> {
        let source = songbird::ytdl(url).await.map_err(|e| e.to_string())?;

        let input = Memory::new(source)
            .map_err(|e| e.to_string())?
            .pipe(|m| m.try_into())
            .map_err(|e: songbird::input::error::Error| e.to_string())?;

        Ok(input)
    }
}
impl Actor for Connector {
    type Context = Context<Self>;
}
impl Handler<CallAction> for Connector {
    type Result = ();

    fn handle(
        &mut self,
        CallAction { kind, from, guild }: CallAction,
        ctx: &mut Self::Context,
    ) -> Self::Result {
        let queues = self.queues.clone();
        let handles = self.handles.clone();

        async move {
            let result: Result<&str, String> = try {
                use CallActionKind::*;
                match kind {
                    Join { channel } => {
                        let arc = Caller::from_registry()
                            .send(DoCall { guild })
                            .await
                            .expect("failed sending")
                            .ok_or("now calling")?;
                        let mut guard = arc.lock().await;

                        let join = guard
                            .join(channel.into())
                            .await
                            .map_err(|e| e.to_string())?;

                        drop(guard);

                        join.await.map_err(|e| e.to_string())?;

                        "joined"
                    },
                    Leave => {
                        let result = Caller::from_registry()
                            .send(DeleteCall { guild })
                            .await
                            .expect("failed sending");

                        let (resp, key) = match result {
                            Ok(Some(k)) => ("leaved", k),
                            Ok(None) => Err("not calling")?,
                            Err(e) => Err(e)?,
                        };

                        queues.remove(&key);
                        handles.remove(&key);

                        resp
                    },
                    Play { url } => {
                        let arc = Caller::from_registry()
                            .send(GetCall { guild })
                            .await
                            .expect("failed sending")
                            .ok_or("not calling")?;
                        let mut guard = arc.lock().await;

                        let key = Arc::as_ptr(&arc) as usize;

                        match (queues.get_mut(&key), handles.get(&key).is_some(), url) {
                            (None, false, None) => Err("queue is empty")?,
                            (None, false, Some(url)) => {
                                queues._entry(key).or_default();

                                let input = Self::create_track(url).await?;

                                let handle = guard.play_source(input);
                                handles.insert(key, handle);

                                "playing"
                            },
                            (None, true, None) => unreachable!("found handle without queue"),
                            (None, true, Some(_)) =>
                                unreachable!("found handle and url without queue"),
                            (Some(mut vec), false, None) => {
                                if vec.len() == 0 {
                                    Err("queue is empty")?
                                }

                                let input = vec.remove(0);

                                let handle = guard.play_source(input);
                                handles.insert(key, handle);

                                "playing"
                            },
                            (Some(mut vec), false, Some(url)) => {
                                let input = Self::create_track(url).await?;

                                vec.push(input);

                                let input = vec.remove(0);

                                let handle = guard.play_source(input);
                                handles.insert(key, handle);

                                "playing"
                            },
                            (Some(_), true, None) => Err("now playing")?,
                            (Some(mut vec), true, Some(url)) => {
                                let input = Self::create_track(url).await?;

                                vec.push(input);

                                "queued"
                            },
                        }
                    },
                    Skip(kind) => {
                        let key = Caller::from_registry()
                            .send(GetCall { guild })
                            .await
                            .expect("failed sending")
                            .ok_or("not calling")?
                            .pipe(|a| Arc::as_ptr(&a))
                            .pipe(|p| p as usize);

                        let queue_opt = queues.get_mut(&key);
                        let queue = match queue_opt {
                            Some(mut vec) => vec,
                            None => Err("queue is empty")?,
                        };

                        use SkipKind::*;
                        match kind {
                            Index(index) => {
                                if !(0..queue.len()).contains(&(index as usize)) {
                                    Err("out of bounds")?
                                }

                                queue.remove(index as usize);
                            },
                            Range(range) => {
                                let range =
                                    (range.0.map(|s| s as usize), range.1.map(|e| e as usize));
                                let max_items = match range.1 {
                                    Bound::Included(i) => i + 1,
                                    Bound::Excluded(i) => i,
                                    Bound::Unbounded => Err("out of bounds")?,
                                };

                                if queue.len() < max_items {
                                    Err("out of bounds")?
                                }

                                queue.drain(range).count();
                            },
                        }

                        "skipped"
                    },
                    Stop => {
                        let key = Caller::from_registry()
                            .send(GetCall { guild })
                            .await
                            .expect("failed sending")
                            .ok_or("not calling")?
                            .pipe(|a| Arc::as_ptr(&a))
                            .pipe(|p| p as usize);

                        match handles.get(&key) {
                            Some(h) => h
                                .stop()
                                .map_err(|e| e.to_string())?
                                .pipe(|()| handles.remove(&key))
                                .pipe(drop),
                            None => (),
                        }

                        queues.remove(&key);

                        "stopped"
                    },
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
                .send(GetCall { guild })
                .await
                .expect("failed sending")
                .map(|a| Arc::as_ptr(&a))
                .map(|p| p as usize)
        }
        .into_actor(self)
        .map(move |opt, _, _| {
            if let Some(key) = opt {
                let mut queue = queues._entry(key).or_default();

                queue.push(input);

                reply("queued", from)
            } else {
                reply_err("not calling", from)
            }
        })
        .wait(ctx);
    }
}
impl Supervised for Connector {}
impl ArbiterService for Connector {}

pub struct CallAction {
    pub kind: CallActionKind,
    pub from: MessageRef,
    pub guild: u64,
}
pub enum CallActionKind {
    Join { channel: u64 },
    Leave,
    Play { url: Option<String> },
    Skip(SkipKind),
    Stop,
}
pub enum SkipKind {
    Index(u32),
    Range((Bound<u32>, Bound<u32>)),
}
impl Message for CallAction {
    type Result = ();
}

pub struct ControlAction {
    pub kind: ControlActionKind,
    pub from: MessageRef,
    pub guild: u64,
}
pub enum ControlActionKind {
    Queue { url: String },
    Pause,
    Resume,
    Loop(LoopKind),
    Shuffle,
    Volume { percent: f32 },
    VolumeCurrent { percent: f32 },
}
pub enum LoopKind {
    Index(u32),
    Range((Bound<u32>, Bound<u32>)),
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
impl Handler<DoCall> for Caller {
    type Result = Option<Arc<Mutex<Call>>>;

    fn handle(&mut self, DoCall { guild }: DoCall, ctx: &mut Self::Context) -> Self::Result {
        if let None = self.handle(GetCall { guild }, ctx) {
            self.songbird.get_or_insert(guild.into()).pipe(Some)
        } else {
            None
        }
    }
}
impl Handler<GetCall> for Caller {
    type Result = Option<Arc<Mutex<Call>>>;

    fn handle(&mut self, GetCall { guild }: GetCall, ctx: &mut Self::Context) -> Self::Result {
        self.songbird.get(guild)
    }
}
impl Handler<DeleteCall> for Caller {
    type Result = ResponseFuture<Result<Option<usize>, String>>;

    fn handle(&mut self, DeleteCall { guild }: DeleteCall, _: &mut Self::Context) -> Self::Result {
        let songbird = self.songbird.clone();

        async move {
            let key = songbird
                .get(guild)
                .map(|a| Arc::as_ptr(&a))
                .map(|p| p as usize);

            match songbird.remove(guild).await {
                Ok(o) => Ok(Some(key.expect("why cannot get key?"))),
                Err(e) => match e {
                    JoinError::NoCall => Ok(None),
                    e => Err(e.to_string()),
                },
            }
        }
        .pipe(Box::pin)
    }
}
impl Supervised for Caller {}
impl ArbiterService for Caller {}

struct DoCall {
    guild: u64,
}
impl Message for DoCall {
    type Result = Option<Arc<Mutex<Call>>>;
}
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
    type Result = Result<Option<usize>, String>;
}
