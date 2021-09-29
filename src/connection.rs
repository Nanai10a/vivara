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
use songbird::tracks::{LoopState, TrackHandle};
use songbird::{create_player, Call, Songbird};
use tokio::sync::Mutex;
use twilight_gateway::cluster::Events;
use twilight_gateway::{Cluster, Intents};

use crate::gateway::MessageRef;
use crate::util::{reply, reply_err, token, Pipe};

type StringResult = Result<String, String>;

pub struct Connector {
    songbird: Arc<Songbird>,
    _events: Option<Events>,
}
impl Connector {
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

    fn try_get_call(songbird: &Arc<Songbird>, guild: GuildId) -> Result<Arc<Mutex<Call>>, String> {
        match songbird.get(guild) {
            Some(call) => Ok(call),
            None => Err(JoinError::NoCall.to_string()),
        }
    }

    fn try_get_handle(call: &Call, guild: GuildId) -> Result<TrackHandle, String> {
        match call.queue().current() {
            Some(th) => th.pipe(Ok),
            None => JoinError::NoCall.to_string().pipe(Err),
        }
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
    ) -> StringResult {
        let guild = guild.into();
        let channel = channel.into();

        Self::_join(songbird, guild, channel).await
    }

    async fn _join(songbird: Arc<Songbird>, guild: GuildId, channel: ChannelId) -> StringResult {
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

    async fn leave(songbird: Arc<Songbird>, guild: impl Into<GuildId>) -> StringResult {
        let guild = guild.into();

        Self::_leave(songbird, guild).await
    }

    async fn _leave(songbird: Arc<Songbird>, guild: GuildId) -> StringResult {
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
    ) -> StringResult {
        let guild = guild.into();

        Self::_slide(songbird, guild, from, to).await
    }

    async fn _slide(
        songbird: Arc<Songbird>,
        guild: GuildId,
        from: usize,
        to: usize,
    ) -> StringResult {
        let call = Self::try_get_call(&songbird, guild)?;

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
    ) -> StringResult {
        let guild = guild.into();

        Self::_drop(songbird, guild, kind).await
    }

    async fn _drop(songbird: Arc<Songbird>, guild: GuildId, kind: DropKind) -> StringResult {
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

        let call = Self::try_get_call(&songbird, guild)?;

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

    async fn fix(songbird: Arc<Songbird>, guild: impl Into<GuildId>) -> StringResult {
        let guild = guild.into();

        Self::_fix(songbird, guild).await
    }

    async fn _fix(songbird: Arc<Songbird>, guild: GuildId) -> StringResult {
        // TODO
        "no operated".to_string().pipe(Ok)
    }

    async fn stop(songbird: Arc<Songbird>, guild: impl Into<GuildId>) -> StringResult {
        let guild = guild.into();

        Self::_stop(songbird, guild).await
    }

    async fn _stop(songbird: Arc<Songbird>, guild: GuildId) -> StringResult {
        let call = Self::try_get_call(&songbird, guild)?;

        call.lock().await.queue().stop();

        Ok("stopped".to_string())
    }
}
impl Handler<ControlAction> for Connector {
    type Result = ();

    fn handle(
        &mut self,
        ControlAction { kind, from, guild }: ControlAction,
        ctx: &mut Self::Context,
    ) -> Self::Result {
        unimplemented!()
    }
}
impl Connector {
    async fn enqueue(
        songbird: Arc<Songbird>,
        guild: impl Into<GuildId>,
        url: String,
    ) -> StringResult {
        let guild = guild.into();

        Self::_enqueue(songbird, guild, url).await
    }

    async fn _enqueue(songbird: Arc<Songbird>, guild: GuildId, url: String) -> StringResult {
        let call = Self::try_get_call(&songbird, guild)?;

        let source = ytdl(url).await.map_err(|e| e.to_string())?;

        call.lock().await.enqueue_source(source);

        "enqueued".to_string().pipe(Ok)
    }

    async fn pause(songbird: Arc<Songbird>, guild: impl Into<GuildId>) -> StringResult {
        let guild = guild.into();

        Self::_pause(songbird, guild).await
    }

    async fn _pause(songbird: Arc<Songbird>, guild: GuildId) -> StringResult {
        let call = Self::try_get_call(&songbird, guild)?;
        let guard = call.lock().await;

        let handle = Self::try_get_handle(&guard, guild)?;

        handle.pause().map_err(|e| e.to_string())?;

        "paused".to_string().pipe(Ok)
    }

    async fn resume(songbird: Arc<Songbird>, guild: impl Into<GuildId>) -> StringResult {
        let guild = guild.into();

        Self::_resume(songbird, guild).await
    }

    async fn _resume(songbird: Arc<Songbird>, guild: GuildId) -> StringResult {
        let call = Self::try_get_call(&songbird, guild)?;
        let guard = call.lock().await;

        let handle = Self::try_get_handle(&guard, guild)?;

        handle.play().map_err(|e| e.to_string())?;

        "resumed".to_string().pipe(Ok)
    }

    async fn r#loop(songbird: Arc<Songbird>, guild: impl Into<GuildId>) -> StringResult {
        let guild = guild.into();

        Self::_loop(songbird, guild).await
    }

    async fn _loop(songbird: Arc<Songbird>, guild: GuildId) -> StringResult {
        let call = Self::try_get_call(&songbird, guild)?;
        let guard = call.lock().await;

        let handle = Self::try_get_handle(&guard, guild)?;
        let state = handle.get_info().await.map_err(|e| e.to_string())?.loops;

        use LoopState::*;
        match state {
            Finite(0) => handle
                .enable_loop()
                .map_err(|e| e.to_string())
                .map(|()| "setted loop".to_string()),

            Finite(_) | Infinite => handle
                .disable_loop()
                .map_err(|e| e.to_string())
                .map(|()| "unsetted loop".to_string()),
        }
    }

    async fn shuffle(songbird: Arc<Songbird>, guild: impl Into<GuildId>) -> StringResult {
        let guild = guild.into();

        Self::_shuffle(songbird, guild).await
    }

    async fn _shuffle(songbird: Arc<Songbird>, guild: GuildId) -> StringResult { unimplemented!() }

    async fn volume(
        songbird: Arc<Songbird>,
        guild: impl Into<GuildId>,
        current_only: bool,
    ) -> StringResult {
        let guild = guild.into();

        Self::_volume(songbird, guild, current_only).await
    }

    async fn _volume(songbird: Arc<Songbird>, guild: GuildId, current_only: bool) -> StringResult {
        unimplemented!()
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
    Loop, // FIXME: uncomplete
    Shuffle,
    Volume { percent: f32, current_only: bool },
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
