pub trait Pipe {
    fn pipe<F, R>(self, f: F) -> R
    where
        Self: Sized,
        F: FnOnce(Self) -> R;
}
impl<T> Pipe for T {
    fn pipe<F, R>(self, f: F) -> R
    where F: FnOnce(Self) -> R {
        f(self)
    }
}

pub trait Pass {
    fn pass<F, R>(self, f: F) -> Self
    where F: FnOnce(&Self) -> R;

    fn pass_mut<F, R>(self, f: F) -> Self
    where F: FnOnce(&Self) -> R;
}
impl<T> Pass for T {
    fn pass<F, R>(self, f: F) -> Self
    where F: FnOnce(&Self) -> R {
        f(&self);
        self
    }

    fn pass_mut<F, R>(mut self, f: F) -> Self
    where F: FnOnce(&Self) -> R {
        f(&mut self);
        self
    }
}

pub fn do_send_handle<M: core::fmt::Debug>(res: Result<(), actix::prelude::SendError<M>>) {
    use actix::prelude::SendError::*;

    let (reason, inner) = match res {
        Ok(_) => return,
        Err(Full(i)) => ("mailbox is full", i),
        Err(Closed(i)) => ("cannot reach addr", i),
    };

    tracing::warn!("do_send failed: {} with: {:?}", reason, inner)
}

pub fn token<R>() -> R
where R: From<String> {
    let token = match std::env::var("DISCORD_BOT_TOKEN") {
        Ok(o) => o,
        Err(e) => {
            tracing::error!("cannot get token: {}", e);
            actix::System::current().stop();
            String::new() // tmp value
        },
    };
    token.into()
}

pub fn reply<S>(msg: S, to: crate::gateway::MsgRef)
where S: ToString {
    reply_inner(msg, crate::gateway::Kind::Ok, to)
}

pub fn reply_err<S>(msg: S, to: crate::gateway::MsgRef)
where S: ToString {
    reply_inner(msg, crate::gateway::Kind::Err, to)
}

fn reply_inner<S>(msg: S, kind: crate::gateway::Kind, to: crate::gateway::MsgRef)
where S: ToString {
    crate::gateway::get_reply_recipient()
        .do_send(crate::gateway::Reply {
            msg: msg.to_string(),
            kind,
            to,
        })
        .pipe(do_send_handle)
}
