//! Does an `AppExit` written while the app is pumped during CONSTRUCTION
//! (no runner, as a synchronous startup loader does) survive into the
//! later `app.run()`? And does re-asserting it each frame make it survive?

use bevy::app::{AppExit, ScheduleRunnerPlugin};
use bevy::prelude::*;

#[derive(Resource)]
struct Loader {
    failed: bool,
    reassert: bool,
}

fn loader_system(mut loader: ResMut<Loader>, mut exit: MessageWriter<AppExit>) {
    if loader.failed {
        if loader.reassert {
            exit.write(AppExit::error());
        }
        return;
    }
    loader.failed = true;
    exit.write(AppExit::error());
}

fn scenario(reassert: bool, pumps: u32) -> AppExit {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(
        std::time::Duration::from_millis(1),
    )));
    app.insert_resource(Loader {
        failed: false,
        reassert,
    });
    app.add_systems(First, loader_system);
    for _ in 0..pumps {
        app.update();
    }
    app.run()
}

fn main() {
    for pumps in [1u32, 2, 3, 5, 50, 1000] {
        let without = scenario(false, pumps);
        let with = scenario(true, pumps);
        println!("pumps={pumps:>5}  without re-assert: {without:?}   with re-assert: {with:?}");
    }
}
