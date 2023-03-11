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
    query::With,
    schedule::IntoSystemDescriptor,
    system::{Query, Res, ResMut},
};
use azalea_world::entity::Local;
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
                .before("healthcheck_checker"),
        )
        .add_system(
            healthcheck_checker
                .label("healthcheck_checker")
                .before("healthcheck_server")
                .after("healthcheck_listener"),
        )
        .add_system(
            healthcheck_server
                .label("healthcheck_server")
                .after("healthcheck_listener")
                .after("healthcheck_checker"),
        )
        .init_resource::<HealthCheckStatus>()
        .init_resource::<HealthCheckTimer>();
    }
}

// Store usernames and statuscodes
#[derive(Debug, Clone, Default, Deref, DerefMut, Resource)]
struct HealthCheckStatus(HashMap<String, StatusCode>);

// Store usernames and timestamps
#[derive(Debug, Clone, Default, Deref, DerefMut, Resource)]
struct HealthCheckTimer(HashMap<String, Instant>);

// Listen for KeepAliveEvents
// and update timestamps
fn healthcheck_listener(
    mut events: EventReader<KeepAliveEvent>,
    mut timer: ResMut<HealthCheckTimer>,
    query: Query<&GameProfileComponent, With<Local>>,
) {
    for event in events.iter() {
        if let Ok(profile) = query.get_component::<GameProfileComponent>(event.entity) {
            timer.insert(profile.name.clone(), Instant::now());
        };
    }
}

// Check timestamps and update status codes
fn healthcheck_checker(mut status: ResMut<HealthCheckStatus>, timer: Res<HealthCheckTimer>) {
    for (username, timer) in timer.iter() {
        // If it's been ten seconds since a keepalive packet,
        // indicate a problem by setting player status to NOT_FOUND 404
        if timer.elapsed().as_secs() > 15 {
            status.insert(username.clone(), StatusCode(404));
        // If the timer is below that and they receive
        // a keepalive set player status back to OK 200
        } else if let Some(&StatusCode(404)) = status.get(username) {
            status.insert(username.clone(), StatusCode(200));
        // If the player is not in the list, add them
        } else if status.get(username).is_none() {
            status.insert(username.clone(), StatusCode(200));
        }
    }
}

// Store the Server object as a Resource
#[derive(Resource, Deref, DerefMut)]
struct HealthCheckServer(tiny_http::Server);

// Complete all incoming http requests
fn healthcheck_server(server: Res<HealthCheckServer>, status: Res<HealthCheckStatus>) {
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

        // Respond with code from HashMap
        if let Some(code) = status.get(url.trim_start_matches("/status/")) {
            drop(request.respond(Response::new_empty(*code)));
        // If not in HashMap, respond NOT_FOUND 404
        } else {
            drop(request.respond(Response::new_empty(StatusCode(404))));
        }
    }
}
