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
use std::{
    marker::PhantomData,
    time::{SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct PluginBridge<T: Clone + Sync + Send + 'static> {
    pub client: ClientSide<T>,
    pub plugin: PluginSide<T>,
}

impl<T: Clone + Sync + Send + 'static> PluginBridge<T> {
    pub fn new(ignore_list: Vec<String>) -> Self {
        // Create commucation channel
        let (client_tx, client_rx) = flume::unbounded();
        let (plugin_tx, plugin_rx) = flume::unbounded();

        Self {
            client: ClientSide {
                rx: plugin_rx,
                tx: client_tx,
                ignore_list,
                _d: PhantomData,
            },
            plugin: PluginSide {
                rx: client_rx,
                tx: plugin_tx,
                _d: PhantomData,
            },
        }
    }
}

#[derive(Debug, Clone, Resource)]
pub struct ClientSide<T> {
    pub rx: Receiver<PluginEvent>,
    pub tx: Sender<AzaleaEvent>,
    pub ignore_list: Vec<String>,
    _d: PhantomData<T>,
}

impl<T: Clone + Sync + Send + 'static> ClientSide<T> {
    pub fn listen_chat(
        client: Res<ClientSide<T>>,
        profiles: Query<&GameProfileComponent>,
        mut chat_events: EventReader<ChatReceivedEvent>,
    ) {
        for event in chat_events.iter() {
            let mut profile = GameProfile::default();

            if let Some(uuid) = event.packet.uuid() {
                if let Some(found) = find_profile(uuid, &profiles) {
                    // Found a matching GameProfile
                    profile = found.0;
                } else {
                    // Message has a uuid but we don't have a GameProfile yet
                    profile.uuid = uuid;
                    profile.name = "Unknown".to_string();
                }
            } else {
                // Server messages have no UUID
                profile.name = "Server".to_string();
            }

            // No not send messages from players in the ignore list
            if client.ignore_list.contains(&profile.name) {
                continue;
            }

            client
                .tx
                .send(AzaleaEvent::Chat(profile, event.packet.clone()))
                .unwrap_or_else(|e| panic!("Unable to send event to plugin: {e}"));
        }
    }

    pub fn listen_event_criteria(plugin: Res<ClientSide<T>>) -> ShouldRun {
        if !plugin.rx.is_empty() {
            ShouldRun::Yes
        } else {
            ShouldRun::No
        }
    }

    pub fn listen_event(client: Res<ClientSide<T>>, mut query: Query<&mut LocalPlayer>) {
        let Ok(mut player) = query.get_single_mut() else { return };
        while let Ok(event) = client.rx.try_recv() {
            let PluginEvent::Chat(username, message) = event;
            let packets = create_packets(username, message);
            for packet in packets {
                player.write_packet(packet.get());
            }
        }
    }
}

impl<T: Clone + Sync + Send + 'static> Plugin for ClientSide<T> {
    fn build(&self, app: &mut App) {
        app.insert_resource(self.clone())
            .add_system(ClientSide::<T>::listen_chat)
            .add_system(
                ClientSide::<T>::listen_event
                    .with_run_criteria(ClientSide::<T>::listen_event_criteria),
            );
    }
}

#[derive(Debug, Clone)]
pub struct PluginSide<T> {
    pub rx: Receiver<AzaleaEvent>,
    pub tx: Sender<PluginEvent>,
    _d: PhantomData<T>,
}

#[derive(Debug, Clone)]
pub enum AzaleaEvent {
    Chat(GameProfile, ChatPacket),
}

#[derive(Debug, Clone)]
pub enum PluginEvent {
    Chat(String, String),
}

fn find_profile(
    uuid: Uuid,
    profiles: &Query<&GameProfileComponent>,
) -> Option<GameProfileComponent> {
    for profile in profiles.iter() {
        if uuid == profile.uuid {
            return Some(profile.clone());
        }
    }
    None
}

fn create_packets(username: String, message: String) -> Vec<ServerboundChatPacket> {
    let mut list: Vec<ServerboundChatPacket> = Vec::new();
    for message in format_message(username, message) {
        list.push(ServerboundChatPacket {
            message,
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
            salt: azalea_crypto::make_salt(),
            signature: None,
            last_seen_messages: LastSeenMessagesUpdate::default(),
        });
    }
    list
}

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
