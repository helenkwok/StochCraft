/*
 * Writes StochCraft's random numbers to standard output as raw 64-bit words, for an external
 * test battery (PractRand's RNG_test stdin64, run as a separate program).
 *
 * A transcription of crates/random/src/lib.rs: SplitMix64 seeding, xoshiro256** and the keyed
 * streams (64-bit FNV-1a of the key, mixed through SplitMix64). `check` compares it with the
 * values pinned in that file's tests (SplitMix64 from 0, xoshiro256** from seed 42, FNV-1a of ""
 * and "a") and exits non-zero if any differs, so the battery tests the same generator.
 *
 * Licence of this file: MIT OR Apache-2.0, like the rest of the repository.
 *
 * Build: cc -O2 -o stream stream.c
 * Usage: stream check
 *        stream single SEED          one stream, Rng::new(SEED)
 *        stream keyed BASE N         N streams Rng::keyed(BASE, "element/<i>"), i = 0..N-1,
 *                                    one word from each in turn
 * Recorded run (docs/developers.md, "Random numbers"):
 *        stream single 1 | RNG_test stdin64 -tlmax 64GB -multithreaded
 *        stream keyed 12345 1000 | RNG_test stdin64 -tlmax 64GB -multithreaded
 */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static uint64_t splitmix64_next(uint64_t *state) {
  uint64_t z = (*state += 0x9e3779b97f4a7c15ULL);
  z = (z ^ (z >> 30)) * 0xbf58476d1ce4e5b9ULL;
  z = (z ^ (z >> 27)) * 0x94d049bb133111ebULL;
  return z ^ (z >> 31);
}

typedef struct {
  uint64_t s[4];
} rng;

static uint64_t rotl(uint64_t x, int k) { return (x << k) | (x >> (64 - k)); }

static rng rng_new(uint64_t seed) {
  rng r;
  uint64_t sm = seed;
  for (int i = 0; i < 4; i++) r.s[i] = splitmix64_next(&sm);
  return r;
}

static uint64_t rng_next(rng *r) {
  uint64_t *s = r->s;
  uint64_t result = rotl(s[1] * 5, 7) * 9;
  uint64_t t = s[1] << 17;
  s[2] ^= s[0];
  s[3] ^= s[1];
  s[1] ^= s[2];
  s[0] ^= s[3];
  s[2] ^= t;
  s[3] = rotl(s[3], 45);
  return result;
}

static uint64_t fnv1a64(const char *key) {
  uint64_t h = 0xcbf29ce484222325ULL;
  for (; *key; key++) {
    h ^= (unsigned char)*key;
    h *= 0x100000001b3ULL;
  }
  return h;
}

static rng rng_keyed(uint64_t base, const char *key) {
  uint64_t hashed = fnv1a64(key);
  uint64_t mixed = splitmix64_next(&hashed);
  uint64_t seed = base ^ mixed;
  return rng_new(splitmix64_next(&seed));
}

static int check(void) {
  int ok = 1;
  uint64_t sm = 0;
  ok &= splitmix64_next(&sm) == 0xe220a8397b1dcdafULL;
  ok &= splitmix64_next(&sm) == 0x6e789e6aa1b965f4ULL;
  ok &= splitmix64_next(&sm) == 0x06c45d188009454fULL;
  rng r = rng_new(42);
  ok &= rng_next(&r) == 1546998764402558742ULL;
  ok &= rng_next(&r) == 6990951692964543102ULL;
  ok &= rng_next(&r) == 12544586762248559009ULL;
  ok &= fnv1a64("") == 0xcbf29ce484222325ULL;
  ok &= fnv1a64("a") == 0xaf63dc4c8601ec8cULL;
  fputs(ok ? "check ok\n" : "check FAILED\n", stderr);
  return ok ? 0 : 1;
}

int main(int argc, char **argv) {
  static uint64_t buf[4096];
  if (argc == 2 && !strcmp(argv[1], "check")) return check();
  if (argc == 3 && !strcmp(argv[1], "single")) {
    rng r = rng_new(strtoull(argv[2], 0, 10));
    for (;;) {
      for (int i = 0; i < 4096; i++) buf[i] = rng_next(&r);
      if (fwrite(buf, 8, 4096, stdout) != 4096) return 0;
    }
  }
  if (argc == 4 && !strcmp(argv[1], "keyed")) {
    uint64_t base = strtoull(argv[2], 0, 10);
    int n = atoi(argv[3]);
    if (n < 1) return 2;
    rng *streams = malloc(sizeof(rng) * (size_t)n);
    if (!streams) return 2;
    char key[64];
    for (int i = 0; i < n; i++) {
      snprintf(key, sizeof key, "element/%d", i);
      streams[i] = rng_keyed(base, key);
    }
    int j = 0;
    for (;;) {
      for (int i = 0; i < 4096; i++) {
        buf[i] = rng_next(&streams[j]);
        j = (j + 1) % n;
      }
      if (fwrite(buf, 8, 4096, stdout) != 4096) return 0;
    }
  }
  fputs("usage: stream check | single SEED | keyed BASE N\n", stderr);
  return 2;
}
