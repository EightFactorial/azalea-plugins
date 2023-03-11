use azalea_bridge::{AzaleaEvent, PluginEvent, PluginSide};
use flume::{Receiver, Sender};
use log::{error, info, warn};
use std::error::Error;
use twilight_cache_inmemory::{InMemoryCache, ResourceType};
use twilight_gateway::{Event, Intents, Shard, ShardId};
use twilight_http::Client as HttpClient;
use twilight_model::id::{
    marker::{ChannelMarker, WebhookMarker},
    Id,
};

use crate::DiscordPlugin;

pub(crate) async fn main(
    bot_token: String,
    channel_id: u64,
    webhook_token: String,
    webhook_id: u64,
    plugin: PluginSide<DiscordPlugin>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    // Create a single shard.
    let mut shard = Shard::new(
        ShardId::ONE,
        bot_token.clone(),
        Intents::GUILD_MESSAGES | Intents::MESSAGE_CONTENT,
    );

    // The http client is separate from the gateway, so startup a new one.
    let http = HttpClient::new(bot_token);

    // Since we only care about messages, make the cache only process messages.
    let cache = InMemoryCache::builder()
        .resource_types(ResourceType::MESSAGE)
        .build();

    // Handle events from Azalea
    tokio::spawn(handle_mc_event(
        http,
        webhook_token,
        Id::new(webhook_id),
        plugin.rx,
    ));

    // Startup the event loop to process each event in the event stream as they
    // come in.
    let channel_id: Id<ChannelMarker> = Id::new(channel_id);
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
        tokio::spawn(handle_discord_event(
            event,
            channel_id.clone(),
            plugin.tx.clone(),
        ));
    }

    Ok(())
}

async fn handle_discord_event(
    event: Event,
    channel_id: Id<ChannelMarker>,
    tx: Sender<PluginEvent>,
) -> anyhow::Result<()> {
    match event {
        Event::Ready(_) => {
            info!("Discord bot is ready!");
        }
        Event::MessageCreate(event) => {
            // Only listen on one channel
            if channel_id != event.channel_id {
                return Ok(());
            }

            // Don't send messages from bots
            if event.author.bot {
                return Ok(());
            }

            // Send message to Azalea
            if let Err(e) = tx
                .send_async(PluginEvent::Chat(
                    event.author.name.clone(),
                    event.content.clone(),
                ))
                .await
            {
                error!("DiscordPlugin unable to send message to Azalea: {e}");
            }
        }
        _ => {}
    }
    Ok(())
}

async fn handle_mc_event(
    http: HttpClient,
    webhook_token: String,
    webhook_id: Id<WebhookMarker>,
    rx: Receiver<AzaleaEvent>,
) -> anyhow::Result<()> {
    loop {
        let Ok(event) = rx.recv_async().await else {
            error!("DiscordPlugin Minecraft listener closed");
            return Err(anyhow::Error::msg("DiscordPlugin Minecraft listener closed"));        
        };
        match event {
            AzaleaEvent::Chat(profile, packet) => {
                let username = if let Some(user) = packet.username() {
                    user
                } else {
                    profile.name
                };

                // Attempt to escape formatting
                let message = packet
                    .content()
                    .replace('\\', "\\*")
                    .replace('*', "\\*")
                    .replace('_', "\\_")
                    .replace('`', "\\`")
                    .replace('>', "\\>");

                if let Ok(message) = http
                    .execute_webhook(webhook_id, &webhook_token)
                    .content(&message)
                {
                    if let Ok(message) = message.username(&username) {
                        if let Err(e) = message.await {
                            error!("Unable to send message: {e}");
                            continue;
                        }
                    } else {
                        error!("Unable to set message username: {username}");
                        continue;
                    }
                } else {
                    error!("Unable to set message content: {message}");
                    continue;
                }
            }
            _ => {}
        }
    }
}
