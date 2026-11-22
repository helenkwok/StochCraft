"""pymoo's pattern search chooses the mean service time of an M/M/1 server.

Part of StochCraft (MIT OR Apache-2.0); see docs/user-guide.md, "Drive the runs from an
optimiser of your own". Arrivals come at rate 1 and the server's mean service time s is the
decision, in [0.2, 0.9]. The cost is the mean number at the server, s / (1 - s), minus 4 s (a
slower machine is cheaper): least at s = 0.5, where it is -1.

Every candidate runs replications 0-9 on the same seed, so the same random numbers (common
random numbers): the cost is then a smooth function of s, which a local search follows well.

    python pymoo_mm1.py path/to/stochcraft-cli
"""

import os
import sys

import numpy as np
from pymoo.algorithms.soo.nonconvex.pattern import PatternSearch
from pymoo.core.problem import ElementwiseProblem
from pymoo.optimize import minimize

from stochcraft_evaluate import Evaluator

HERE = os.path.dirname(os.path.abspath(__file__))


class ServiceTime(ElementwiseProblem):
    """One decision, the mean service time s, and one objective, the cost."""

    def __init__(self, evaluator):
        super().__init__(n_var=1, n_obj=1, xl=np.array([0.2]), xu=np.array([0.9]))
        self.evaluator = evaluator

    def _evaluate(self, x, out, *args, **kwargs):
        # Kept in the bounds, which the evaluation enforces: a step can land a rounding error
        # outside them.
        s = float(np.clip(x[0], 0.2, 0.9))
        answer = self.evaluator.evaluate({"server.mean": s}, first=0, count=10)
        out["F"] = [answer["results"][0]["mean"] - 4 * s]


def main():
    cli = sys.argv[1] if len(sys.argv) > 1 else "stochcraft-cli"
    model = os.path.join(HERE, "mm1-cost.bpmn")
    evaluator = Evaluator(
        cli,
        model,
        results=["server.mean_work_in_progress"],
        ranges=["server.mean=0.2..0.9"],
    )
    result = minimize(
        ServiceTime(evaluator),
        PatternSearch(),
        termination=("n_evals", 60),
        seed=1,
    )
    evaluator.close()
    s, value = float(result.X[0]), float(result.F[0])
    print(f"pymoo: s = {s:.4f}, cost {value:.4f} ({result.algorithm.evaluator.n_eval} evaluations)")
    if abs(s - 0.5) > 0.05:
        print("expected s within 0.05 of 0.5", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
