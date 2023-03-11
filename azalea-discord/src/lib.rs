use azalea_auth::game_profile::GameProfile;
use azalea_client::{
    packet_handling::ChatReceivedEvent, ChatPacket, GameProfileComponent, LocalPlayer,
};
use azalea_ecs::{
    app::Plugin,
    event::EventReader,
    schedule::{IntoSystemDescriptor, ShouldRun},
    system::{Query, Res, Resource},
};
use azalea_protocol::packets::game::serverbound_chat_packet::{
    LastSeenMessagesUpdate, ServerboundChatPacket,
};
use flume::{Receiver, Sender};
use std::time::{SystemTime, UNIX_EPOCH};

mod discord;

#[derive(Debug, Clone)]
pub struct DiscordPlugin {
    pub resource: DiscordPluginChannel,
}

impl DiscordPlugin {
    pub async fn new(token: &str, channel_id: u64, ignore_list: Vec<&str>) -> Self {
        let ignore = ignore_list.iter().map(|s| s.to_string()).collect();

        // Create commucation channel
        let (client_tx, client_rx) = flume::unbounded();
        let (server_tx, server_rx) = flume::unbounded();

        tokio::spawn(discord::main(
            token.to_string(),
            channel_id,
            client_rx,
            server_tx,
        ));

        Self {
            resource: DiscordPluginChannel {
                rx: server_rx,
                tx: client_tx,
                ignore,
                mode: DiscordPluginMode::default(),
            },
        }
    }
}

#[derive(Debug, Clone, Resource)]
pub struct DiscordPluginChannel {
    rx: Receiver<DiscordEvent>,
    tx: Sender<AzaleaEvent>,
    pub ignore: Vec<String>,
    pub mode: DiscordPluginMode,
}

#[derive(Debug, Clone, Default)]
pub enum DiscordPluginMode {
    #[default]
    Default,
    Single(String),
}

#[derive(Debug, Clone)]
enum AzaleaEvent {
    Chat(Option<GameProfile>, ChatPacket),
}

#[derive(Debug, Clone)]
enum DiscordEvent {
    Chat(String, String),
}

impl Plugin for DiscordPlugin {
    fn build(&self, app: &mut azalea_ecs::app::App) {
        app.insert_resource(self.resource.clone())
            .add_system(listen_mc_chat)
            .add_system(listen_discord_chat.with_run_criteria(listen_discord_chat_criteria));
    }
}

fn listen_mc_chat(
    channel: Res<DiscordPluginChannel>,
    profiles: Query<&GameProfileComponent>,
    mut chat_events: EventReader<ChatReceivedEvent>,
) {
    let mut received = vec![];
    for event in chat_events.iter() {
        // In Single mode, only listen to events from the targeted bot
        if let DiscordPluginMode::Single(target) = &channel.mode {
            let profile = profiles.get(event.entity).unwrap();
            if profile.name != *target {
                continue;
            }
        }

        // Do not repeatedly send the same message if received by multiple bots
        if received.contains(&event.packet.content()) {
            continue;
        }
        received.push(event.packet.content());

        // Get the profile and check username
        let mut sender_profile = None;
        if let Some(sender) = event.packet.uuid() {
            // Filter list of profiles by UUID
            let profile: Vec<&GameProfileComponent> =
                profiles.iter().filter(|p| p.uuid == sender).collect();
            // There should only be one profile with a matching UUID.
            // If there's no match, it is most likely a Server message.
            if let Some(profile) = profile.first() {
                // Get the profile of who sent the message
                sender_profile = Some(profile.0.clone());

                // Do not send messages sent by players in the filter list
                if channel.ignore.contains(&profile.name) {
                    continue;
                }
            }
        }

        // Send event to Matrix
        channel
            .tx
            .send(AzaleaEvent::Chat(sender_profile, event.packet.clone()))
            .expect("Unable to send message event to DiscordPlugin");
    }
}

// Only run if there are messages to receive
fn listen_discord_chat_criteria(channel: Res<DiscordPluginChannel>) -> ShouldRun {
    if !channel.rx.is_empty() {
        ShouldRun::Yes
    } else {
        ShouldRun::No
    }
}

fn listen_discord_chat(
    channel: Res<DiscordPluginChannel>,
    mut query: Query<(&GameProfileComponent, &mut LocalPlayer)>,
) {
    for (profile, mut player) in query.iter_mut() {
        // In Single mode, only send events from the targeted bot
        if let DiscordPluginMode::Single(target) = &channel.mode {
            if profile.name != *target {
                continue;
            }
        }

        // Receive all messages from Matrix
        while let Ok(event) = channel.rx.try_recv() {
            match event {
                DiscordEvent::Chat(name, msg) => {
                    // Split message if longer than 255 characters
                    for message in format_message(name, msg) {
                        // Create chat packet with message
                        let packet = ServerboundChatPacket {
                            message,
                            timestamp: SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .unwrap()
                                .as_millis() as u64,
                            salt: azalea_crypto::make_salt(),
                            signature: None,
                            last_seen_messages: LastSeenMessagesUpdate::default(),
                        };

                        // Send packet
                        player.write_packet(packet.get());
                    }
                }
            }
        }
    }
}

// Split message into multiple Strings with length at most 254
fn format_message(name: String, msg: String) -> Vec<String> {
    let mut message = format!("{name}: {msg}");
    if message.len() < 255 {
        return vec![message];
    }

    let mut result = vec![];
    while message.len() >= 255 {
        let (first, second) = message.split_at(254);
        result.push(first.to_string());
        message = second.to_string();
    }
    result.push(message);
    result
}
