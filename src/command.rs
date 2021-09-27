use std::ops::Bound;

use actix::prelude::{Actor, ArbiterService, Context, Handler, Message, Supervised};
use clap::{ArgGroup, Clap};
use url::Url;

use crate::connection::{CallAction, CallActionKind, Connector, ControlAction, ControlActionKind};
use crate::gateway::{MessageRef, RawCommand};
use crate::util::reply_err;

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
            user,
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
                    let PrivateCommandParser { cmd } =
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

#[derive(Clap)]
struct GuildCommandParser {
    #[clap(subcommand)]
    cmd: GuildCommand,
}
#[derive(Clap)]
enum GuildCommand {
    Join {
        channel: u64,
    },
    Leave,

    Enqueue {
        url: Url,
    },

    ShowCurrent,
    ShowQueue {
        page: Option<u32>,
    },
    ShowHistory {
        page: Option<u32>,
    },

    Play {
        url: Option<Url>,
    },
    Pause,
    Resume,
    #[clap(group = ArgGroup::new("items").required(true))]
    Skip {
        #[clap(short = 'i', long, group = "items")]
        items: Option<usize>,
        #[clap(short = 'r', long, group = "items", parse(try_from_str = range_parser::parse))]
        range: Option<(Bound<usize>, Bound<usize>)>,
    },
    #[clap(group = ArgGroup::new("items").required(true))]
    Loop {
        #[clap(short = 'i', long, group = "items")]
        index: Option<usize>,
        #[clap(short = 'r', long, group = "items", parse(try_from_str = range_parser::parse))]
        range: Option<(Bound<usize>, Bound<usize>)>,
    },
    Shuffle,
    Volume {
        percent: f32,
    },
    VolumeCurrent {
        percent: f32,
    },
    Stop,
}

#[derive(Clap)]
struct PrivateCommandParser {
    #[clap(subcommand)]
    cmd: PrivateCommand,
}
#[derive(Clap)]
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
    type Result = ();

    fn handle(
        &mut self,
        GuildCommandData { cmd, from, guild }: GuildCommandData,
        _: &mut Self::Context,
    ) -> Self::Result {
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

            Enqueue { url } => Connector::from_registry()
                .try_send(ControlAction {
                    kind: ControlActionKind::Queue {
                        url: url.to_string(),
                    },
                    from,
                    guild,
                })
                .expect("failed sending"),

            ShowCurrent => unimplemented!(),
            ShowQueue { page } => unimplemented!(),
            ShowHistory { page } => unimplemented!(),

            Play { url } => Connector::from_registry()
                .try_send(CallAction {
                    kind: CallActionKind::Play {
                        url: url.map(|u| u.to_string()),
                    },
                    from,
                    guild,
                })
                .expect("failed sending"),
            Pause => unimplemented!(),
            Resume => unimplemented!(),
            Skip { items, range } => unimplemented!(),
            Loop { index, range } => unimplemented!(),
            Shuffle => unimplemented!(),
            Volume { percent } => unimplemented!(),
            VolumeCurrent { percent } => unimplemented!(),
            Stop => Connector::from_registry()
                .try_send(CallAction {
                    kind: CallActionKind::Stop,
                    from,
                    guild,
                })
                .expect("failed sending"),
        }
    }
}
impl Supervised for GuildCommandProcesser {}
impl ArbiterService for GuildCommandProcesser {}
