# Changelog

## 0.1.0 - 2026-08-10

- Changed: backend fetch rewired onto `reqwest::blocking` (#37)
- Fixed: probes now inherit `accept_invalid_certs`/`accept_invalid_hostnames` from their parent backend (#30)
- Changed: dropped Varnish 8 CI/support, minimum supported Varnish version is now 9.0+
- Added: happy-path HTTPS backend test
- Fixed: several panics on integer-conversion overflow (`follow`, response `Content-Length`) now return a `VclError` instead (#38)

## 0.0.17 - 2026-06-17

- Changed: updated `varnish-rs` to 0.7.0 (#34)
- Changed: increased `VARNISHTEST_DURATION` to give Varnish 8 more time to shut down (#32)
- Changed: refreshed README and CI (#29)

## 0.0.16 - 2025-09-18

- Changed: upgraded dependencies (#27)
- Chore: unified CI config across vmods (#25)

## 0.0.15 - 2025-06-02

- Added: test covering an unusual `Host` header (#22)
- Changed: ported Docker setup from `varnish-rs` (#21)
- Fixed: buffer-consumption handling
- Chore: sorted `justfile` (#23), various fmt/clippy cleanups (#20)

## 0.0.14 - 2025-03-31

- Changed: upgraded to `varnish-rs` 0.4.0
- Added: container image build (#14)
- Chore: API tweaks and formatting

## 0.0.13 - 2024-11-12

- Fixed: avoid panicking on an invalid buffer
- Changed: switched to `varnish-rs`'s macros and new defaults
- Changed: updated Docker image for Varnish 7.5 (#11)
- Chore: more verbose CI

## 0.0.12 - 2024-03-24

- Fixed: use `c_char` correctly
- Added: `authors` metadata
- Chore: upgraded `bindgen` to build on newer Fedora

## 0.0.11 - 2024-03-19

- Added: Dockerfile support for Varnish 7.4 (#8), then updated for Varnish 7.5
- Chore: switched to the sparse Cargo index

## 0.0.10 - 2023-09-23

- Added: `client.copy_headers_to_req()`
- Released for Varnish 7.4

## 0.0.9 - 2023-07-09

- Changed: simplified the `header()` function
- Fixed: Docker packaging (wrong file copied)

## 0.0.8 - 2023-03-19

- Maintenance release

## 0.0.7 - 2023-03-19

- Added: support for 0-length bodies
- Added: `sep` argument to `.header()`
- Changed: upgraded to `varnish-rs` 0.0.14
- Changed: simplified error handling
- Chore: updated Dockerfile to a more recent Rust

## 0.0.6 - 2022-12-19

- Added: streaming support
- Fixed: Dockerfile/CI issues

## 0.0.5 - 2022-11-29

- Changed: renamed `Body` to `ReqBody` (request-only)
- Changed: cleaned up error handling, `init()` can no longer fail
- Added: probe tests and docs

## 0.0.4 - 2022-06-22

- Added: initial probe support
- Fixed: smarter URL building
- Chore: CI updated to Varnish 7.1, examples cleanup, Discord link added

## 0.0.3 - 2022-01-30

- Added: initial backend implementation, unified `client` object
- Chore: proper dependency setup, build script, license fix

## 0.0.2 - 2021-12-19

- Early bring-up

## 0.0.1 - 2021-12-05

- Initial commit
