# Vendored crates

`gpui/` is GPUI 0.2.2 from crates.io (Apache-2.0, see `gpui/LICENSE-APACHE`) with its
examples removed and two changes, each marked "T1K patch":

- `gpui/src/platform/mac/metal_renderer.rs`: the Metal layer's `nextDrawable` may time out.
  Without that, a stalled WindowServer or GPU freezes the main thread for good.
  Upstream fix: https://github.com/zed-industries/zed/pull/67486
- `gpui/src/window.rs`: `Window::topmost_hitbox_at`, so a spinner can tell during paint
  whether a menu or dialog covers it (spinners turn as Core Animation layers above the
  Metal layer, see `crates/t1k-app/src/spin.rs`).
