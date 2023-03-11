use flume::{Receiver, Sender};
use log::{error, warn, info};
use matrix_sdk::{
    event_handler::Ctx,
    room::Room,
    ruma::{events::room::{message::{MessageType, OriginalSyncRoomMessageEvent, RoomMessageEventContent, TextMessageEventContent}, member::{OriginalSyncRoomMemberEvent, MembershipState}, join_rules::JoinRule}, UserId, api::{client::error::ErrorKind, appservice::{Namespace, Namespaces}}}, config::SyncSettings,
};
use matrix_sdk_appservice::AppService;
use uuid::Uuid;

use crate::{AzaleaEvent, MatrixEvent};

pub(crate) async fn startup(
    bot_name: Option<String>,
    _bot_image: Option<String>,
    target_room: String,
    appservice: AppService,
    tx: Sender<MatrixEvent>,
    rx: Receiver<AzaleaEvent>,
) -> anyhow::Result<()> {
    appservice
        .register_user_query(Box::new(|_, req| Box::pin(async move {
            info!("Got request for {}", req.user_id);
            true
         })))
        .await;
 
    let client = appservice.user(None).await?;
    client.sync_once(SyncSettings::default()).await?;

    // Set display name if needed
    if let Some(name) = bot_name {
        let account = client.account();
        if let Some(current) = account.get_display_name().await? {
            if name != current {
                account.set_display_name(Some(&name)).await?;
            }
        } else {
            account.set_display_name(Some(&name)).await?;
        }
    }

    // TODO: Support setting bot profile picture
    // let url = client.account().upload_avatar(Mime, Vec::new()).await?;
    // client.account().set_avatar_url(Some(url)).await?;

    // Add context for later
    client.add_event_handler_context(appservice.clone());
    client.add_event_handler_context(tx);

    // Handle room invites
    client.add_event_handler(mx_room_handler);

    // Get target room
    let room = get_room(target_room, client.clone()).unwrap();

    // Join if invited
    if let Some(invited) = client.get_invited_room(room.room_id()) {
        client.join_room_by_id(invited.room_id()).await?;
    }

    client.add_event_handler_context(room.clone());

    // Handle room events
    room.add_event_handler(mx_message_handler);

    // Get namespace
    let namespace = get_namespace(appservice.registration().namespaces.clone()).unwrap();

    // Listen for events from channel
    tokio::spawn(mc_message_handler(appservice.clone(), namespace, room, rx));

    // Run AppService
    let (host, port) = appservice.registration().get_host_and_port().unwrap();
    appservice.run(host, port).await?;
    
    Ok(())
}

async fn mc_message_handler(appservice: AppService, namespace: Namespace, room: Room, rx: Receiver<AzaleaEvent>) -> anyhow::Result<()> {
    // Listen for messages from Plugin
    while let Ok(event) = rx.recv_async().await {
        match event {
            // Chat messages
            AzaleaEvent::Chat(profile, packet) => {
                let username = if let Some(username) = packet.username() { username } else { "Server".to_string() };
                let uuid = if let Some(uuid) = packet.uuid() { uuid } else { Uuid::default() };

                // Kind of gross but does the job?
                let localpart = format!("{}{}", namespace.regex.trim_start_matches('@').trim_end_matches(".*"), uuid.to_string().replace('-', "_"));

                // Register the user if using for the first time
                if !appservice.users().contains_key(&localpart) {
                    if let Err(e) = appservice.register_user(&localpart, None).await {
                        // Do not error if the user is already in use
                        if let Err(e) = should_error(e) {
                            error!("{e:?}");
                            continue;
                        }
                    }    
                }

                // Get the user
                let user = appservice.user(Some(&localpart)).await.unwrap();

                // Sync the first time the user is used
                if user.rooms().is_empty() {
                    user.sync_once(SyncSettings::default()).await.unwrap();

                    // Set profile picture on sync
                    let account = user.account();
                    if let Some(_profile) = profile {
                        if let Ok(icon) = account.get_avatar_url().await {
                            if matches!(icon, None) {
                            // TODO: Get property, convert to Vec<u8>, upload, and set avatar url
                            // info!("{:?}", profile);
                            }
                        }
                    }
                }

                // Set username if different
                {
                    let account = user.account();
                    if let Some(current) = account.get_display_name().await.unwrap() {
                        if username != current {
                            account.set_display_name(Some(&username)).await.unwrap();
                        }
                    } else {
                        account.set_display_name(Some(&username)).await.unwrap();
                    }
                }

                // If user hasn't joined the room
                if matches!(user.get_joined_room(room.room_id()), None) {
                    // If the room is not public and the user has not been invited
                    if room.join_rule() != JoinRule::Public && matches!(user.get_invited_room(room.room_id()), None) {
                        // Send invite
                        let Room::Joined(room) = room.clone() else {
                            error!("Bot has not joined room!");
                            continue;
                        };

                        if let Err(e) = room.invite_user_by_id(user.user_id().unwrap()).await {
                            error!("Unable to send bot user an invite to the room: {e}");
                            continue;
                        };

                        // Wait a tiny amount of time
                        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

                        // Sync again, just to make sure the invite was received
                        user.sync_once(SyncSettings::default()).await.unwrap();
                    }

                    // Join the room / accept invite
                    user.join_room_by_id(room.room_id()).await.unwrap();
               }

                // The room has been joined
                let Room::Joined(room) = user.get_room(room.room_id()).unwrap() else {
                    error!("Bot user has not joined room!");
                    continue;
                };

                // Send message
                let event = RoomMessageEventContent::new(MessageType::Text(TextMessageEventContent::plain(packet.content())));
                room.send(event, None).await.unwrap();
            }
        }
    }
    error!("MatrixPlugin event listener exited!");
    Err(anyhow::Error::msg("Event listener exited!"))
}

async fn mx_message_handler(
    event: OriginalSyncRoomMessageEvent,
    room: Room,
    appservice: Ctx<AppService>,
    tx: Ctx<Sender<MatrixEvent>>,
) {
    if appservice.user_id_is_in_namespace(&event.sender) {
        return;
    }
    match event.content.msgtype {
        MessageType::Text(message) => {
            // Get the sender
            let Ok(Some(sender)) = room.get_member(&event.sender).await else { 
                warn!("MatrixPlugin was unable to get message sender");
                return
            };

            // Get a username
            let username = if let Some(displayname) = sender.display_name() {
                displayname
            } else {
                sender.name()
            }.to_string();

            // Send message to Plugin
            drop(tx.send_async(MatrixEvent::Chat(username, message.body)).await);
        }
        _ => {}
    }
}

// Sending an invitation doesn't seem to trigger an event
// but revoking an invitation does?
async fn mx_room_handler(
    event: OriginalSyncRoomMemberEvent,
    room: Room,
    target_room: Ctx<Room>,
    appservice: Ctx<AppService>
) -> anyhow::Result<()> {

    if room.room_id() != target_room.room_id() {
        // Check to make sure bot only responds to room id specified in PluginBuilder
        warn!("Got event for wrong room: {:?}", room.room_id());
    } else if !appservice.user_id_is_in_namespace(&event.state_key) {
        // Check if the AppService manages this user
        warn!("User not in namespace: {}", event.state_key);
    } else {
        match event.content.membership {
            // If the event is an invite
            MembershipState::Invite => {
                // Get the user id
                let user_id = UserId::parse(event.state_key.as_str())?;

                // Register the user if using for the first time
                if !appservice.users().contains_key(user_id.localpart()) {
                    if let Err(e) = appservice.register_user(user_id.localpart(), None).await {
                        // Do not error if the user is already in use
                        if let Err(e) = should_error(e) {
                            error!("{e:?}");
                            return Err(e);
                        }
                    }    
                }

                // Get the client
                let client = appservice.user(Some(user_id.localpart())).await.unwrap();

                // Join the room
                client.join_room_by_id(room.room_id()).await.unwrap();         
            },
            _ => {},
        }    
    }
    Ok(())
}

// Do not error if the user is in use
fn should_error(error: matrix_sdk_appservice::Error) -> anyhow::Result<()> {
        if let matrix_sdk_appservice::Error::Matrix(error) = error {
            return if error.client_api_error_kind() == Some(&ErrorKind::UserInUse) {
                Ok(())
            } else {
                Err(error.into())
            }
        }
        Err(error.into())
}

// Yeah, it's not a great solution is it?
// Gets the *last* 'valid' namespace
fn get_namespace(namespaces: Namespaces) -> Result<Namespace, anyhow::Error> {
    let mut namespace: Option<Namespace> = None;
    for space in namespaces.users.iter() {
        if space.regex.ends_with(".*") {
            namespace = Some(space.clone());
            break;
        }
    }

    let Some(namespace) = namespace else {
        error!("No valid namespace for player accounts!");
        return Err(anyhow::Error::msg("No valid namespace for player accounts"));
    };
    Ok(namespace)
}

// It's gotta be in here somewhere, right?
fn get_room(target: String, client: matrix_sdk::Client) -> Result<Room, anyhow::Error> {
    for room in client.rooms() {
        if *room.room_id() == target {
            return Ok(room);
        }
    }

    for room in client.joined_rooms() {
        if *room.room_id() == target {
            return Ok(Room::Joined(room));
        }
    }

    for room in client.invited_rooms() {
        if *room.room_id() == target {
            return Ok(Room::Invited(room));
        }
    }

    for room in client.left_rooms() {
        if *room.room_id() == target {
            return Ok(Room::Left(room));
        }
    }

    error!("Bot is not in target room");
    Err(anyhow::Error::msg("Bot is not in target room"))
}