# proxy-wasm-rust-rate-limiting

A prototype implementation of a rate-limiting filter written in Rust,
using the proxy-wasm API for running on WebAssembly-enabled gateways.

## What's implemented

* "local" policy only, using the SHM-based key-value store

## What's missing

* Getting proper route and service ids for producing identifiers.
* Other policies, which would require additional features from the
  underlying system, such as calling out to a Redis instance.

## Build requirements

* Rust
  * [rustup.rs](https://rustup.rs) is the easiest way to install Rust.
    * Then add the Wasm32-WASI target to your toolchain: `rustup target add wasm32-wasi`.

## Building and running

Once the environment is set up with `cargo` in your PATH,
you can build it with:

```
cargo build --release
```

This will produce a .wasm file in `target/wasm32-wasi/release/`.

Once you have a Wasm-enabled Kong container with a recent ngx_wasm_module
integrated (the container from the Summit 2022 Tech Preview is too old),
you can run the script in `test/demo.sh` to give the filter a spin.

You also need the `kong_wasm_rate_limiting_counters` shared memory
key-value store enabled in your Kong configuration. One way to
achieve this is via the following environment variable:

```sh
export KONG_NGINX_WASM_SHM_KV_KONG_WASM_RATE_LIMITING_COUNTERS=12m
```
