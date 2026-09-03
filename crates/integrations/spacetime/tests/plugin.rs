use bevy::prelude::*;
use spacetime_plugin::{SpacetimeClient, SpacetimePlugin};
use std::sync::Arc;
use std::sync::Mutex;

#[derive(Default)]
struct TestConnection;

#[test]
fn plugin_tolerates_a_client_inserted_later() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(SpacetimePlugin::<TestConnection>::default());
    app.update();

    let reporter = Arc::new(Mutex::new(None));
    let reporter_for_connect = reporter.clone();
    app.insert_resource(SpacetimeClient::frame_driven(
        move |callback_reporter| {
            *reporter_for_connect.lock().unwrap() = Some(callback_reporter);
            Ok::<_, std::io::Error>(TestConnection)
        },
        |_connection| Ok::<_, std::convert::Infallible>(()),
    ));
    app.update();
    assert!(
        !app.world()
            .resource::<SpacetimeClient<TestConnection>>()
            .is_ready()
    );

    let reporter = reporter.lock().unwrap().clone().unwrap();
    reporter.subscriptions_ready();
    reporter.connected();
    app.update();
    assert!(
        app.world()
            .resource::<SpacetimeClient<TestConnection>>()
            .is_ready()
    );
}
