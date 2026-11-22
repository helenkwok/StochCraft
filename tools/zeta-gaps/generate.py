#!/usr/bin/env python3
# Generates the interarrival-gap distribution used by tests/models/zeta-queue.bpmn.
#
# It computes the first N non-trivial zeros of the Riemann zeta function (heights gamma_n on the
# critical line) with mpmath's zetazero, unfolds them with the smooth counting function
#   N(T) = (T / 2pi) * log(T / (2 pi e)) + 7/8
# so that the normalised gaps s_n = N(gamma_{n+1}) - N(gamma_n) have mean close to 1, bins the
# gaps into a discrete distribution (bin midpoints and shares), and compares the histogram with
# the GUE Wigner surmise p(s) = (32 / pi^2) s^2 exp(-4 s^2 / pi).
#
# Dependency: mpmath (BSD-3-Clause), used here for data generation only; it is not part of the
# StochCraft build. The zeros are computed, not taken from published tables.
# Licence of this file: MIT OR Apache-2.0, like the rest of the repository.
#
# Usage: python3 -I generate.py --n 10000 --out DIR [--bins 40] [--workers 12]
# N + 1 zeros give N gaps. With --zeros FILE the heights are read from an earlier zeros.csv.
# Writes DIR/zeros.csv, DIR/gaps.csv, DIR/bins.tsv, DIR/points.xml (the model's data points,
# scaled to --mean) and DIR/summary.json.
import argparse
import json
import math
import multiprocessing as mp
import os
import sys
import time


def zero_height(n):
    import mpmath

    mpmath.mp.dps = 25
    return str(mpmath.zetazero(n).imag)


def smooth_count(t):
    return t / (2 * math.pi) * math.log(t / (2 * math.pi * math.e)) + 7.0 / 8.0


def wigner_cdf_between(a, b):
    # Integral of p(s) = (32/pi^2) s^2 exp(-4 s^2 / pi) over [a, b], by Simpson's rule.
    steps = 200
    h = (b - a) / steps
    f = lambda s: 32 / math.pi**2 * s * s * math.exp(-4 * s * s / math.pi)
    total = f(a) + f(b)
    for i in range(1, steps):
        total += (4 if i % 2 else 2) * f(a + i * h)
    return total * h / 3


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--n", type=int, default=10000)
    ap.add_argument("--out", required=True)
    ap.add_argument("--bins", type=int, default=40)
    ap.add_argument("--zeros", help="read the heights from this zeros.csv instead of computing them")
    ap.add_argument("--mean", type=float, default=1.25, help="mean gap in the model, in minutes")
    ap.add_argument("--workers", type=int, default=max(1, (os.cpu_count() or 2) - 2))
    args = ap.parse_args()
    os.makedirs(args.out, exist_ok=True)
    import mpmath

    zeros_path = os.path.join(args.out, "zeros.csv")
    start = time.time()
    heights = []
    elapsed = None
    if args.zeros:
        with open(args.zeros) as f:
            next(f)
            heights = [float(line.split(",")[1]) for line in f]
    else:
        with mp.Pool(args.workers) as pool:
            for i, h in enumerate(pool.imap(zero_height, range(1, args.n + 2), chunksize=20), 1):
                heights.append(float(mpmath.mpf(h)))
                if i % 500 == 0:
                    print(f"{i} zeros, {time.time() - start:.0f} s", flush=True)
        elapsed = time.time() - start
        with open(zeros_path, "w", newline="\n") as f:
            f.write("n,height\n")
            for i, h in enumerate(heights, 1):
                f.write(f"{i},{h!r}\n")
    # n zeros give n gaps: the (n+1)th zero closes the last one.
    counts = [smooth_count(h) for h in heights]
    gaps = [counts[i + 1] - counts[i] for i in range(args.n)]
    with open(os.path.join(args.out, "gaps.csv"), "w", newline="\n") as f:
        f.write("n,gap\n")
        for i, g in enumerate(gaps, 1):
            f.write(f"{i},{g!r}\n")
    n = len(gaps)
    mean = sum(gaps) / n
    var = sum((g - mean) ** 2 for g in gaps) / (n - 1)
    below = sum(1 for g in gaps if g < 0.25) / n
    hi = max(gaps)
    width = hi / args.bins
    hist = [0] * args.bins
    for g in gaps:
        hist[min(int(g / width), args.bins - 1)] += 1
    probs = [c / n for c in hist]
    mids = [(i + 0.5) * width for i in range(args.bins)]
    # Exact mean 1 after binning, so the model's mean is the stated rate.
    bmean = sum(p * m for p, m in zip(probs, mids))
    scale = 1.0 / bmean
    wig = [wigner_cdf_between(i * width, (i + 1) * width) for i in range(args.bins)]
    with open(os.path.join(args.out, "bins.tsv"), "w", newline="\n") as f:
        f.write("midpoint\tprobability\twigner_probability\n")
        for m, p, w in zip(mids, probs, wig):
            f.write(f"{m * scale:.6f}\t{p:.6f}\t{w:.6f}\n")
    # The BPSim discrete UserDistribution of the model: gaps scaled to the mean of the model.
    with open(os.path.join(args.out, "points.xml"), "w", newline="\n") as f:
        for m, c in zip(mids, hist):
            f.write(
                f'<bpsim:UserDistributionDataPoint probability="{c / n!r}">'
                f'<bpsim:FloatingParameter value="{m * scale * args.mean:.6f}"/>'
                f"</bpsim:UserDistributionDataPoint>\n"
            )
    bvar = sum(p * (m * scale - 1) ** 2 for p, m in zip(probs, mids))
    summary = {
        "mpmath": mpmath.__version__,
        "zeros": len(heights),
        "last_height": heights[args.n - 1],
        "seconds": elapsed,
        "gap_mean": mean,
        "gap_variance": var,
        "gap_max": hi,
        "share_below_0_25": below,
        "poisson_share_below_0_25": 1 - math.exp(-0.25),
        "wigner_share_below_0_25": wigner_cdf_between(0, 0.25),
        "wigner_variance": 3 * math.pi / 8 - 1,
        "bins": args.bins,
        "binned_mean_before_scaling": bmean,
        "binned_variance_after_scaling": bvar,
        "total_variation_vs_wigner": 0.5 * sum(abs(p - w) for p, w in zip(probs, wig)),
        "wigner_mass_beyond_max": 1 - sum(wig),
    }
    with open(os.path.join(args.out, "summary.json"), "w", newline="\n") as f:
        json.dump(summary, f, indent=2)
        f.write("\n")
    print(json.dumps(summary, indent=2), flush=True)


if __name__ == "__main__":
    sys.exit(main())
