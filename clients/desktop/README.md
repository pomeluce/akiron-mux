# AkironMux GPUI Desktop

The native Desktop Client is a standalone Rust workspace so its GPUI toolchain
and lockfile do not change the daemon's Rust 1.80 compatibility boundary.

```sh
nix develop .#gpui
cargo check --locked --workspace --all-targets
cargo test --locked --workspace
cargo run --locked -p akmux-desktop
```

Release packages use `cargo-packager` 0.11.8 and `packager.json`:

```sh
cargo install cargo-packager --version 0.11.8 --locked
cargo packager --release --config packager.json --formats deb,appimage
```

The React/Vite application in `web/session-ui` remains the Embedded WebUI. It
is not linked into or bundled with this Desktop Client.
