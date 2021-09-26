use actix::prelude::*;
use url::Url;

use crate::connection::{Action, ActionKind, Connector, UrlQueue, UrlQueueData};
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

            use clap::Clap;
            match guild {
                None => {
                    let CtrlCmdParser { cmd } =
                        CtrlCmdParser::try_parse_from(split).map_err(|e| e.to_string())?;

                    unimplemented!();
                },
                Some(guild) => {
                    let PlayCmdParser { cmd } =
                        PlayCmdParser::try_parse_from(split).map_err(|e| e.to_string())?;

                    PlayCommandProcesser::from_registry()
                        .try_send(PlayCommand { cmd, guild, from })
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

#[derive(clap::Clap)]
struct PlayCmdParser {
    #[clap(subcommand)]
    cmd: PlayCmd,
}

#[derive(clap::Clap)]
enum PlayCmd {
    #[clap(short_flag = 'Q')]
    Queue {
        #[clap(subcommand)]
        cmd: QueueCmd,
    },
    #[clap(short_flag = 'A')]
    Access {
        #[clap(subcommand)]
        cmd: AccessCmd,
    },
}

#[derive(clap::Clap)]
enum QueueCmd {
    #[clap(short_flag = 'u')]
    Url { url: Url },
}

#[derive(clap::Clap)]
enum AccessCmd {
    #[clap(short_flag = 'j')]
    Join { channel: u64 },
    #[clap(short_flag = 'p')]
    Play,
    #[clap(short_flag = 's')]
    Stop,
    #[clap(short_flag = 'l')]
    Leave,
}

#[derive(clap::Clap)]
struct CtrlCmdParser {
    #[clap(subcommand)]
    cmd: CtrlCmd,
}

#[derive(clap::Clap)]
enum CtrlCmd {}

pub struct PlayCommand {
    cmd: PlayCmd,
    from: MessageRef,
    guild: u64,
}
impl Message for PlayCommand {
    type Result = ();
}

#[derive(Default)]
pub struct PlayCommandProcesser;
impl Actor for PlayCommandProcesser {
    type Context = Context<Self>;
}
impl Handler<PlayCommand> for PlayCommandProcesser {
    type Result = ();

    fn handle(
        &mut self,
        PlayCommand { cmd, from, guild }: PlayCommand,
        _: &mut Self::Context,
    ) -> Self::Result {
        match cmd {
            PlayCmd::Queue { cmd } => match cmd {
                QueueCmd::Url { url } => UrlQueue::from_registry()
                    .try_send(UrlQueueData { url, from, guild })
                    .expect("failed sending"),
            },
            PlayCmd::Access { cmd } => {
                let kind = match cmd {
                    AccessCmd::Join { channel } => ActionKind::Join { channel },
                    AccessCmd::Play => ActionKind::Play,
                    AccessCmd::Stop => ActionKind::Stop,
                    AccessCmd::Leave => ActionKind::Leave,
                };

                Connector::from_registry()
                    .try_send(Action { kind, from, guild })
                    .expect("failed sending")
            },
        }
    }
}
impl Supervised for PlayCommandProcesser {}
impl ArbiterService for PlayCommandProcesser {}
