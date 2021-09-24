use actix::prelude::*;

#[derive(Debug)]
pub struct MsgRef {
    message_id: u64,
    channel_id: u64,
    guild_id: Option<u64>,
}
impl MsgRef {
    pub fn is_dm(&self) -> bool { self.guild_id.is_none() }
}

pub struct RawCommand{
    pub content: String, 
    pub from: MsgRef,
    pub user: u64,
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
