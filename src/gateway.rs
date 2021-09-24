use actix::prelude::*;

#[derive(Debug)]
pub struct MsgRef {
    message: u64,
    channel: u64,
    guild: Option<u64>,
}
impl MsgRef {
    pub fn is_dm(&self) -> bool { self.guild.is_none() }
}

pub struct RawCommand {
    pub content: String,
    pub from: MsgRef,
    pub user: u64,
    pub guild: u64,
}
impl Message for RawCommand {
    type Result = ();
}

#[derive(Debug)]
pub struct Reply {
    pub msg: String,
    pub kind: Kind,
    pub to: MsgRef,
}

#[derive(Debug)]
pub enum Kind {
    Ok,
    Err,
}

impl Message for Reply {
    type Result = ();
}

pub fn get_reply_recipient() -> Recipient<Reply> { unimplemented!() }
