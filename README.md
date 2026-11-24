# Dalvine

A from-scratch Android APK loader in Rust that runs apps on a PC with no emulator or Android OS. It parses APKs (manifest, DEX), runs Dalvik bytecode in its own VM, and maps Android API calls to native host code. It aims to work like Wine, but for Android apps.

```
cargo run -- app.apk run                      # open the app in a window (click = tap, Esc = back)
cargo run -- app.apk shot out.ppm [x,y ...]   # headless screenshot, optionally after taps
cargo run -- app.apk [info|manifest|classes|tree]
cargo test
```
