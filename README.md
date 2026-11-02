# StochCraft

Discrete-event and agent-based simulation for factories, logistics and queueing systems: an
open-source, clean-room simulation application in pure Rust, for native desktops and the browser.

**Status:** just started. `stochcraft-des` runs SimPy-style async processes with resources on a deterministic event queue, checked against M/M/1 queueing theory; `stochcraft-random` provides seeded xoshiro256** streams.

## Building

```bash
cargo test --workspace
```

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for the clean-room and licence rules.

## Licence

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or
  <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT licence ([LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/licenses/MIT>)

at your option. Third-party code and assets keep their documented licences; see
[NOTICE](NOTICE) and [ATTRIBUTION.md](ATTRIBUTION.md).

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in
StochCraft by you, as defined in the Apache-2.0 licence, shall be dual licensed as above, without
any additional terms or conditions.

StochCraft is independent and not affiliated with any simulation or CAD vendor; product names
are trademarks of their owners (see [NOTICE](NOTICE)).
