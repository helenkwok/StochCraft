# Installing StochCraft

StochCraft runs in a web browser or as a desktop app. Both have the same window and give the same
results for the same model and seed.

## In the browser (nothing to install)

Open [helenkwok.github.io/StochCraft](https://helenkwok.github.io/StochCraft/) in a current
desktop browser.

- **Offline.** Once the page has loaded, it keeps working without a connection.
- **As an app.** Your browser can install it so it opens in its own window: in Chrome or Edge,
  click the install icon at the right of the address bar.
- **Your files stay with you.** Models open from and save to your own computer. Nothing is sent
  to a server.
- **Speed.** Runs use every processor core but one, up to 16.

## On the desktop

Download a release (below), or build the desktop app from source. Building takes a few minutes
the first time.

1. Install Rust 1.95 or newer from [rustup.rs](https://rustup.rs).
2. Get the source:

   ```bash
   git clone https://github.com/helenkwok/StochCraft.git
   cd StochCraft
   ```

3. Build and start the app:

   ```bash
   cargo run --release -p stochcraft
   ```

   The program is then at `target/release/stochcraft` (`stochcraft.exe` on Windows); you can
   copy it anywhere and start it from there.

The command line, for scripted and batch runs, installs with
`cargo install --path apps/stochcraft-cli`; see the
[developers guide](developers.md#the-command-line).

### Download a release

Releases are on the [releases page](https://github.com/helenkwok/StochCraft/releases), with:

- **Windows (x64):** `stochcraft-<version>-windows-x64-portable.zip`, with `stochcraft.exe`
  and `stochcraft-cli.exe`. There is no installer: unzip it anywhere and run `stochcraft.exe`.
  The programs are not code-signed, so Windows may warn you the first time: click
  **More info**, then **Run anyway**.
- **macOS (Apple silicon and Intel):** `stochcraft-<version>-macos-universal.dmg`, a disk image;
  drag StochCraft to Applications. The command line comes in its own archive,
  `stochcraft-cli-<version>-macos-universal.tar.gz`.
- **Linux (x86-64 and ARM64):** `stochcraft-<version>-linux-x86_64.tar.gz` or
  `-linux-aarch64.tar.gz`, with both programs.
- **`SHA256SUMS.txt`:** the SHA-256 checksum of every file, in `sha256sum` format.

Each app download also holds a `README.md`, the licence files (`LICENSE-APACHE`, `LICENSE-MIT`
and `NOTICE`), `THIRD-PARTY-LICENSES.html`, which lists every open-source Rust library in the
programs with its version and licence text, and `OFL-IBM-Plex.txt`, the licence of the interface
font. On macOS the files are inside the app (right-click StochCraft, then **Show Package
Contents**, then `Contents/Resources`); the macOS command-line archive holds the licence files,
`THIRD-PARTY-LICENSES.html` and the `README.md`. The browser app serves the same list as
`THIRD-PARTY-LICENSES.html` beside the app, and the About box links to it.
