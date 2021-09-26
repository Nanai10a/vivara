#![feature(try_blocks)]
#![feature(result_flattening)]
#![allow(clippy::unit_arg)]

extern crate alloc;

mod command;
mod connection;
mod gateway;
mod util;

fn main() {
    tracing_subscriber::fmt().pretty().init();

    let sys = actix::System::new();

    sys.block_on(async move {
        use actix::Actor;

        gateway::Gateway::start_default();

        tracing::info!("system initialized");
    });

    if let Err(e) = sys.run() {
        tracing::error!("system error: {}", e);
    }
}
