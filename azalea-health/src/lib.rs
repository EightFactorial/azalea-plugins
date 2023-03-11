use std::{
    collections::HashMap,
    net::{AddrParseError, SocketAddr},
    time::Instant,
};

use azalea_client::{
    ecs::{
        app::{App, Plugin},
        system::Resource,
    },
    packet_handling::KeepAliveEvent,
    GameProfileComponent,
};
use azalea_ecs::{
    event::EventReader,
    schedule::IntoSystemDescriptor,
    system::{Query, Res, ResMut},
};
use derive_more::{Deref, DerefMut};
use tiny_http::{Method, Response, StatusCode};

// This is the plugin
#[derive(Debug, Clone)]
pub struct HealthCheck {
    pub addr: SocketAddr,
}

// You should use this function
// to create the plugin
impl HealthCheck {
    pub fn new(address: &str, port: u16) -> Result<Self, AddrParseError> {
        Ok(Self {
            addr: SocketAddr::new(address.parse()?, port),
        })
    }
}
impl Plugin for HealthCheck {
    fn build(&self, app: &mut App) {
        let server = tiny_http::Server::http(self.addr)
            .unwrap_or_else(|_| panic!("HealthCheck is unable to bind to {}", self.addr));
        app.insert_resource(HealthCheckServer(server));

        app.add_system(
            healthcheck_listener
                .label("healthcheck_listener")
        )
        .add_system(
            healthcheck_server
                .label("healthcheck_server")
                .after("healthcheck_listener")
        )
        .init_resource::<HealthCheckTimer>();
    }
}

// Store usernames and timestamps
#[derive(Debug, Clone, Default, Deref, DerefMut, Resource)]
struct HealthCheckTimer(HashMap<String, Instant>);

// Listen for KeepAliveEvents
// and update timestamps
fn healthcheck_listener(
    mut events: EventReader<KeepAliveEvent>,
    mut timer: ResMut<HealthCheckTimer>,
    query: Query<&GameProfileComponent>,
) {
    for event in events.iter() {
        if let Ok(profile) = query.get_component::<GameProfileComponent>(event.entity) {
            timer.insert(profile.name.clone(), Instant::now());
        };
    }
}

// Store the Server object as a Resource
#[derive(Resource, Deref, DerefMut)]
struct HealthCheckServer(tiny_http::Server);

// Complete all incoming http requests
fn healthcheck_server(server: Res<HealthCheckServer>, status: Res<HealthCheckTimer>) {
    while let Ok(Some(request)) = server.try_recv() {
        // Only respond to GET and HEAD requests
        match request.method() {
            Method::Get => {}
            Method::Head => {}
            _ => {
                drop(request.respond(Response::new_empty(StatusCode(400))));
                return;
            }
        }

        // URL Cleanup
        let url = request.url().trim().trim_end_matches('/').to_string();

        // Respond to general health check OK 200
        if url == "/health" {
            drop(request.respond(Response::new_empty(StatusCode(200))));
            return;
        }
        // If not asking about player, respond BAD_REQUEST 400
        if !url.starts_with("/status/") {
            drop(request.respond(Response::new_empty(StatusCode(400))));
            return;
        }

        // Respond with code based on timestamp
        if let Some(time) = status.get(url.trim_start_matches("/status/")) {
            let code = if time.elapsed().as_secs() < 15 {
                StatusCode::from(200)
            } else {
                StatusCode::from(404)
            };
            
            drop(request.respond(Response::new_empty(code)));
        // If not in HashMap, respond NOT_FOUND 404
        } else {
            drop(request.respond(Response::new_empty(StatusCode(404))));
        }
    }
}
