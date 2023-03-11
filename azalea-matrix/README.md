# Matrix Bot

Example Usage:
```
#[tokio::main]
async fn main() -> anyhow::Result<()> {

    let matrix_plugin = MatrixPlugin::new(
        "server_url",
        "server_name",
        "path-to-appservice-registration.yaml",
        "!room-id:matrix_server",
        vec!["Bot Name", "Spammers"],
        Some("Name of Account in Matrix".to_string()),
        // Not supported yet
        // Some("Path to Profile Picture".to_string())
        None,
    )
    .await?;

    ClientBuilder::new()
        .add_plugin(matrix_plugin)
        .set_handler(handle_client)
        .start(Account::offline("Azalea"), "localhost")
        .await?;

    Ok(())
}
```
