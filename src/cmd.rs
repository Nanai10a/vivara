use actix::prelude::*;

use crate::connection::{UrlQueue, UrlQueueData};
use crate::gateway::{get_reply_recipient, Kind, MsgRef, RawCommand, Reply};
use crate::util::{do_send_handle, Pipe};

macro_rules! try_handle {
    ($expr:expr; to = $to:expr) => {
        try_handle!($expr; to = $to; match = o => o)
    };
    ($expr:expr; to = $to:expr; match = $pat:pat => $( $out:tt )*) => {
        match $expr {
            Ok($pat) => $( $out )*,
            Err(e) =>
                return get_reply_recipient()
                    .do_send(Reply {
                        msg: e.to_string(),
                        kind: Kind::Err,
                        to: $to
                    })
                    .pipe(do_send_handle),
        }
    };
}

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
                from: user,
                to: from,
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
    from: u64,
    to: MsgRef,
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
        Command { cmd, from, to }: Command,
        _: &mut Self::Context,
    ) -> Self::Result {
        use Cmd::*;
        match cmd {
            Url { url } => UrlQueue::from_registry().do_send(UrlQueueData { url, from, to }),
            #[allow(unreachable_patterns)]
            _ => unimplemented!("unimplemented command"),
        }
    }
}
impl Supervised for CommandProcesser {}
impl ArbiterService for CommandProcesser {}
