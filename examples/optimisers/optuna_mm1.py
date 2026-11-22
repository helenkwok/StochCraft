"""Optuna, asking and telling, chooses the mean service time of an M/M/1 server.

Part of StochCraft (MIT OR Apache-2.0); see docs/user-guide.md, "Drive the runs from an
optimiser of your own". Arrivals come at rate 1 and the server's mean service time s is the
decision, in [0.2, 0.9]. The cost is the mean number at the server, s / (1 - s), minus 4 s (a
slower machine is cheaper): least at s = 0.5, where it is -1.

Each trial first gets replications 0-4; Optuna's median pruner stops it there when it is worse
than the median of earlier trials at that stage, and otherwise it gets replications 5-9 too.
Every trial uses the same seed, so the same random numbers (common random numbers).

    python optuna_mm1.py path/to/stochcraft-cli
"""

import os
import sys

import optuna

from stochcraft_evaluate import Evaluator

HERE = os.path.dirname(os.path.abspath(__file__))


def cost(answer, s):
    """The cost in each replication: its mean number at the server minus 4 s."""
    return [w - 4 * s for w in answer["results"][0]["values"]]


def main():
    cli = sys.argv[1] if len(sys.argv) > 1 else "stochcraft-cli"
    model = os.path.join(HERE, "mm1-cost.bpmn")
    evaluator = Evaluator(
        cli,
        model,
        results=["server.mean_work_in_progress"],
        ranges=["server.mean=0.2..0.9"],
    )
    optuna.logging.set_verbosity(optuna.logging.WARNING)
    study = optuna.create_study(
        direction="minimize",
        sampler=optuna.samplers.TPESampler(seed=1),
        pruner=optuna.pruners.MedianPruner(n_startup_trials=5),
    )
    for _ in range(40):
        trial = study.ask()
        s = trial.suggest_float("s", 0.2, 0.9)
        costs = cost(evaluator.evaluate({"server.mean": s}, first=0, count=5), s)
        trial.report(sum(costs) / len(costs), step=0)
        if trial.should_prune():
            study.tell(trial, state=optuna.trial.TrialState.PRUNED)
            continue
        # The next five replications: with the first five, exactly what ten at once give.
        costs += cost(evaluator.evaluate({"server.mean": s}, first=5, count=5), s)
        study.tell(trial, sum(costs) / len(costs))
    evaluator.close()

    # The best of the trials that ran to the end (a pruned trial is never the answer).
    best = study.best_trial
    s, value = best.params["s"], best.value
    pruned = len(study.get_trials(states=[optuna.trial.TrialState.PRUNED]))
    print(f"Optuna: s = {s:.4f}, cost {value:.4f} ({pruned} of 40 trials pruned)")
    if abs(s - 0.5) > 0.05:
        print("expected s within 0.05 of 0.5", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
