#![allow(clippy::unit_arg)]

extern crate alloc;

macro_rules! try_handle {
    ($expr:expr; to = $to:expr) => {
        try_handle!($expr; to = $to; match = o => o)
    };
    ($expr:expr; to = $to:expr; match = $pat:pat => $( $out:tt )*) => {{
        use crate::util::Pipe;

        match $expr {
            Ok($pat) => $( $out )*,
            Err(e) =>
                return crate::gateway::get_reply_recipient()
                    .do_send(crate::gateway::Reply {
                        msg: e.to_string(),
                        kind: crate::gateway::Kind::Err,
                        to: $to
                    })
                    .pipe(crate::util::do_send_handle),
        }}
    };
}

mod cmd;
mod connection;
mod gateway;
mod util;

fn main() {
    tracing_subscriber::fmt().pretty().init();

    let sys = actix::System::new();

    sys.block_on(async move {
        tracing::info!("initializing...");

        use actix::Actor;

        gateway::Gateway::start_default();

        tracing::info!("initialized.");
    });

    sys.run().unwrap()
}
