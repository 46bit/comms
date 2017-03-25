# `comms`

This is an experimental Futures library for coordinating 1-to-N server clients.

## Status

[![Build Status](https://api.travis-ci.org/46bit/comms.svg)](https://travis-ci.org/46bit/comms) [![Coverage Status](https://coveralls.io/repos/github/46bit/comms/badge.svg)](https://coveralls.io/github/46bit/comms)

## Description

At the time of writing `tokio-core`, `tokio-proto` and `tokio-service` are focused on network interactions that closely follow the Request-Response pattern. This library aims to provide useful, flexible communication primitives to enable more complex network applications in asynchronous Rust.

## Development

* Reformat code with `cargo fmt`.
* Lint code with `cargo build --features dev`.
* Run tests with `cargo test`.

## License

`comms` is primarily distributed under the terms of both the MIT license and the Apache License (Version 2.0), with portions covered by various BSD-like licenses.

See LICENSE-APACHE, and LICENSE-MIT for details.
