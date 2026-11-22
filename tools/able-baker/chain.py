#!/usr/bin/env python3
# The exact long-run answer for tests/models/able-baker.bpmn, checked by
# crates/engine/tests/examples.rs: Able and Baker (Banks, Carson, Nelson and Nicol,
# Discrete-Event System Simulation, Example 2.2) as a discrete-time Markov chain on whole
# minutes.
#
# State after the events at an instant: (minutes to the next arrival n, Able's remaining service
# ra, Baker's remaining service rb, queue length q). At an instant: finished services free their
# servers, an arrival joins the queue, then the queue (first come, first served) takes the free
# servers, Able before Baker. Service starts at max(arrival, first free). The chain is truncated
# at queue length Q; the values are the same to six digits at Q = 30 and Q = 50.
#
# Dependency: numpy (BSD-3-Clause), used here for the check only; it is not part of the
# StochCraft build. Licence of this file: MIT OR Apache-2.0, like the rest of the repository.
#
# Usage: python3 -P chain.py 30
import sys
import numpy as np
IA = {1: .25, 2: .40, 3: .20, 4: .15}
AB = {2: .30, 3: .28, 4: .25, 5: .17}
BA = {3: .35, 4: .25, 5: .20, 6: .20}
Q = int(sys.argv[1])
def assign(q, ra, rb):
    out, res = [(1.0, q, ra, rb)], []
    while out:
        p, q, ra, rb = out.pop()
        if q > 0 and ra == 0:
            out += [(p * ps, q - 1, s, rb) for s, ps in AB.items()]
        elif q > 0 and rb == 0:
            out += [(p * ps, q - 1, ra, s) for s, ps in BA.items()]
        else:
            res.append((p, q, ra, rb))
    return res
states = [(n, ra, rb, q) for n in IA for ra in range(6) for rb in range(7) for q in range(Q + 1)]
index = {s: i for i, s in enumerate(states)}
src, dst, pr, waited = [], [], [], []
for (n, ra, rb, q) in states:
    i = index[(n, ra, rb, q)]
    n1, ra1, rb1 = n - 1, max(ra - 1, 0), max(rb - 1, 0)
    arrive = n1 == 0
    q1 = min(q + arrive, Q)
    for p2, q2, ra2, rb2 in assign(q1, ra1, rb1):
        for n2, pn in (IA.items() if arrive else [(n1, 1.0)]):
            src.append(i); dst.append(index[(n2, ra2, rb2, q2)]); pr.append(p2 * pn)
            waited.append(1.0 if (arrive and q2 > 0) else 0.0)
src, dst, pr, waited = map(np.array, (src, dst, pr, waited))
arrive_mask = np.array([states[i][0] == 1 for i in src], dtype=float)
pi = np.full(len(states), 1.0 / len(states))
for it in range(200000):
    new = np.bincount(dst, weights=pi[src] * pr, minlength=len(states))
    new = 0.5 * pi + 0.5 * new
    if np.abs(new - pi).sum() < 1e-15:
        pi = new
        break
    pi = new
st = np.array(states)
able, baker = pi[st[:, 1] > 0].sum(), pi[st[:, 2] > 0].sum()
lq = (pi * st[:, 3]).sum()
flow = pi[src] * pr
arr = (flow * arrive_mask).sum()
pw = (flow * waited).sum() / arr
lam = 1 / sum(k * v for k, v in IA.items())
print(f"Q={Q} it={it} able={able:.6f} baker={baker:.6f} Lq={lq:.6f} Wq={lq / lam:.6f} "
      f"P(wait)={pw:.6f} Wq|waited={lq / lam / pw:.6f} arrivals/min={arr:.6f} (1/2.25={lam:.6f}) "
      f"P(q=Q)={pi[st[:, 3] == Q].sum():.2e}")
