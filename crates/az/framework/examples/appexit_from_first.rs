//! Does an `AppExit::error()` written from a system in `First` actually stop a
//! `MinimalPlugins` app? Mirrors the world-server's failure path exactly.

use bevy::app::{AppExit, ScheduleRunnerPlugin};
use bevy::prelude::*;

#[derive(Resource, Default)]
struct Ticks(u32);

fn failing_loader(mut ticks: ResMut<Ticks>, mut exit: MessageWriter<AppExit>) {
    ticks.0 += 1;
    if ticks.0 == 1 {
        println!("tick {}: writing AppExit::error()", ticks.0);
        exit.write(AppExit::error());
        return;
    }
    println!("tick {}: still running AFTER the exit was written", ticks.0);
    assert!(
        ticks.0 <= 20,
        "app never honoured AppExit::error() written from `First`"
    );
}

fn main() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(
        std::time::Duration::from_millis(1),
    )));
    app.init_resource::<Ticks>();
    app.add_systems(First, failing_loader);
    let exit = app.run();
    println!("runner returned: {exit:?}");
}
