# Permission fixture moved

The five permission scenarios for both browser classes now live in the
[headless integration suite](../integration/README.md), with test logic in the
separate Rust `gdcef_itest` addon. The old GDScript runner has been removed.

After `cargo xtask bundle`, run:

```sh
cargo xtask integration --godot /absolute/path/to/godot
```

On Linux, run under `xvfb-run -a` for CEF. To select a previous permission case,
prefix its name with `permission_`, for example
`--case CefTexture2D:permission_navigation`.
