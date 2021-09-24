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
        }: RawCommand,
        _: &mut Self::Context,
    ) -> Self::Result {
        let split = try_handle!(shell_words::split(&content); to = from);

        match split.get(0).map(|s| s.as_str()) {
            Some("*v") => (),
            _ => return,
        }

        use clap::Clap;
        if from.is_dm() {
            let _ = try_handle!(CtrlCmdParser::try_parse_from(split);
                to = from;
                match = CtrlCmdParser { cmd } => cmd
            );

            unimplemented!();
        } else {
            let cmd = try_handle!(PlayCmdParser::try_parse_from(split);
                to = from;
                match = PlayCmdParser { cmd } => cmd
            );

            PlayCommandProcesser::from_registry().do_send(PlayCommand {
                cmd,
                guild: from.guild().unwrap(),
                from,
            });
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
#[non_exhaustive]
enum PlayCmd {
    /// using url.
    #[clap(short_flag = 'U')]
    Url {
        /// url to youtube's video.
        url: url::Url,
    },
}

#[derive(clap::Clap)]
struct CtrlCmdParser {
    #[clap(subcommand)]
    cmd: CtrlCmd,
}

#[derive(clap::Clap)]
#[non_exhaustive]
enum CtrlCmd {}

pub struct PlayCommand {
    cmd: PlayCmd,
    from: MsgRef,
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
        use PlayCmd::*;
        match cmd {
            Url { url } => UrlQueue::from_registry().do_send(UrlQueueData { url, from, guild }),
            #[allow(unreachable_patterns)]
            _ => unimplemented!("unimplemented command"),
        }
    }
}
impl Supervised for PlayCommandProcesser {}
impl ArbiterService for PlayCommandProcesser {}
