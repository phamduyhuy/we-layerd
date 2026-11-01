# we-layerd — local build notes

## Building on this machine

System does not have wayland/pulse dev packages installed globally. The dev
deps are extracted under `~/.local/opt/deps` (dpkg -x style). The upstream
renderer source is missing, so point the build at the prebuilt renderer from
the extracted AppImage runtime:

```sh
export PKG_CONFIG_PATH="$HOME/.local/opt/deps/usr/lib/x86_64-linux-gnu/pkgconfig:$HOME/.local/opt/deps/usr/share/pkgconfig"
export LIBRARY_PATH="$HOME/.local/opt/deps/usr/lib/x86_64-linux-gnu"
export WE_LAYERD_PREBUILT_RENDERER_ROOT="$HOME/.local/opt/we-layerd/usr"

cargo build --release --bin we-layerd
cargo test --release -p we-layerd   # one test fails inside a GNOME session (env-dependent)
```

## Deployment

- Daemon binary: `target/release/we-layerd` → `~/.local/opt/we-layerd/usr/bin/we-layerd.real`
  (the `we-layerd` wrapper next to it sets `LD_LIBRARY_PATH` + GPU env vars).
- GNOME extension: `contrib/gnome-shell-extension/we-layerd@aromatic/*` →
  `~/.local/share/gnome-shell/extensions/we-layerd@aromatic/`
- Runtime service: `systemctl --user {start,stop,status} we-layerd.service`,
  logs via `journalctl --user -u we-layerd.service`.

## GNOME Shell extension reload

On Wayland, `gnome-extensions disable/enable` does NOT reload extension JS —
the GJS module cache is per Shell process. Any change to extension JS/XML
requires a logout/login (or reboot) to take effect.

## GPU / presentation notes

- This laptop: AMD Cezanne iGPU (drives eDP-1 + mutter) + GTX 1650 (offload).
- `__NV_PRIME_RENDER_OFFLOAD=1` forces the SHM readback path (~9fps, heavy
  CPU). Rendering on the iGPU gives dma-buf zero-copy presentation (~60fps).
- Event loop frame callbacks are `gated` for layer-shell surfaces (always
  visible → wl_surface.frame is a vsync clock) and ungated for the GNOME
  XDG toplevel (mutter stops callbacks when the window is fully occluded;
  the loop self-paces via the render interval instead).
- Window rules (`[rules] focused/maximized/fullscreen`) use the GNOME
  extension's `GetWindowStates` D-Bus method because mutter does not
  implement `zwlr_foreign_toplevel_manager_v1`.
