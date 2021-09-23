mod cmd;
mod connection;
mod gateway;
mod util;

fn main() {
    tracing_subscriber::fmt().pretty().init();

    let sys = actix::System::new();

    sys.block_on(async move {
        tracing::info!("initializing...");

        // process here

        tracing::info!("initialized.");
    });
}
