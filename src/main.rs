#![feature(try_blocks)]
#![feature(result_flattening)]
#![feature(bound_map)]
#![feature(unboxed_closures)]
#![feature(fn_traits)]
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
        initialize().await;

        tracing::info!("system initialized");
    });

    if let Err(e) = sys.run() {
        tracing::error!("system error: {}", e);
    }
}

async fn initialize() {
    use alloc::sync::Arc;

    use actix::{Actor, ArbiterService, Registry};
    use connection::Connector;
    use futures_util::StreamExt;
    use gateway::{Gateway, GatewayMessage, MessageRef};
    use songbird::Songbird;
    use twilight_gateway::Event;
    use util::Pipe;

    let (cluster, user_id, mut events) = build_cluster().await;
    cluster.up().await;

    let songbird = Songbird::twilight(cluster, user_id).pipe(Arc::new);

    let connector = Connector::new(songbird.clone()).start();
    Registry::set(connector);

    let gateway = Gateway::from_registry();

    let fut = async move {
        while let Some((id, event)) = events.next().await {
            tracing::trace!("received event: ({}) {:?}", id, event);
            songbird.process(&event).await;

            if let Event::MessageCreate(mc) = event {
                let msg = GatewayMessage {
                    content: mc.0.content,
                    from: MessageRef {
                        message: mc.0.id.0,
                        channel: mc.0.channel_id.0,
                    },
                    user: mc.0.author.id.0,
                    guild: mc.0.guild_id.map(|i| i.0),
                };

                gateway.try_send(msg).expect("failed sending")
            }
        }
    };

    tokio::spawn(fut);
}

async fn build_cluster() -> (
    twilight_gateway::Cluster,
    twilight_model::id::UserId,
    twilight_gateway::cluster::Events,
) {
    use twilight_gateway::{Cluster, Intents};
    use twilight_http::Client;
    use util::token;

    let (cluster, events) = loop {
        match Cluster::new(
            token::<String>(),
            Intents::GUILD_VOICE_STATES | Intents::GUILD_MESSAGES | Intents::DIRECT_MESSAGES,
        )
        .await
        {
            Ok(t) => break t,
            Err(e) => tracing::warn!("failed initializing cluster: {}", e),
        }
    };

    let user_id = loop {
        let result = Client::new(token()).current_user().exec().await;

        let response = match result {
            Ok(o) => o,
            Err(e) => {
                tracing::warn!("failed getting current_user: {}", e);
                continue;
            },
        };

        let user = match response.model().await {
            Ok(o) => o,
            Err(e) => {
                tracing::warn!(
                    "failed deserialization to response of getting current_user: {}",
                    e
                );
                continue;
            },
        };

        break user.id;
    };

    (cluster, user_id, events)
}
