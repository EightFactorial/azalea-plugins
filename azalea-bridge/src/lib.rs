use azalea_auth::game_profile::GameProfile;
use azalea_client::{
    packet_handling::ChatReceivedEvent, ChatPacket, GameProfileComponent, LocalPlayer,
};
use azalea_ecs::{
    app::{App, Plugin},
    entity::Entity,
    event::EventReader,
    schedule::{IntoSystemDescriptor, ShouldRun},
    system::{Query, Res, Resource},
};
#[cfg(feature = "bridge")]
use azalea_protocol::packets::game::clientbound_player_chat_packet::{
    ChatType, ChatTypeBound, ClientboundPlayerChatPacket, FilterMask, PackedLastSeenMessages,
    PackedSignedMessageBody,
};
use azalea_protocol::packets::game::serverbound_chat_packet::{
    LastSeenMessagesUpdate, ServerboundChatPacket,
};
use flume::{Receiver, Sender};
#[cfg(feature = "bridge")]
use log::error;
#[cfg(feature = "bridge")]
use std::sync::Arc;
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
                #[cfg(feature = "bridge")]
                links: vec![],
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
    #[cfg(feature = "bridge")]
    links: Vec<Sender<AzaleaEvent>>,
}

impl<T: Clone + Sync + Send + 'static> ClientSide<T> {
    // Bridge two plugins together and share events
    #[cfg(feature = "bridge")]
    pub fn bridge<U: Clone + Sync + Send + 'static>(&mut self, other: &mut ClientSide<U>) {
        self.links.push(other.tx.clone());
        other.links.push(self.tx.clone());
    }

    // Send chat events to plugin
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

            // Do not send messages from players in the ignore list
            if client.ignore_list.contains(&profile.name) {
                continue;
            }

            // Send event to plugin
            client
                .tx
                .send(AzaleaEvent::Chat(profile, event.packet.clone()))
                .unwrap_or_else(|e| panic!("Unable to send event to plugin: {e}"));
        }
    }

    // Whether or not to run the listen_event system
    pub fn listen_event_criteria(plugin: Res<ClientSide<T>>) -> ShouldRun {
        if !plugin.rx.is_empty() {
            ShouldRun::Yes
        } else {
            ShouldRun::No
        }
    }

    // Process events from channel
    pub fn listen_event(
        client: Res<ClientSide<T>>,
        mut query: Query<(Entity, &mut LocalPlayer)>,
        #[cfg(feature = "bridge")] profiles: Query<&GameProfileComponent>,
    ) {
        let Ok((_entity, mut player)) = query.get_single_mut() else { return };
        while let Ok(event) = client.rx.try_recv() {
            #[cfg(feature = "bridge")]
            if let Err(e) = Self::link_plugins(&client, event.clone(), _entity, &profiles) {
                error!("Unable to send message to linked plugin: {e}");
            }
            match event {
                PluginEvent::Chat(username, message) => {
                    let packets = create_packets(username, message);
                    for packet in packets {
                        player.write_packet(packet.get());
                    }
                }
                _ => {}
            }
        }
    }

    // Bridge events to other plugins
    #[cfg(feature = "bridge")]
    fn link_plugins(
        client: &ClientSide<T>,
        event: PluginEvent,
        entity: Entity,
        profiles: &Query<&GameProfileComponent>,
    ) -> anyhow::Result<()> {
        if client.links.len() == 0 {
            return Ok(());
        }
        let profile = profiles.get(entity)?;
        match event {
            PluginEvent::Chat(username, message) => {
                let packet = ClientboundPlayerChatPacket {
                    sender: profile.uuid.clone(),
                    index: 0,
                    signature: None,
                    body: PackedSignedMessageBody {
                        content: format!("{username}: {message}"),
                        timestamp: 0,
                        salt: 0,
                        last_seen: PackedLastSeenMessages {
                            entries: Vec::new(),
                        },
                    },
                    unsigned_content: None,
                    filter_mask: FilterMask::PassThrough,
                    chat_type: ChatTypeBound {
                        chat_type: ChatType::Chat,
                        name: profile.name.clone().into(),
                        target_name: None,
                    },
                };

                let packet = ChatPacket::Player(Arc::new(packet));

                for link in &client.links {
                    link.send(AzaleaEvent::Chat(profile.0.clone(), packet.clone()))?
                }
            }
            _ => {}
        }
        Ok(())
    }
}

// Add channel and systems to Bevy
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
    UnusedEnum,
}

#[derive(Debug, Clone)]
pub enum PluginEvent {
    Chat(String, String),
    UnusedEnum,
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

// Create ChatPackets
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

// Limit message length to 254 characters
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
