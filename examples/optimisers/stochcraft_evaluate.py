"""Drives `stochcraft-cli evaluate` from Python: one JSON line out per evaluation, one back.

Part of StochCraft (MIT OR Apache-2.0). Needs only the standard library.
"""

import json
import subprocess


class Evaluator:
    """A running `stochcraft-cli evaluate` with the model open, answering one line at a time."""

    def __init__(self, cli, model, results, ranges=(), seed=None):
        args = [cli, "evaluate", model]
        for r in results:
            args += ["--result", r]
        for r in ranges:
            args += ["--range", r]
        if seed is not None:
            args += ["--seed", str(seed)]
        self.process = subprocess.Popen(
            args,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            text=True,
            bufsize=1,
        )

    def evaluate(self, decisions, first=0, count=None):
        """Each result's values in replications `first` to `first + count - 1`.

        `decisions` maps `id.field` to a value, such as {"server.mean": 0.5}. Returns the
        answer of `experiment.evaluate`; raises ValueError with the engine's message if refused.
        """
        line = {"decisions": [], "first": first}
        for name, value in decisions.items():
            element, field = name.rsplit(".", 1)
            line["decisions"].append({"id": element, "field": field, "value": value})
        if count is not None:
            line["count"] = count
        self.process.stdin.write(json.dumps(line) + "\n")
        self.process.stdin.flush()
        answer = json.loads(self.process.stdout.readline())
        if "error" in answer:
            raise ValueError(answer["error"]["message"])
        return answer

    def close(self):
        self.process.stdin.close()
        self.process.wait()
