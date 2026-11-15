# Installing StochCraft

StochCraft runs in a web browser or as a desktop app. Both have the same window and give the same
results for the same model and seed.

## In the browser (nothing to install)

Open [helenkwok.github.io/StochCraft](https://helenkwok.github.io/StochCraft/) in a current
Chrome, Edge, Firefox or Safari.

- **Offline.** Once the page has loaded, it keeps working without a connection.
- **As an app.** Your browser can install it so it opens in its own window: in Chrome or Edge,
  click the install icon at the right of the address bar.
- **Your files stay with you.** Models open from and save to your own computer. Nothing is sent
  to a server.
- **Speed.** Runs use every processor core but one, up to 16.

## On the desktop

There is no downloadable release yet. Until there is, build the desktop app from source. It takes
a few minutes the first time.

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

### When releases come

Releases will be on the [releases page](https://github.com/helenkwok/StochCraft/releases), with:

- **Windows (x64):** a zip with `stochcraft.exe` and `stochcraft-cli.exe`. There is no
  installer: unzip it anywhere and run `stochcraft.exe`. The programs are not code-signed, so
  Windows may warn you the first time: click **More info**, then **Run anyway**.
- **macOS (Apple silicon and Intel):** a disk image; drag StochCraft to Applications.
- **Linux (x86-64 and ARM64):** a `.tar.gz` with both programs.
