# HealthCheck

This plugin should be mostly stable.

Serves an HTTP server that responds OK 200 on /health, and individual checks on any bot at /status/username

Example Usage:
```
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let healthcheck = HealthCheck::new("127.0.0.1", 8080)?;

    ClientBuilder::new()
        .add_plugin(healthcheck)
        .set_handler(handle_client)
        .start(Account::offline("Azalea"), "localhost")
        .await?;

    Ok(())
}
```
