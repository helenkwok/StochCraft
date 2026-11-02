"""M/M/1 in SimPy, the same two workloads as bench/src/main.rs.

    python3 bench/simpy/mm1.py [store|resource|all] [N]

store:    an arrival process puts timestamps into a simpy.Store, one server process takes them.
resource: the arrival process starts one process per customer, which requests a
          simpy.Resource(capacity=1).
"""

import random
import sys
import time

import simpy

LAMBDA = 0.9
MU = 1.0


def store_model(n):
    env = simpy.Environment()
    store = simpy.Store(env)
    acc = {"n": 0, "sum": 0.0}

    def arrivals():
        for _ in range(n):
            yield env.timeout(random.expovariate(LAMBDA))
            yield store.put(env.now)

    def server():
        while True:
            arrived = yield store.get()
            yield env.timeout(random.expovariate(MU))
            acc["sum"] += env.now - arrived
            acc["n"] += 1

    env.process(arrivals())
    env.process(server())
    env.run()
    return acc


def resource_model(n):
    env = simpy.Environment()
    server = simpy.Resource(env, capacity=1)
    acc = {"n": 0, "sum": 0.0}

    def customer():
        arrived = env.now
        with server.request() as req:
            yield req
            yield env.timeout(random.expovariate(MU))
        acc["sum"] += env.now - arrived
        acc["n"] += 1

    def arrivals():
        for _ in range(n):
            yield env.timeout(random.expovariate(LAMBDA))
            env.process(customer())

    env.process(arrivals())
    env.run()
    return acc


def report(workload, model, n):
    random.seed(1)
    started = time.perf_counter()
    acc = model(n)
    secs = time.perf_counter() - started
    w = acc["sum"] / max(acc["n"], 1)
    print(f"{'simpy':<12} {workload:<9} {acc['n']:>9} customers  W = {w:>7.4f}  {secs:>7.3f} s  "
          f"{acc['n'] / secs / 1e6:>6.2f} M customers/s")


def main():
    which = sys.argv[1] if len(sys.argv) > 1 else "all"
    n = int(sys.argv[2]) if len(sys.argv) > 2 else 1_000_000
    print(f"SimPy {simpy.__version__}, Python {sys.version.split()[0]}")
    if which in ("store", "all"):
        report("store", store_model, n)
    if which in ("resource", "all"):
        report("resource", resource_model, n)


if __name__ == "__main__":
    main()
