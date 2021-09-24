use actix::prelude::*;

use crate::connection::{UrlQueue, UrlQueueData};
use crate::gateway::{MsgRef, RawCommand};

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
        let split = try_handle!(shell_words::split(&content); to = from);
        if let Some("*v") = split.get(0).map(|s| s.as_str()) {
            use clap::Clap;

            let cmd = try_handle!(Parser::try_parse_from(split);
                to = from;
                match = Parser { cmd } => cmd
            );

            CommandProcesser::from_registry().do_send(Command {
                cmd,
                user,
                from,
                guild,
            });
        }
    }
}
impl Supervised for CommandParser {}
impl ArbiterService for CommandParser {}

#[derive(clap::Clap)]
struct Parser {
    #[clap(subcommand)]
    cmd: Cmd,
}

#[derive(clap::Clap)]
#[non_exhaustive]
enum Cmd {
    /// using url.
    #[clap(short_flag = 'U')]
    Url {
        /// url to youtube's video.
        url: url::Url,
    },
}

pub struct Command {
    cmd: Cmd,
    from: MsgRef,
    user: u64,
    guild: u64,
}
impl Message for Command {
    type Result = ();
}

#[derive(Default)]
pub struct CommandProcesser;
impl Actor for CommandProcesser {
    type Context = Context<Self>;
}
impl Handler<Command> for CommandProcesser {
    type Result = ();

    fn handle(
        &mut self,
        Command {
            cmd,
            user,
            from,
            guild,
        }: Command,
        _: &mut Self::Context,
    ) -> Self::Result {
        use Cmd::*;
        match cmd {
            Url { url } => UrlQueue::from_registry().do_send(UrlQueueData {
                url,
                from,
                user,
                guild,
            }),
            #[allow(unreachable_patterns)]
            _ => unimplemented!("unimplemented command"),
        }
    }
}
impl Supervised for CommandProcesser {}
impl ArbiterService for CommandProcesser {}
