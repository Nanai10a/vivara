use actix::prelude::*;
use url::Url;

use crate::connection::{Action, ActionKind, Connector, Queuer, UrlQueueData};
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

#[derive(clap::Clap)]
struct GuildCommandParser {
    #[clap(subcommand)]
    cmd: GuildCommand,
}
#[derive(clap::Clap)]
enum GuildCommand {
    #[clap(short_flag = 'Q')]
    Queue {
        #[clap(subcommand)]
        cmd: QueueCommand,
    },
    #[clap(short_flag = 'C')]
    Control {
        #[clap(subcommand)]
        cmd: ControlCommand,
    },
}
#[derive(clap::Clap)]
enum QueueCommand {
    #[clap(short_flag = 'u')]
    Url { url: Url },
}
#[derive(clap::Clap)]
enum ControlCommand {
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
struct PrivateCommandParser {
    #[clap(subcommand)]
    cmd: PrivateCommand,
}
#[derive(clap::Clap)]
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
        match cmd {
            GuildCommand::Queue { cmd } => match cmd {
                QueueCommand::Url { url } => Queuer::from_registry()
                    .try_send(UrlQueueData { url, from, guild })
                    .expect("failed sending"),
            },
            GuildCommand::Control { cmd } => {
                let kind = match cmd {
                    ControlCommand::Join { channel } => ActionKind::Join { channel },
                    ControlCommand::Play => ActionKind::Play,
                    ControlCommand::Stop => ActionKind::Stop,
                    ControlCommand::Leave => ActionKind::Leave,
                };

                Connector::from_registry()
                    .try_send(Action { kind, from, guild })
                    .expect("failed sending")
            },
        }
    }
}
impl Supervised for GuildCommandProcesser {}
impl ArbiterService for GuildCommandProcesser {}
