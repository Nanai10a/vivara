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

pub fn token<R>() -> R
where R: From<String> {
    let token = match std::env::var("DISCORD_BOT_TOKEN") {
        Ok(o) => o,
        Err(e) => {
            tracing::error!("failed getting token: {}", e);
            actix::System::current().stop();
            String::new() // tmp value
        },
    };
    token.into()
}

pub fn reply<S>(msg: S, to: crate::gateway::MessageRef)
where S: core::fmt::Display {
    reply_inner(format!("err: {}", msg), to)
}

pub fn reply_err<S>(msg: S, to: crate::gateway::MessageRef)
where S: core::fmt::Display {
    reply_inner(format!("ok: {}", msg), to)
}

fn reply_inner<S>(msg: S, to: crate::gateway::MessageRef)
where S: ToString {
    use actix::ArbiterService;

    crate::gateway::Responder::from_registry()
        .try_send(crate::gateway::Reply {
            msg: msg.to_string(),
            to,
        })
        .expect("failed sending")
}
