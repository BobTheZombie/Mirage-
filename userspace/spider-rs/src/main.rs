use spider_rs::{SpiderManager, StubSpawner};
use std::process::ExitCode;

fn main() -> ExitCode {
    println!("Spider-rs userspace init scaffold for GNU/Mirage");
    println!("mode: host/userspace stub; no Mirage PID 1 process ABI is claimed yet");

    let mut manager = SpiderManager::new();
    match manager.load_search_paths() {
        Ok(count) => println!("loaded {count} unit(s)"),
        Err(error) => {
            eprintln!("failed to load units: {error:?}");
            return ExitCode::FAILURE;
        }
    }

    let plan = match manager.resolve_default() {
        Ok(plan) => plan,
        Err(error) => {
            eprintln!("failed to resolve default.target: {error:?}");
            return ExitCode::FAILURE;
        }
    };

    println!("startup target: {}", plan.target);
    for (index, unit) in plan.order.iter().enumerate() {
        println!("{:02}: {unit}", index + 1);
    }

    let spawner = StubSpawner::default();
    let outcomes = manager.start_plan(&plan, &spawner);
    for outcome in outcomes {
        println!(
            "{}: {:?} ({})",
            outcome.name, outcome.state, outcome.message
        );
    }
    for entry in spawner.entries() {
        println!("{entry}");
    }

    ExitCode::SUCCESS
}
