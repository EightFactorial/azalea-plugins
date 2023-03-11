use azalea_bridge::{ClientSide, PluginBridge};

mod discord;

#[derive(Debug, Clone)]
pub struct DiscordPlugin;

impl DiscordPlugin {
    pub async fn new(
        bot_token: &str,
        webhook_token: &str,
        webhook_id: u64,
        ignore_list: Vec<&str>,
    ) -> ClientSide<DiscordPlugin> {
        let ignore = ignore_list.iter().map(|s| s.to_string()).collect();

        // Create commucation channel
        let bridge = PluginBridge::<DiscordPlugin>::new(ignore);

        // Spawn Discord bot
        tokio::spawn(discord::main(
            bot_token.to_string(),
            webhook_token.to_string(),
            webhook_id,
            bridge.plugin,
        ));

        // Return a 'ClientSide' Plugin to insert into Azalea
        bridge.client
    }
}
