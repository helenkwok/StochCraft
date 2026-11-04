//! Writing a model and reading it back gives the same model; opened files save unchanged.

use stochcraft_blocks::Builder;
use stochcraft_io::{Format, read, write_new};
use stochcraft_model::{Dist, Experiment, Model, TimeUnit};

/// A model using every element kind and every distribution.
pub fn showcase() -> Model {
    let mut m = Builder::new();
    let src = m.source(
        "arrivals",
        Dist::Triangular {
            min: 1.0,
            mode: 2.0,
            max: 4.5,
        },
        Some(500),
    );
    let cut = m.server(
        "cut",
        2,
        Dist::Gamma {
            shape: 2.5,
            scale: 0.4,
        },
    );
    let mut previous = cut;
    m.connect(src, cut);
    let delays = [
        Dist::Constant(1.25),
        Dist::Exponential { mean: 0.75 },
        Dist::Uniform { min: 0.5, max: 1.5 },
        Dist::Normal {
            mean: 3.0,
            std_dev: 0.5,
        },
        Dist::LogNormal {
            mean: 2.0,
            std_dev: 0.3,
        },
        Dist::TruncatedNormal {
            mean: 2.0,
            std_dev: 1.0,
            min: 1.0,
            max: 1000.0,
        },
        Dist::Beta {
            shape: 2.0,
            scale: 5.0,
        },
        Dist::Weibull {
            shape: 1.5,
            scale: 2.0,
        },
        Dist::Erlang { mean: 6.0, k: 3 },
        Dist::Poisson { mean: 4.0 },
        Dist::Binomial {
            probability: 0.3,
            trials: 10,
        },
        Dist::User {
            points: vec![(0.2, 1.0), (0.5, 4.0), (0.3, 10.0)],
            discrete: true,
        },
        Dist::User {
            points: vec![(0.0, 0.0), (0.5, 2.0), (0.5, 10.0)],
            discrete: false,
        },
    ];
    for (i, d) in delays.into_iter().enumerate() {
        let step = m.delay(&format!("step-{i}"), d);
        m.connect(previous, step);
        previous = step;
    }
    let out = m.sink("exit");
    m.connect(previous, out);
    let mut model = m.build(Experiment {
        time_unit: TimeUnit::Hours,
        warmup: 2.5,
        duration: 40.0,
        replications: 7,
        seed: u64::MAX - 3,
    });
    model.name = "Showcase & <friends>".into();
    model
}

#[test]
fn written_models_read_back_identically() {
    let model = showcase();
    let bytes = write_new(&model);
    let loaded = read(&bytes);
    assert!(loaded.is_ok(), "{loaded:?}");
    let Ok(loaded) = loaded else { return };
    assert_eq!(loaded.opened.format, Format::Bpmn);
    assert!(loaded.notes.is_empty(), "{:#?}", loaded.notes);
    assert_eq!(loaded.model, model);
    assert!(model.validate().is_empty());
}

#[test]
fn added_ids_never_collide_with_model_ids() {
    let mut m = Builder::new();
    // Names that are also the ids StochCraft adds to the file.
    let src = m.source("definitions", Dist::Constant(1.0), None);
    let srv = m.server("process", 1, Dist::Constant(0.5));
    let out = m.sink("scenario");
    m.connect(src, srv);
    m.connect(srv, out);
    let model = m.build(Experiment::default());
    let text = String::from_utf8(write_new(&model)).unwrap_or_default();
    let mut ids: Vec<&str> = text
        .split(" id=\"")
        .skip(1)
        .filter_map(|s| s.split('"').next())
        .collect();
    let n = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), n, "duplicate ids in:\n{text}");
    assert_eq!(read(text.as_bytes()).map(|l| l.model).ok(), Some(model));
}

#[test]
fn opened_files_save_unchanged_and_with_edits() {
    let bytes = write_new(&showcase());
    let Ok(loaded) = read(&bytes) else {
        unreachable!()
    };
    assert_eq!(loaded.opened.save(&loaded.model), Ok(bytes.clone()));
    let mut edited = loaded.model.clone();
    edited.experiment.replications += 1;
    edited.experiment.warmup = 0.0;
    edited.elements.pop();
    edited.connections.pop();
    let saved = loaded.opened.save(&edited);
    let back = saved.as_deref().map(|b| read(b).map(|l| l.model));
    assert_eq!(back, Ok(Ok(edited)));
}
