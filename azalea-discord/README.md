# Discord Bot

TODO: Escape Text Formatting

Example Usage:
```
#[tokio::main]
async fn main() -> anyhow::Result<()> {

    let discord_plugin = DiscordPlugin::new(
        "Bot Token",
        room_id,
        vec!["Bot Name", "Spammers"],
    )
    .await;

    ClientBuilder::new()
        .add_plugin(discord_plugin)
        .set_handler(handle_client)
        .start(Account::offline("Azalea"), "localhost")
        .await?;

    Ok(())
}
```
