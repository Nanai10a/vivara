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
