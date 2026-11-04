//! `cargo run -p stochcraft-io --example inspect -- FILE [SCENARIO]`
//!
//! Reads a BPMN or XPDL model with BPSim data and prints what StochCraft understood.

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("usage: inspect FILE [SCENARIO]");
        std::process::exit(2);
    };
    let scenario = args.next();
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("{path}: {e}");
            std::process::exit(1);
        }
    };
    match stochcraft_io::read_scenario(&bytes, scenario.as_deref()) {
        Ok(l) => {
            println!(
                "{:?}, scenario {:?} of {:?}",
                l.opened.format, l.opened.scenario, l.opened.scenarios
            );
            let x = &l.model.experiment;
            println!(
                "experiment: {} replications, seed {}, warm-up {} + duration {} {}",
                x.replications,
                x.seed,
                x.warmup,
                x.duration,
                x.time_unit.bpsim_name()
            );
            for e in &l.model.elements {
                println!("  {:<40} {:<30} {:?}", e.id.as_str(), e.name, e.kind);
            }
            for c in &l.model.connections {
                println!("  {} -> {}", c.from, c.to);
            }
            for n in &l.notes {
                println!("{n}");
            }
            for d in l.model.validate() {
                println!("model {d}");
            }
        }
        Err(e) => {
            eprintln!("{path}: {e}");
            std::process::exit(1);
        }
    }
}
