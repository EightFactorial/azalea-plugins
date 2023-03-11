use azalea_bridge::{AzaleaEvent, PluginSide};
use flume::{Receiver, Sender};
use log::{error, info, warn};
use std::{error::Error, sync::Arc};
use twilight_cache_inmemory::{InMemoryCache, ResourceType};
use twilight_gateway::{Event, Intents, Shard, ShardId};
use twilight_http::Client as HttpClient;
use twilight_model::id::{marker::ChannelMarker, Id};

use crate::DiscordEvent;

pub(crate) async fn main(
    token: String,
    channel_id: u64,
    plugin: PluginSide<DiscordEvent>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    // Specify intents requesting events about things like new and updated
    // messages in a guild and direct messages.
    let intents = Intents::GUILD_MESSAGES | Intents::MESSAGE_CONTENT;

    // Create a single shard.
    let mut shard = Shard::new(ShardId::ONE, token.clone(), intents);

    // The http client is separate from the gateway, so startup a new
    // one, also use Arc such that it can be cloned to other threads.
    let http = Arc::new(HttpClient::new(token));

    let tx = Arc::new(plugin.tx);

    // Since we only care about messages, make the cache only process messages.
    let cache = InMemoryCache::builder()
        .resource_types(ResourceType::MESSAGE)
        .build();

    tokio::spawn(handle_mc_event(
        Arc::clone(&http),
        Id::new(channel_id),
        plugin.rx,
    ));

    // Startup the event loop to process each event in the event stream as they
    // come in.
    loop {
        let event = match shard.next_event().await {
            Ok(event) => event,
            Err(source) => {
                warn!("Error receiving event: {source}");
                if source.is_fatal() {
                    break;
                }
                continue;
            }
        };
        // Update the cache.
        cache.update(&event);

        // Spawn a new task to handle the event
        tokio::spawn(handle_discord_event(event, Arc::clone(&tx)));
    }

    Ok(())
}

async fn handle_discord_event(event: Event, tx: Arc<Sender<DiscordEvent>>) -> anyhow::Result<()> {
    match event {
        Event::Ready(_) => {
            info!("Discord bot is ready!");
        }
        Event::MessageCreate(event) => {
            // Don't send messages from bots
            if event.author.bot {
                return Ok(());
            }

            // Send message to Azalea
            tx.send_async(DiscordEvent {
                username: event.author.name.clone(),
                message: event.content.clone(),
            })
            .await?;
        }
        _ => {}
    }
    Ok(())
}

async fn handle_mc_event(
    http: Arc<HttpClient>,
    channel_id: Id<ChannelMarker>,
    rx: Receiver<AzaleaEvent>,
) -> anyhow::Result<()> {
    loop {
        let Ok(event) = rx.recv_async().await else {
            error!("DiscordPlugin Minecraft listener closed");
            return Err(anyhow::Error::msg("DiscordPlugin Minecraft listener closed"));        
        };
        match event {
            AzaleaEvent::Chat(_profile, packet) => {
                let (sender, message) = packet.split_sender_and_content();
                let sender = if let Some(username) = sender {
                    username
                } else {
                    "Server".to_string()
                };

                // TODO: Escape formatting characters
                http.create_message(channel_id)
                    .content(&format!("{sender}: {message}"))
                    .unwrap()
                    .await
                    .unwrap();
            }
        }
    }
}
