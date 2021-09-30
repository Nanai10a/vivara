use alloc::collections::VecDeque;
use alloc::sync::Arc;
use core::fmt::Display;
use core::ops::Bound;
use core::time::Duration;
use std::thread::spawn;

use actix::prelude::{
    Actor, ArbiterService, Context, ContextFutureSpawner, Handler, Message, ResponseFuture,
    Supervised, WrapFuture,
};
use dashmap::DashMap;
use futures_util::StreamExt;
use rand::seq::SliceRandom;
use rand::thread_rng;
use songbird::error::JoinError;
use songbird::id::{ChannelId, GuildId};
use songbird::input::cached::Memory;
use songbird::input::{ytdl, Input};
use songbird::tracks::{LoopState, PlayMode, TrackHandle, TrackState};
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
    default_volumes: Arc<DashMap<u64, f32>>,
    history: Arc<DashMap<u64, Vec<TrackInfo>>>,
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

    fn try_get_handle(call: &Call) -> Result<TrackHandle, String> {
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
            default_volumes: DashMap::new().pipe(Arc::new),
            history: DashMap::new().pipe(Arc::new),
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
        let default_volumes = self.default_volumes.clone();

        async move {
            use CallActionKind::*;
            let result = match kind {
                Join { channel } => Self::join(songbird, default_volumes, guild, channel).await,
                Leave => Self::leave(songbird, default_volumes, guild).await,
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
        default_volumes: Arc<DashMap<u64, f32>>,
        guild: impl Into<GuildId>,
        channel: impl Into<ChannelId>,
    ) -> StringResult {
        let guild = guild.into();
        let channel = channel.into();

        Self::_join(songbird, default_volumes, guild, channel).await
    }

    async fn _join(
        songbird: Arc<Songbird>,
        default_volumes: Arc<DashMap<u64, f32>>,
        guild: GuildId,
        channel: ChannelId,
    ) -> StringResult {
        let _: Option<()> = try {
            let current = songbird.get(guild)?.lock().await.current_channel()?;
            if current == channel.into() {
                return Err("already joined".to_string());
            }
        };

        if let (_, Err(e)) = songbird.join(guild, channel).await {
            return Err(e.to_string());
        }

        if default_volumes.insert(guild.0, 1.0).is_some() {
            unreachable!("must empty value");
        }

        Ok("joined".to_string())
    }

    async fn leave(
        songbird: Arc<Songbird>,
        default_volumes: Arc<DashMap<u64, f32>>,
        guild: impl Into<GuildId>,
    ) -> StringResult {
        let guild = guild.into();

        Self::_leave(songbird, default_volumes, guild).await
    }

    async fn _leave(
        songbird: Arc<Songbird>,
        default_volumes: Arc<DashMap<u64, f32>>,
        guild: GuildId,
    ) -> StringResult {
        if let Err(e) = songbird.remove(guild).await {
            return Err(e.to_string());
        }

        if default_volumes.remove(&guild.0).is_none() {
            unreachable!("must remove value");
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
            Index(index) =>
                (move |deq: &mut VecDeque<_>| deq.remove(index).is_some()).pipe(Either::Left),
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

    async fn _fix(_: Arc<Songbird>, _: GuildId) -> StringResult {
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
        let songbird = self.songbird.clone();
        let default_volumes = self.default_volumes.clone();

        async move {
            use ControlActionKind::*;
            let result = match kind {
                Enqueue { url } => Self::enqueue(songbird, default_volumes, guild, url).await,
                Pause => Self::pause(songbird, guild).await,
                Resume => Self::resume(songbird, guild).await,
                Loop => Self::r#loop(songbird, guild).await,
                Shuffle => Self::shuffle(songbird, guild).await,
                Volume {
                    percent,
                    current_only,
                } => Self::volume(songbird, default_volumes, guild, percent, current_only).await,
            };

            match result {
                Ok(o) => reply(o, from),
                Err(e) => reply(e, from),
            }
        }
        .into_actor(self)
        .spawn(ctx);
    }
}
impl Connector {
    async fn enqueue(
        songbird: Arc<Songbird>,
        default_volumes: Arc<DashMap<u64, f32>>,
        guild: impl Into<GuildId>,
        url: String,
    ) -> StringResult {
        let guild = guild.into();

        Self::_enqueue(songbird, default_volumes, guild, url).await
    }

    async fn _enqueue(
        songbird: Arc<Songbird>,
        default_volumes: Arc<DashMap<u64, f32>>,
        guild: GuildId,
        url: String,
    ) -> StringResult {
        let call = Self::try_get_call(&songbird, guild)?;
        let default_volume = *default_volumes.get(&guild.0).expect("must get value");

        let source = ytdl(url).await.map_err(|e| e.to_string())?;
        let (track, handle) = create_player(source);
        handle
            .set_volume(default_volume)
            .map_err(|e| e.to_string())?;

        call.lock().await.enqueue(track);

        "enqueued".to_string().pipe(Ok)
    }

    async fn pause(songbird: Arc<Songbird>, guild: impl Into<GuildId>) -> StringResult {
        let guild = guild.into();

        Self::_pause(songbird, guild).await
    }

    async fn _pause(songbird: Arc<Songbird>, guild: GuildId) -> StringResult {
        let call = Self::try_get_call(&songbird, guild)?;
        let guard = call.lock().await;

        let handle = Self::try_get_handle(&guard)?;

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

        let handle = Self::try_get_handle(&guard)?;

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

        let handle = Self::try_get_handle(&guard)?;
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

    async fn _shuffle(songbird: Arc<Songbird>, guild: GuildId) -> StringResult {
        let call = Self::try_get_call(&songbird, guild)?;
        let guard = call.lock().await;

        guard
            .queue()
            .modify_queue(|deq| deq.make_contiguous().shuffle(&mut thread_rng()));

        "shuffled".to_string().pipe(Ok)
    }

    async fn volume(
        songbird: Arc<Songbird>,
        default_volumes: Arc<DashMap<u64, f32>>,
        guild: impl Into<GuildId>,
        volume: f32,
        current_only: bool,
    ) -> StringResult {
        let guild = guild.into();

        Self::_volume(songbird, default_volumes, guild, volume, current_only).await
    }

    async fn _volume(
        songbird: Arc<Songbird>,
        default_volumes: Arc<DashMap<u64, f32>>,
        guild: GuildId,
        volume: f32,
        current_only: bool,
    ) -> StringResult {
        let call = Self::try_get_call(&songbird, guild)?;
        let guard = call.lock().await;

        if current_only {
            let handle = Self::try_get_handle(&guard)?;
            handle.set_volume(volume).map_err(|e| e.to_string())?;

            "changed volume"
        } else {
            *default_volumes.get_mut(&guild.0).expect("must get value") = volume;

            let results = guard.queue().modify_queue(|deq| {
                deq.iter()
                    .map(|h| h.set_volume(volume))
                    .filter_map(|r| r.err())
                    .map(|e| e.to_string())
                    .collect::<Vec<_>>()
            });

            if !results.is_empty() {
                let mut buf = String::new();
                results
                    .into_iter()
                    .enumerate()
                    .for_each(|(i, e)| buf += &format!("{}: {}", i, e));

                return Err(buf);
            }

            "changed default volume"
        }
        .to_string()
        .pipe(Ok)
    }
}
impl Handler<GetCurrentStatus> for Connector {
    type Result = ResponseFuture<Result<CurrentStatus, String>>;

    fn handle(
        &mut self,
        GetCurrentStatus { guild }: GetCurrentStatus,
        _: &mut Self::Context,
    ) -> Self::Result {
        let songbird = self.songbird.clone();

        async move {
            let result: Result<TrackStatus, String> = try {
                let call = Self::try_get_call(&songbird, guild.into())?;
                let guard = call.lock().await;

                let handle = Self::try_get_handle(&guard)?;
                handle.get_info().await.map_err(|e| e.to_string())?.into()
            };

            result.map(|current_track| CurrentStatus { current_track })
        }
        .pipe(Box::pin)
    }
}
impl Handler<GetQueueStatus> for Connector {
    type Result = ResponseFuture<Result<QueueStatus, String>>;

    fn handle(
        &mut self,
        GetQueueStatus { guild, page }: GetQueueStatus,
        _: &mut Self::Context,
    ) -> Self::Result {
        let songbird = self.songbird.clone();

        async move {
            if page == 0 {
                return "cannot specify page under 1".to_string().pipe(Err);
            }

            let result: Result<_, String> = try {
                let call = Self::try_get_call(&songbird, guild.into())?;
                let guard = call.lock().await;

                let mut queue = guard
                    .queue()
                    .current_queue()
                    .into_iter()
                    .enumerate()
                    .collect::<Vec<_>>();

                const ITEMS: usize = 10;
                let start = ITEMS * (page - 1);
                let mut end = ITEMS * page;
                if start > queue.len() {
                    return "out of bounds".to_string().pipe(Err);
                }
                if end > queue.len() {
                    end = queue.len();
                }
                let paging = start..end;

                let mut ok_vec = vec![];
                let mut err_vec = vec![];
                for (i, h) in queue.drain(paging) {
                    match h.get_info().await {
                        Ok(ts) => ok_vec.push((i, ts.into())),
                        Err(e) => err_vec.push(e.to_string()),
                    }
                }
                (ok_vec, err_vec)
            };

            let (oks, errs) = match result {
                Ok(o) => o,
                Err(e) => return Err(e),
            };

            if errs.is_empty() {
                let mut buf = String::new();
                errs.into_iter()
                    .enumerate()
                    .for_each(|(i, e)| buf += &format!("{}: {}", i, e));

                return Err(buf);
            }

            QueueStatus { tracks: oks }.pipe(Ok)
        }
        .pipe(Box::pin)
    }
}
impl Handler<GetHistoryStatus> for Connector {
    type Result = Result<HistoryStatus, String>;

    fn handle(
        &mut self,
        GetHistoryStatus { guild, page }: GetHistoryStatus,
        _: &mut Self::Context,
    ) -> Self::Result {
        self.history
            .get(&guild)
            .map(|v| HistoryStatus {
                history: v.iter().cloned().enumerate().collect(),
            })
            .ok_or_else(|| "no history".to_string())
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
pub struct CurrentStatus {
    pub current_track: TrackStatus,
}
impl Message for GetCurrentStatus {
    type Result = Result<CurrentStatus, String>;
}

pub struct GetQueueStatus {
    pub guild: u64,
    pub page: usize,
}
pub struct QueueStatus {
    pub tracks: Vec<(usize, TrackStatus)>,
}
impl Message for GetQueueStatus {
    type Result = Result<QueueStatus, String>;
}
pub struct GetHistoryStatus {
    pub guild: u64,
    pub page: usize,
}
pub struct HistoryStatus {
    pub history: Vec<(usize, TrackInfo)>,
}
impl Message for GetHistoryStatus {
    type Result = Result<HistoryStatus, String>;
}

pub struct TrackStatus {
    pub mode: TrackMode,
    pub volume: f32,
    pub position: Duration,
    pub total: Duration,
    pub loops: TrackLoop,
}
pub enum TrackMode {
    Play,
    Pause,
    Stop,
    End,
}
impl Display for TrackMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use TrackMode::*;
        let s = match self {
            Play => "Play",
            Pause => "Pause",
            Stop => "Stop",
            End => "End",
        };

        write!(f, "{}", s)
    }
}
pub enum TrackLoop {
    Infinite,
    Finite(usize),
}
impl Display for TrackLoop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use TrackLoop::*;
        let s = match self {
            Infinite => "Infinite".to_string(),
            Finite(t) => format!("Finite - until {} times", t),
        };

        write!(f, "{}", s)
    }
}
impl From<TrackState> for TrackStatus {
    fn from(
        TrackState {
            playing,
            volume,
            position,
            play_time,
            loops,
        }: TrackState,
    ) -> Self {
        TrackStatus {
            mode: playing.into(),
            volume,
            position,
            total: play_time,
            loops: loops.into(),
        }
    }
}
impl From<PlayMode> for TrackMode {
    fn from(mode: PlayMode) -> Self {
        use PlayMode::*;
        match mode {
            Play => TrackMode::Play,
            Pause => TrackMode::Pause,
            Stop => TrackMode::Stop,
            End => TrackMode::End,
            _ => unreachable!("forgotten pattern"),
        }
    }
}
impl From<LoopState> for TrackLoop {
    fn from(state: LoopState) -> Self {
        use LoopState::*;
        match state {
            Infinite => TrackLoop::Infinite,
            Finite(n) => TrackLoop::Finite(n),
        }
    }
}
#[derive(Clone)]
pub struct TrackInfo {
    pub url: String,
}
