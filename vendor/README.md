# Vendored crates

`gpui/` is GPUI 0.2.2 from crates.io (Apache-2.0, see `gpui/LICENSE-APACHE`) with its
examples removed and one change, marked "T1K patch" in
`gpui/src/platform/mac/metal_renderer.rs`: the Metal layer's `nextDrawable` may time out.
Without that, a stalled WindowServer or GPU freezes the main thread for good.
Upstream fix: https://github.com/zed-industries/zed/pull/67486
