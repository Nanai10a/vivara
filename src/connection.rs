use alloc::sync::Arc;
use std::thread::spawn;

use actix::fut::wrap_future;
use actix::prelude::*;
use dashmap::DashMap;
use futures_util::StreamExt;
use songbird::input::cached::Memory;
use songbird::input::Input;
use songbird::tracks::TrackHandle;
use songbird::{Call, Songbird};
use tokio::sync::Mutex;
use twilight_gateway::cluster::Events;
use url::Url;

use crate::gateway;
use crate::util::{reply, reply_err, token, Pipe};

#[derive(Default)]
pub struct UrlQueue;
impl Actor for UrlQueue {
    type Context = Context<Self>;
}
impl Handler<UrlQueueData> for UrlQueue {
    type Result = ();

    fn handle(
        &mut self,
        UrlQueueData { url, from, guild }: UrlQueueData,
        ctx: &mut Self::Context,
    ) -> Self::Result {
        songbird::ytdl(url)
            .pipe(wrap_future::<_, Self>)
            .map(move |res, _, _| {
                let input = match res {
                    Ok(o) => o,
                    Err(e) => return reply_err(e, from),
                };

                let input = match Memory::new(input).map(|m| m.try_into()).flatten() {
                    Ok(o) => o,
                    Err(e) => return reply_err(e, from),
                };

                Connector::from_registry().do_send(QueueData { guild, from, input });
            })
            .wait(ctx)
    }
}
impl Supervised for UrlQueue {}
impl ArbiterService for UrlQueue {}

pub struct UrlQueueData {
    pub from: gateway::MsgRef,
    pub guild: u64,
    pub url: Url,
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
                .unwrap();
            let key = Arc::as_ptr(&arc) as usize;
            let mut guard = arc.lock().await;

            use ActionKind::*;
            match kind {
                Join { channel } => {
                    let join = match guard.join(channel.into()).await {
                        Ok(o) => o,
                        Err(e) => return reply_err(e, from),
                    };

                    drop(guard);

                    if let Err(e) = join.await {
                        return reply_err(e, from);
                    }

                    reply("joined", from);
                },
                Play => {
                    let input = match queues.get_mut(&key) {
                        None => return reply_err("queue is empty", from),
                        Some(mut vec) => match vec.len() {
                            0 => return reply_err("queue is empty", from),
                            _ => vec.remove(0),
                        },
                    };

                    let handle = guard.play_source(input);
                    handles.insert(key, handle);

                    reply("playing", from);
                },
                Stop => {
                    let handle = match handles.get(&key) {
                        Some(h) => h,
                        None => return reply_err("not playing", from),
                    };

                    if let Err(e) = handle.stop() {
                        return reply_err(e, from);
                    }

                    reply("stopped", from);
                },
                Leave => {
                    if let Err(e) = guard.leave().await {
                        return reply_err(e, from);
                    }

                    reply("leaved", from);
                },
            }
        }
        .pipe(wrap_future::<_, Self>)
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
                .pipe(|r| match r {
                    Ok(o) => (&o).pipe(Arc::as_ptr).pipe(|p| p as usize).pipe(Some),
                    Err(e) => gateway::get_reply_recipient()
                        .do_send(gateway::Reply {
                            msg: e.to_string(),
                            kind: gateway::Kind::Err,
                            to: from,
                        })
                        .unwrap()
                        .pipe(|()| None),
                })
        }
        .pipe(wrap_future::<_, Self>)
        .map(move |opt, _, _| {
            if let Some(key) = opt {
                let mut queue = match queues.get_mut(&key) {
                    Some(v) => v,
                    None => {
                        queues.insert(key, vec![]);
                        queues.get_mut(&key).unwrap()
                    },
                };

                queue.push(input);

                reply("queued", from)
            }
        })
        .wait(ctx);
    }
}
impl Supervised for Connector {}
impl ArbiterService for Connector {}

#[non_exhaustive]
pub struct Action {
    pub kind: ActionKind,
    pub from: gateway::MsgRef,
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
    from: gateway::MsgRef,
    guild: u64,
    input: Input,
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
            match twilight_gateway::Cluster::new(
                token::<String>(),
                twilight_gateway::Intents::GUILD_VOICE_STATES,
            )
            .await
            {
                Ok(t) => break t,
                Err(e) => tracing::warn!("initialize error: {}", e),
            }
        };

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

        (Songbird::twilight(cluster, user_id), events)
    }
}
impl Default for Caller {
    fn default() -> Self {
        let handle = tokio::runtime::Handle::current();
        let (songbird, events) = spawn(move || handle.block_on(async { Self::init().await }))
            .join()
            .unwrap();

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
        let mut events = self.events.take().unwrap();

        async move {
            while let Some((id, event)) = events.next().await {
                tracing::trace!("id: {} | event: {:?}", id, event);
                arc.process(&event).await;
            }
        }
        .pipe(wrap_future::<_, Self>)
        .spawn(ctx);
    }
}
impl Handler<CallRequest> for Caller {
    type Result = Arc<Mutex<Call>>;

    fn handle(
        &mut self,
        CallRequest { guild }: CallRequest,
        _: &mut Self::Context,
    ) -> Self::Result {
        self.songbird.get_or_insert(guild.into())
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
