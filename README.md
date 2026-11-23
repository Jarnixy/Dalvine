# Dalvine

A from-scratch Android APK loader in Rust that runs apps on a PC with no emulator or Android OS. It parses APKs (manifest, DEX), runs Dalvik bytecode in its own VM, and maps Android API calls to native host code. It aims to work like Wine, but for Android apps.

```
cargo run -- app.apk [info|manifest|classes]
cargo test
```
