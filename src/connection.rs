use alloc::collections::VecDeque;
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
use songbird::id::{ChannelId, GuildId};
use songbird::input::cached::Memory;
use songbird::input::{ytdl, Input};
use songbird::tracks::TrackHandle;
use songbird::{create_player, Call, Songbird};
use tokio::sync::Mutex;
use twilight_gateway::cluster::Events;
use twilight_gateway::{Cluster, Intents};

use crate::gateway::MessageRef;
use crate::util::{reply, reply_err, token, Pipe};

pub struct Connector {
    songbird: Arc<Songbird>,
    _events: Option<Events>,
}
impl Connector {
    // FIXME: do generalize
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

    async fn create_track(url: String) -> Result<Input, String> {
        let source = songbird::ytdl(url).await.map_err(|e| e.to_string())?;

        let input = Memory::new(source)
            .map_err(|e| e.to_string())?
            .pipe(|m| m.try_into())
            .map_err(|e: songbird::input::error::Error| e.to_string())?;

        Ok(input)
    }
}
impl Default for Connector {
    fn default() -> Self {
        let handle = tokio::runtime::Handle::current();
        let (songbird, events) = spawn(move || handle.block_on(async { Self::init().await }))
            .join()
            .expect("failed joining thread");

        Self {
            songbird: Arc::new(songbird),
            _events: events.pipe(Some),
        }
    }
}
impl Actor for Connector {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        let arc = self.songbird.clone();
        let mut events = self._events.take().expect("must be found value");

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
impl Handler<CallAction> for Connector {
    type Result = ();

    fn handle(
        &mut self,
        CallAction { kind, from, guild }: CallAction,
        ctx: &mut Self::Context,
    ) -> Self::Result {
        let songbird = self.songbird.clone();

        async move {
            use CallActionKind::*;
            let result = match kind {
                Join { channel } => Self::join(songbird, guild, channel).await,
                Leave => Self::leave(songbird, guild).await,
                Slide { from, to } => Self::slide(songbird, guild, from, to).await,
                Drop { kind } => Self::drop(songbird, guild, kind).await,
                Fix => Self::fix(songbird, guild).await,
                Stop => Self::stop(songbird, guild).await,
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
impl Connector {
    async fn join(
        songbird: Arc<Songbird>,
        guild: impl Into<GuildId>,
        channel: impl Into<ChannelId>,
    ) -> Result<String, String> {
        let guild = guild.into();
        let channel = channel.into();

        Self::_join(songbird, guild, channel).await
    }

    async fn _join(
        songbird: Arc<Songbird>,
        guild: GuildId,
        channel: ChannelId,
    ) -> Result<String, String> {
        let _: Option<()> = try {
            let current = songbird.get(guild)?.lock().await.current_channel()?;
            if current == channel.into() {
                return Err("already joined".to_string());
            }
        };

        if let (_, Err(e)) = songbird.join(guild, channel).await {
            return Err(e.to_string());
        }

        Ok("joined".to_string())
    }

    async fn leave(songbird: Arc<Songbird>, guild: impl Into<GuildId>) -> Result<String, String> {
        let guild = guild.into();

        Self::_leave(songbird, guild).await
    }

    async fn _leave(songbird: Arc<Songbird>, guild: GuildId) -> Result<String, String> {
        if let Err(e) = songbird.remove(guild).await {
            return Err(e.to_string());
        }

        Ok("leaved".to_string())
    }

    async fn slide(
        songbird: Arc<Songbird>,
        guild: impl Into<GuildId>,
        from: usize,
        to: usize,
    ) -> Result<String, String> {
        let guild = guild.into();

        Self::_slide(songbird, guild, from, to).await
    }

    async fn _slide(
        songbird: Arc<Songbird>,
        guild: GuildId,
        from: usize,
        to: usize,
    ) -> Result<String, String> {
        let call = match songbird.get(guild) {
            Some(c) => c,
            None => return Err(JoinError::NoCall.to_string()),
        };

        let result = call.lock().await.queue().modify_queue(|deq| {
            if deq.len() > from {
                return false;
            }

            let target = deq.remove(from).expect("must removable");
            let afters = deq.drain(to..).collect::<Vec<_>>();

            deq.push_back(target);
            deq.append(&mut afters.into());

            true
        });

        match result {
            true => "slided".to_string().pipe(Ok),
            false => "out of bounds".to_string().pipe(Err),
        }
    }

    async fn drop(
        songbird: Arc<Songbird>,
        guild: impl Into<GuildId>,
        kind: DropKind,
    ) -> Result<String, String> {
        let guild = guild.into();

        Self::_drop(songbird, guild, kind).await
    }

    async fn _drop(
        songbird: Arc<Songbird>,
        guild: GuildId,
        kind: DropKind,
    ) -> Result<String, String> {
        enum Either<L, R> {
            Left(L),
            Right(R),
        }
        impl<L, R, I, O> FnOnce<I> for Either<L, R>
        where
            L: FnOnce<I, Output = O>,
            R: FnOnce<I, Output = O>,
        {
            type Output = O;

            extern "rust-call" fn call_once(self, i: I) -> Self::Output {
                match self {
                    Self::Left(l) => l.call_once(i),
                    Self::Right(r) => r.call_once(i),
                }
            }
        }

        let call = match songbird.get(guild) {
            Some(c) => c,
            None => return Err(JoinError::NoCall.to_string()),
        };

        use DropKind::*;
        let func = match kind {
            Index(index) => (move |deq: &mut VecDeque<_>| match deq.remove(index) {
                Some(_) => true,
                None => false,
            })
            .pipe(Either::Left),
            Range(range) => (move |deq: &mut VecDeque<_>| {
                use Bound::*;
                let end = match range.1 {
                    Unbounded => return false,
                    Included(e) => e + 1,
                    Excluded(e) => e,
                };

                if end > deq.len() {
                    return false;
                }

                deq.drain(range).for_each(drop);

                true
            })
            .pipe(Either::Right),
        };

        let result = call.lock().await.queue().modify_queue(func);

        match result {
            true => "dropped".to_string().pipe(Ok),
            false => "out of bounds".to_string().pipe(Err),
        }
    }

    async fn fix(songbird: Arc<Songbird>, guild: impl Into<GuildId>) -> Result<String, String> {
        let guild = guild.into();

        Self::_fix(songbird, guild).await
    }

    async fn _fix(songbird: Arc<Songbird>, guild: GuildId) -> Result<String, String> {
        // TODO
        "no operated".to_string().pipe(Ok)
    }

    async fn stop(songbird: Arc<Songbird>, guild: impl Into<GuildId>) -> Result<String, String> {
        let guild = guild.into();

        Self::_stop(songbird, guild).await
    }

    async fn _stop(songbird: Arc<Songbird>, guild: GuildId) -> Result<String, String> {
        let call = match songbird.get(guild) {
            Some(c) => c,
            None => return Err(JoinError::NoCall.to_string()),
        };

        call.lock().await.queue().stop();

        Ok("stopped".to_string())
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
    Slide { from: usize, to: usize },
    Drop { kind: DropKind },
    Fix,
    Stop,
}
pub enum DropKind {
    Index(usize),
    Range((Bound<usize>, Bound<usize>)),
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
    Enqueue { url: String },
    Pause,
    Resume,
    Loop,
    Shuffle,
    Volume { percent: f32 },
    VolumeCurrent { percent: f32 },
}
impl Message for ControlAction {
    type Result = ();
}

pub struct GetCurrentStatus {
    pub guild: u64,
}
pub struct CurrentStatus {}
impl Message for GetCurrentStatus {
    type Result = CurrentStatus;
}

pub struct GetQueueStatus {
    pub guild: u64,
    pub page: u32,
}
pub struct QueueStatus {}
impl Message for GetQueueStatus {
    type Result = QueueStatus;
}
pub struct GetHistoryStatus {
    pub guild: u64,
    pub page: u32,
}
pub struct HistoryStatus {}
impl Message for GetHistoryStatus {
    type Result = HistoryStatus;
}

struct QueueData {
    input: Input,
    from: MessageRef,
    guild: u64,
}
impl Message for QueueData {
    type Result = ();
}

