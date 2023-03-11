use azalea_auth::game_profile::GameProfile;
use azalea_client::{
    packet_handling::ChatReceivedEvent, ChatPacket, GameProfileComponent, LocalPlayer,
};
use azalea_ecs::{
    app::{App, Plugin},
    event::EventReader,
    schedule::{IntoSystemDescriptor, ShouldRun},
    system::{Query, Res, Resource},
};
use azalea_protocol::packets::game::serverbound_chat_packet::{
    LastSeenMessagesUpdate, ServerboundChatPacket,
};
use flume::{Receiver, Sender};
use matrix_sdk_appservice::{AppServiceBuilder, AppServiceRegistration};
use std::time::{SystemTime, UNIX_EPOCH};

mod matrix;

#[derive(Debug, Clone)]
pub struct MatrixPlugin {
    pub resource: MatrixPluginChannel,
}

impl MatrixPlugin {
    /// Starts the Matrix bot and gives you a plugin piece to give to Azalea.
    pub async fn new(
        server_url: &str,
        server_name: &str,
        registration: &str,
        room_id: &str,
        ignore_list: Vec<&str>,
        bot_name: Option<String>,
        bot_image: Option<String>,
    ) -> Result<MatrixPlugin, matrix_sdk_appservice::Error> {
        let room = room_id.to_string();
        let ignore_list = ignore_list.iter().map(|s| s.to_string()).collect();

        // Create commucation channel
        let (client_tx, client_rx) = flume::unbounded();
        let (server_tx, server_rx) = flume::unbounded();

        // Create the AppService
        let appservice = AppServiceBuilder::new(
            server_url.parse()?,
            server_name.parse()?,
            AppServiceRegistration::try_from_yaml_file(registration)?,
        )
        .build()
        .await?;

        // Spawn a new thread to do matrix bot things
        tokio::spawn(matrix::startup(
            bot_name, bot_image, room, appservice, server_tx, client_rx,
        ));

        // Return a 'MatrixPlugin' to insert into Azalea
        Ok(MatrixPlugin {
            resource: MatrixPluginChannel {
                rx: server_rx,
                tx: client_tx,
                mode: MatrixPluginMode::default(),
                ignore: ignore_list,
            },
        })
    }
}

/// Contains plugin settings and channel objects used
/// to send information between Matrix and Azalea
#[derive(Debug, Clone, Resource)]
pub struct MatrixPluginChannel {
    rx: Receiver<MatrixEvent>,
    tx: Sender<AzaleaEvent>,
    pub mode: MatrixPluginMode,
    pub ignore: Vec<String>,
}

/// The 'MatrixPluginMode' should not matter if you only have a single bot.
/// If you have multiple, in a swarm for example, the bot who sends
/// chat messages is not guaranteed. In this case, you can set
/// the mode to `Single(username)` to only send and receive messages
/// from that bot.
#[derive(Debug, Default, Clone)]
pub enum MatrixPluginMode {
    #[default]
    Default,
    Single(String),
}
impl Plugin for MatrixPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(self.resource.clone())
            .add_system(listen_mc_chat)
            .add_system(listen_mx_chat.with_run_criteria(listen_mx_chat_criteria));
    }
}

// Enums in case there are new events added
#[derive(Debug, Clone)]
enum AzaleaEvent {
    Chat(Option<GameProfile>, ChatPacket),
}

// Enums in case there are new events added
#[derive(Debug, Clone)]
enum MatrixEvent {
    Chat(String, String),
}

fn listen_mc_chat(
    channel: Res<MatrixPluginChannel>,
    profiles: Query<&GameProfileComponent>,
    mut chat_events: EventReader<ChatReceivedEvent>,
) {
    let mut received = vec![];
    for event in chat_events.iter() {
        // In Single mode, only listen to events from the targeted bot
        if let MatrixPluginMode::Single(target) = &channel.mode {
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
            .expect("Unable to send message event to MatrixPlugin");
    }
}

// Only run if there are messages to receive
fn listen_mx_chat_criteria(channel: Res<MatrixPluginChannel>) -> ShouldRun {
    if !channel.rx.is_empty() {
        ShouldRun::Yes
    } else {
        ShouldRun::No
    }
}

fn listen_mx_chat(
    channel: Res<MatrixPluginChannel>,
    mut query: Query<(&GameProfileComponent, &mut LocalPlayer)>,
) {
    for (profile, mut player) in query.iter_mut() {
        // In Single mode, only send events from the targeted bot
        if let MatrixPluginMode::Single(target) = &channel.mode {
            if profile.name != *target {
                continue;
            }
        }

        // Receive all messages from Matrix
        while let Ok(event) = channel.rx.try_recv() {
            match event {
                MatrixEvent::Chat(name, msg) => {
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
