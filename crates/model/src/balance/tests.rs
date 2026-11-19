#![allow(clippy::panic)]

use super::*;

/// Unwraps a value a test expects to be there, failing the test with the error otherwise.
trait Must<T> {
    fn must(self) -> T;
}

impl<T, E: fmt::Debug> Must<T> for Result<T, E> {
    fn must(self) -> T {
        match self {
            Ok(v) => v,
            Err(e) => panic!("expected Ok, got {e:?}"),
        }
    }
}

impl<T> Must<T> for Option<T> {
    fn must(self) -> T {
        match self {
            Some(v) => v,
            None => panic!("expected Some"),
        }
    }
}

fn el(id: &str, time: f64, preds: &[&str]) -> WorkElement {
    WorkElement {
        id: ElementId::unchecked(id),
        time,
        predecessors: preds.iter().map(|p| ElementId::unchecked(p)).collect(),
    }
}

fn names(s: &Station) -> Vec<&str> {
    s.elements.iter().map(ElementId::as_str).collect()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9
}

/// Six elements with precedences, worked by hand for cycle time 10. Work content 24, so the
/// theoretical minimum is 3.
///
/// A(4) ─→ B(4)      C(5) ─→ D(3)      E(6) ─→ F(2)
fn six() -> Vec<WorkElement> {
    vec![
        el("A", 4.0, &[]),
        el("B", 4.0, &["A"]),
        el("C", 5.0, &[]),
        el("D", 3.0, &["C"]),
        el("E", 6.0, &[]),
        el("F", 2.0, &["E"]),
    ]
}

#[test]
fn largest_candidate_rule_on_the_roadmap_example() {
    // Times 6, 5, 3, 2, 2, 2 with cycle 10, no precedence: three stations by the largest
    // candidate rule (6 + 3, 5 + 2 + 2, 2), where two suffice (6 + 2 + 2, 5 + 3 + 2).
    let elements: Vec<WorkElement> = [6.0, 5.0, 3.0, 2.0, 2.0, 2.0]
        .iter()
        .enumerate()
        .map(|(i, &t)| el(&format!("e{i}"), t, &[]))
        .collect();
    let b = balance(&elements, 10.0, Heuristic::LargestCandidate).must();
    let shape: Vec<(Vec<&str>, f64)> = b.stations.iter().map(|s| (names(s), s.time)).collect();
    assert_eq!(
        shape,
        vec![
            (vec!["e0", "e2"], 9.0),
            (vec!["e1", "e3", "e4"], 9.0),
            (vec!["e5"], 2.0),
        ]
    );
    let c = compare(&elements, 10.0).must();
    assert_eq!(c.theoretical_min, 2);
    assert!(close(c.work_content, 20.0));
}

#[test]
fn six_elements_worked_by_hand() {
    let c = compare(&six(), 10.0).must();
    assert_eq!(c.theoretical_min, 3);
    assert!(close(c.work_content, 24.0));

    // Largest candidate: E(6) then A(4) fill station 1 exactly; C(5) then B(4) fill station 2
    // to 9; D(3) and F(2) fill station 3.
    let lcr = &c.balances[0];
    assert_eq!(lcr.heuristic, Heuristic::LargestCandidate);
    let shape: Vec<(Vec<&str>, f64)> = lcr.stations.iter().map(|s| (names(s), s.time)).collect();
    assert_eq!(
        shape,
        vec![
            (vec!["E", "A"], 10.0),
            (vec!["C", "B"], 9.0),
            (vec!["D", "F"], 5.0),
        ]
    );
    assert!(close(lcr.efficiency, 0.8));
    assert!(close(lcr.delay, 0.2));
    assert!(close(lcr.smoothness, 26f64.sqrt()));

    // Kilbridge and Wester: column 0 is A, C, E; then column 1 is B, D, F.
    let kw = &c.balances[1];
    assert_eq!(kw.heuristic, Heuristic::KilbridgeWester);
    let shape: Vec<(Vec<&str>, f64)> = kw.stations.iter().map(|s| (names(s), s.time)).collect();
    assert_eq!(
        shape,
        vec![
            (vec!["A", "C"], 9.0),
            (vec!["E", "B"], 10.0),
            (vec!["D", "F"], 5.0),
        ]
    );

    // Ranked positional weight: A, C and E weigh 8 (own time plus the follower's), B 4, D 3,
    // F 2; the same stations as Kilbridge and Wester here.
    let rpw = &c.balances[2];
    assert_eq!(rpw.heuristic, Heuristic::RankedPositionalWeight);
    let shape: Vec<(Vec<&str>, f64)> = rpw.stations.iter().map(|s| (names(s), s.time)).collect();
    assert_eq!(
        shape,
        vec![
            (vec!["A", "C"], 9.0),
            (vec!["E", "B"], 10.0),
            (vec!["D", "F"], 5.0),
        ]
    );
    assert!(close(rpw.efficiency, 0.8));
}

#[test]
fn positional_weights_count_each_follower_once() {
    // A is followed by B (through one path) and by D through B's other branch: the weight of A
    // counts each follower once.
    let elements = vec![
        el("A", 1.0, &[]),
        el("B", 2.0, &["A"]),
        el("C", 3.0, &["A"]),
        el("D", 4.0, &["B", "C"]),
    ];
    let w = positional_weights(&elements).must();
    let weights: Vec<(&str, f64)> = w.iter().map(|(id, w)| (id.as_str(), *w)).collect();
    assert_eq!(
        weights,
        vec![("A", 10.0), ("B", 6.0), ("C", 7.0), ("D", 4.0)]
    );
}

#[test]
fn kilbridge_and_wester_columns_are_precedence_levels() {
    let elements = six();
    let g = Graph::new(&elements).must();
    assert_eq!(g.columns(), vec![0, 1, 0, 1, 0, 1]);

    // A chain of three, and an element after two branches of different lengths.
    let chain = vec![
        el("X", 1.0, &[]),
        el("Y", 1.0, &["X"]),
        el("Z", 1.0, &["Y"]),
        el("W", 1.0, &["X", "Z"]),
    ];
    let g = Graph::new(&chain).must();
    assert_eq!(g.columns(), vec![0, 1, 2, 3]);
}

#[test]
fn times_that_add_to_the_cycle_fit_despite_rounding() {
    // 0.1 + 0.2 + 0.7 is 1.0000000000000002 in floating point. It fills a station of cycle 1.0
    // and needs one station, not two.
    let elements = vec![el("a", 0.1, &[]), el("b", 0.2, &[]), el("c", 0.7, &[])];
    let c = compare(&elements, 1.0).must();
    assert_eq!(c.theoretical_min, 1);
    for b in &c.balances {
        assert_eq!(b.stations.len(), 1, "{:?}", b.heuristic);
    }
}

#[test]
fn a_repeated_predecessor_is_one_predecessor() {
    let elements = vec![el("A", 1.0, &[]), el("B", 1.0, &["A", "A"])];
    let b = balance(&elements, 10.0, Heuristic::LargestCandidate).must();
    assert_eq!(b.stations.len(), 1);
    assert_eq!(names(&b.stations[0]), vec!["A", "B"]);
}

#[test]
fn an_element_longer_than_the_cycle_is_an_error_and_one_equal_to_it_is_not() {
    let too_long = vec![el("A", 10.5, &[])];
    assert_eq!(
        compare(&too_long, 10.0),
        Err(BalanceError::TooLong {
            element: ElementId::unchecked("A"),
            time: 10.5,
            cycle: 10.0,
        })
    );
    let exact = vec![el("A", 10.0, &[])];
    assert!(compare(&exact, 10.0).is_ok());
}

#[test]
fn bad_input_is_reported_not_balanced() {
    assert_eq!(compare(&[], 10.0), Err(BalanceError::NoElements));
    assert_eq!(
        compare(&[el("A", 1.0, &[])], 0.0),
        Err(BalanceError::CycleNotPositive(0.0))
    );
    assert!(matches!(
        compare(&[el("A", 1.0, &[])], f64::NAN),
        Err(BalanceError::CycleNotPositive(_))
    ));
    assert_eq!(
        compare(&[el("A", 1.0, &[]), el("A", 2.0, &[])], 10.0),
        Err(BalanceError::DuplicateElement(ElementId::unchecked("A")))
    );
    assert_eq!(
        compare(&[el("A", 0.0, &[])], 10.0),
        Err(BalanceError::BadTime {
            element: ElementId::unchecked("A"),
            time: 0.0,
        })
    );
    assert_eq!(
        compare(&[el("A", f64::INFINITY, &[])], 10.0),
        Err(BalanceError::BadTime {
            element: ElementId::unchecked("A"),
            time: f64::INFINITY,
        })
    );
    assert_eq!(
        compare(&[el("A", 1.0, &["Q"])], 10.0),
        Err(BalanceError::UnknownPredecessor {
            element: ElementId::unchecked("A"),
            predecessor: ElementId::unchecked("Q"),
        })
    );
}

#[test]
fn a_loop_of_precedences_is_reported() {
    let two = vec![el("A", 1.0, &["B"]), el("B", 1.0, &["A"])];
    assert_eq!(
        compare(&two, 10.0),
        Err(BalanceError::PrecedenceLoop(ElementId::unchecked("A")))
    );
    let itself = vec![el("A", 1.0, &["A"])];
    assert_eq!(
        compare(&itself, 10.0),
        Err(BalanceError::PrecedenceLoop(ElementId::unchecked("A")))
    );
    // C is not on the loop but comes after it, so it cannot be ordered either.
    let after = vec![
        el("C", 1.0, &["A"]),
        el("A", 1.0, &["B"]),
        el("B", 1.0, &["A"]),
    ];
    assert_eq!(
        compare(&after, 10.0),
        Err(BalanceError::PrecedenceLoop(ElementId::unchecked("C")))
    );
}

/// A small deterministic generator, so the random instances are the same on every run.
struct Xorshift(u64);

impl Xorshift {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// A random assembly: elements in input order with precedences only to earlier ones, so the
/// graph is acyclic, and times no longer than the cycle.
fn random_instance(rng: &mut Xorshift) -> (Vec<WorkElement>, f64) {
    let cycle = [1.0, 10.0, 7.5][rng.below(3) as usize];
    let n = 1 + rng.below(25) as usize;
    let elements = (0..n)
        .map(|i| {
            let time = cycle * (1 + rng.below(100)) as f64 / 100.0;
            let predecessors = (0..i)
                .filter(|_| rng.below(4) == 0)
                .map(|j| ElementId::unchecked(&format!("e{j}")))
                .collect();
            WorkElement {
                id: ElementId::unchecked(&format!("e{i}")),
                time,
                predecessors,
            }
        })
        .collect();
    (elements, cycle)
}

#[test]
fn every_heuristic_respects_the_rules_on_random_assemblies() {
    let mut rng = Xorshift(0x9e37_79b9_7f4a_7c15);
    for _ in 0..400 {
        let (elements, cycle) = random_instance(&mut rng);
        let c = compare(&elements, cycle).must();
        let work: f64 = elements.iter().map(|e| e.time).sum();
        assert!(close(c.work_content, work));
        assert_eq!(
            c.theoretical_min,
            ((work / cycle - 1e-9).ceil() as usize).max(1)
        );

        for b in &c.balances {
            // Every element is placed exactly once.
            let mut placed: Vec<&str> = b
                .stations
                .iter()
                .flat_map(|s| s.elements.iter().map(ElementId::as_str))
                .collect();
            placed.sort_unstable();
            let mut all: Vec<&str> = elements.iter().map(|e| e.id.as_str()).collect();
            all.sort_unstable();
            assert_eq!(placed, all, "{:?}", b.heuristic);

            // Station times are the sums of their elements and never exceed the cycle.
            let mut station_of = BTreeMap::new();
            let mut position = BTreeMap::new();
            for (s, station) in b.stations.iter().enumerate() {
                let mut sum = 0.0;
                for (p, id) in station.elements.iter().enumerate() {
                    station_of.insert(id.clone(), s);
                    position.insert(id.clone(), p);
                    let e = elements.iter().find(|e| &e.id == id).must();
                    sum += e.time;
                }
                assert!(close(station.time, sum), "{:?}", b.heuristic);
                assert!(station.time <= cycle * (1.0 + 1e-9), "{:?}", b.heuristic);
            }

            // No precedence is broken: a predecessor is at or before the element's station, and
            // earlier in its own station when they share one.
            for e in &elements {
                for p in &e.predecessors {
                    let (sp, se) = (station_of[p], station_of[&e.id]);
                    assert!(sp <= se, "{:?}: {} before {}", b.heuristic, p, e.id);
                    if sp == se {
                        assert!(position[p] < position[&e.id], "{:?}", b.heuristic);
                    }
                }
            }

            // The theoretical minimum is never beaten.
            assert!(b.stations.len() >= c.theoretical_min, "{:?}", b.heuristic);

            // The measures agree with their definitions.
            let longest = b.stations.iter().map(|s| s.time).fold(0.0, f64::max);
            assert!(close(b.longest, longest));
            assert!(close(
                b.efficiency,
                work / (b.stations.len() as f64 * longest)
            ));
            assert!(close(b.delay, 1.0 - b.efficiency));
            let smooth: f64 = b
                .stations
                .iter()
                .map(|s| (longest - s.time) * (longest - s.time))
                .sum::<f64>()
                .sqrt();
            assert!(close(b.smoothness, smooth));
        }
    }
}

#[test]
fn the_same_input_gives_the_same_balance() {
    let mut rng = Xorshift(42);
    for _ in 0..50 {
        let (elements, cycle) = random_instance(&mut rng);
        let first = compare(&elements, cycle).must();
        let second = compare(&elements, cycle).must();
        assert_eq!(first, second);
    }
}
