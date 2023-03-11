use azalea_bridge::{ClientSide, IntoEvent, PluginBridge};

mod discord;

#[derive(Debug, Clone)]
pub struct DiscordPlugin;

impl DiscordPlugin {
    pub async fn new(
        token: &str,
        channel_id: u64,
        ignore_list: Vec<&str>,
    ) -> ClientSide<DiscordEvent> {
        let ignore = ignore_list.iter().map(|s| s.to_string()).collect();

        // Create commucation channel
        let bridge = PluginBridge::<DiscordEvent>::new(ignore);

        tokio::spawn(discord::main(token.to_string(), channel_id, bridge.plugin));

        bridge.client
    }
}

#[derive(Debug, Clone)]
pub struct DiscordEvent {
    pub username: String,
    pub message: String,
}

impl IntoEvent for DiscordEvent {
    fn chat(&self) -> (String, String) {
        (self.username.clone(), self.message.clone())
    }
}
