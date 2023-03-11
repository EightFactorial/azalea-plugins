use azalea_bridge::{ClientSide, PluginBridge};
use matrix_sdk_appservice::{AppServiceBuilder, AppServiceRegistration};

mod matrix;

#[derive(Debug, Clone)]
pub struct MatrixPlugin;

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
    ) -> Result<ClientSide<MatrixPlugin>, matrix_sdk_appservice::Error> {
        let room = room_id.to_string();
        let list: Vec<String> = ignore_list.iter().map(|s| s.to_string()).collect();

        // Create commucation channel
        let bridge = PluginBridge::<MatrixPlugin>::new(list);

        // Create the AppService
        let appservice = AppServiceBuilder::new(
            server_url.parse()?,
            server_name.parse()?,
            AppServiceRegistration::try_from_yaml_file(registration)?,
        )
        .build()
        .await?;

        // Spawn Matrix bot
        tokio::spawn(matrix::startup(
            bot_name,
            bot_image,
            room,
            appservice,
            bridge.plugin,
        ));

        // Return a 'ClientSide' Plugin to insert into Azalea
        Ok(bridge.client)
    }
}
