use std::ops::Bound;

use actix::prelude::{Actor, ArbiterService, Context, Handler, Message, Supervised};
use actix::ResponseFuture;
use clap::{ArgGroup, Parser};
use url::Url;

use crate::connection::{
    CallAction, CallActionKind, Connector, ControlAction, ControlActionKind, CurrentStatus,
    DropKind, GetCurrentStatus, GetHistoryStatus, GetQueueStatus, HistoryStatus, QueueStatus,
    TrackInfo, TrackStatus,
};
use crate::gateway::{MessageRef, RawCommand};
use crate::util::{reply, reply_err, Pipe};

#[derive(Default)]
pub struct CommandParser;
impl Actor for CommandParser {
    type Context = Context<Self>;
}
impl Handler<RawCommand> for CommandParser {
    type Result = ();

    fn handle(
        &mut self,
        RawCommand {
            content,
            from,
            user: _,
            guild,
        }: RawCommand,
        _: &mut Self::Context,
    ) -> Self::Result {
        let result: Result<_, String> = try {
            let split = shell_words::split(&content).map_err(|e| e.to_string())?;

            match split.get(0).map(|s| s.as_str()) {
                Some("*v") => (),
                _ => return,
            }

            match guild {
                None => {
                    let PrivateCommandParser { cmd: _ } =
                        PrivateCommandParser::try_parse_from(split).map_err(|e| e.to_string())?;

                    unimplemented!();
                },
                Some(guild) => {
                    let GuildCommandParser { cmd } =
                        GuildCommandParser::try_parse_from(split).map_err(|e| e.to_string())?;

                    GuildCommandProcesser::from_registry()
                        .try_send(GuildCommandData { cmd, guild, from })
                        .expect("failed sending");
                },
            }
        };

        match result {
            Ok(o) => o,
            Err(e) => reply_err(e, from),
        }
    }
}
impl Supervised for CommandParser {}
impl ArbiterService for CommandParser {}

#[derive(Parser)]
struct GuildCommandParser {
    #[clap(subcommand)]
    cmd: GuildCommand,
}
#[derive(Parser)]
enum GuildCommand {
    Join {
        channel: u64,
    },
    Leave,
    Slide {
        from: usize,
        to: usize,
    },
    #[clap(group = ArgGroup::new("items").required(true))]
    Drop {
        #[clap(short = 'i', long, group = "items")]
        items: Option<usize>,
        #[clap(short = 'r', long, group = "items", parse(try_from_str = range_parser::parse))]
        range: Option<(Bound<usize>, Bound<usize>)>,
    },
    Fix,
    Stop,

    Enqueue {
        url: Url,
    },
    Pause,
    Resume,
    Loop,
    Shuffle,
    Volume {
        percent: f32,
    },
    VolumeCurrent {
        percent: f32,
    },

    ShowCurrent,
    ShowQueue {
        page: Option<usize>,
    },
    ShowHistory {
        page: Option<usize>,
    },
}

#[derive(Parser)]
struct PrivateCommandParser {
    #[clap(subcommand)]
    cmd: PrivateCommand,
}
#[derive(Parser)]
enum PrivateCommand {}

pub struct GuildCommandData {
    cmd: GuildCommand,
    from: MessageRef,
    guild: u64,
}
impl Message for GuildCommandData {
    type Result = ();
}

#[derive(Default)]
pub struct GuildCommandProcesser;
impl Actor for GuildCommandProcesser {
    type Context = Context<Self>;
}
impl Handler<GuildCommandData> for GuildCommandProcesser {
    type Result = ResponseFuture<()>;

    fn handle(
        &mut self,
        GuildCommandData { cmd, from, guild }: GuildCommandData,
        _: &mut Self::Context,
    ) -> Self::Result {
        async move {
            use GuildCommand::*;
            match cmd {
                Join { channel } => Connector::from_registry()
                    .try_send(CallAction {
                        kind: CallActionKind::Join { channel },
                        from,
                        guild,
                    })
                    .expect("failed sending"),
                Leave => Connector::from_registry()
                    .try_send(CallAction {
                        kind: CallActionKind::Leave,
                        from,
                        guild,
                    })
                    .expect("failed sending"),
                Slide {
                    from: origin,
                    to: dest,
                } => Connector::from_registry()
                    .try_send(CallAction {
                        kind: CallActionKind::Slide {
                            from: origin,
                            to: dest,
                        },
                        from,
                        guild,
                    })
                    .expect("failed sending"),
                Drop { items, range } => {
                    let kind = match (items, range) {
                        (Some(n), None) => DropKind::Index(n),
                        (None, Some(r)) => DropKind::Range(r),
                        t => unreachable!("unexpected pattern: {:?}", t),
                    };

                    Connector::from_registry()
                        .try_send(CallAction {
                            kind: CallActionKind::Drop { kind },
                            from,
                            guild,
                        })
                        .expect("failed sending")
                },
                Fix => Connector::from_registry()
                    .try_send(CallAction {
                        kind: CallActionKind::Fix,
                        from,
                        guild,
                    })
                    .expect("failed sending"),
                Stop => Connector::from_registry()
                    .try_send(CallAction {
                        kind: CallActionKind::Stop,
                        from,
                        guild,
                    })
                    .expect("failed sending"),

                Enqueue { url } => Connector::from_registry()
                    .try_send(ControlAction {
                        kind: ControlActionKind::Enqueue {
                            url: url.to_string(),
                        },
                        from,
                        guild,
                    })
                    .expect("failed sending"),

                Pause => Connector::from_registry()
                    .try_send(ControlAction {
                        kind: ControlActionKind::Pause,
                        from,
                        guild,
                    })
                    .expect("failed sending"),
                Resume => Connector::from_registry()
                    .try_send(ControlAction {
                        kind: ControlActionKind::Resume,
                        from,
                        guild,
                    })
                    .expect("failed sending"),
                Loop => Connector::from_registry()
                    .try_send(ControlAction {
                        kind: ControlActionKind::Loop,
                        from,
                        guild,
                    })
                    .expect("failed sending"),
                Shuffle => Connector::from_registry()
                    .try_send(ControlAction {
                        kind: ControlActionKind::Shuffle,
                        from,
                        guild,
                    })
                    .expect("failed sending"),
                Volume { percent } => Connector::from_registry()
                    .try_send(ControlAction {
                        kind: ControlActionKind::Volume {
                            percent,
                            current_only: false,
                        },
                        from,
                        guild,
                    })
                    .expect("failed sending"),
                VolumeCurrent { percent } => Connector::from_registry()
                    .try_send(ControlAction {
                        kind: ControlActionKind::Volume {
                            percent,
                            current_only: true,
                        },
                        from,
                        guild,
                    })
                    .expect("failed sending"),

                ShowCurrent => Connector::from_registry()
                    .send(GetCurrentStatus { guild })
                    .await
                    .expect("failed sending")
                    .map_err(|e| reply_err(e, from))
                    .map(|CurrentStatus { current_track }| {
                        format!("current:\n{}", format_track_status(current_track))
                    })
                    .map(|msg| reply(msg, from))
                    .pipe(drop),
                ShowQueue { page } => Connector::from_registry()
                    .send(GetQueueStatus {
                        guild,
                        page: page.unwrap_or(1),
                    })
                    .await
                    .expect("failed sending")
                    .map_err(|e| reply_err(e, from))
                    .map(|QueueStatus { tracks }| {
                        let mut buf = String::new();
                        tracks.into_iter().for_each(|(i, ts)| {
                            buf += &format!("{}:\n{}\n\n", i, format_track_status(ts));
                        });
                        buf
                    })
                    .map(|msg| reply(msg, from))
                    .pipe(drop),
                ShowHistory { page } => Connector::from_registry()
                    .send(GetHistoryStatus {
                        guild,
                        page: page.unwrap_or(1),
                    })
                    .await
                    .expect("failed sending")
                    .map_err(|e| reply_err(e, from))
                    .map(|HistoryStatus { history }| {
                        let mut buf = String::new();
                        history.into_iter().for_each(|(idx, info)| {
                            buf += &format!("{}\n{}\n\n", idx, format_track_info(info))
                        });
                        buf
                    })
                    .map(|msg| reply(msg, from))
                    .pipe(drop),
            }
        }
        .pipe(Box::pin)
    }
}
impl Supervised for GuildCommandProcesser {}
impl ArbiterService for GuildCommandProcesser {}

fn format_track_status(
    TrackStatus {
        mode,
        volume,
        position,
        total,
        loops,
    }: TrackStatus,
) -> String {
    format!(
        "mode: {}\nvolume: {}\nposition: {}s\ntotal playing: {}s\nloop: {}",
        mode,
        volume,
        position.as_secs_f32(),
        total.as_secs_f32(),
        loops
    )
}

fn format_track_info(TrackInfo { url }: TrackInfo) -> String { format!("url: {}", url) }
