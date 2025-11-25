# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

This is a procedural macro crate that provides the `#[controller]` attribute macro for firmware development in `no_std` environments. The macro generates boilerplate for decoupling component interactions through:

* A controller struct that manages peripheral state.
* Client API for sending commands to the controller.
* Signal mechanism for broadcasting events.
* Pub/sub system for state change notifications.

The macro works by processing both struct definitions (to add publishers) and impl blocks (to generate the controller's `run` method, client API, and signal infrastructure).

## Build & Test Commands

```bash
# Run all tests (includes doc tests from README)
cargo test --locked

# Run a specific test
cargo test --locked <test_name>

# Check formatting (requires nightly)
cargo +nightly fmt -- --check

# Auto-format code (requires nightly)
cargo +nightly fmt

# Run clippy (CI fails on warnings)
cargo clippy --locked -- -D warnings

# Build the crate
cargo build --locked

# Build documentation
cargo doc --locked
```

## Architecture

### Macro Entry Point (`src/lib.rs`)
The `controller` attribute macro dispatches to either `item_struct` or `item_impl` based on input type.

### Struct Processing (`src/controller/item_struct.rs`)
Processes `#[controller]` on struct definitions. For fields marked with `#[controller(publish)]`:
* Adds publisher fields to the struct.
* Generates setters (`set_<field>`) that broadcast changes.
* Creates `<StructName><FieldName>` stream type and `<StructName><FieldName>Changed` event struct.

The generated `new()` method initializes both user fields and generated publisher fields.

### Impl Processing (`src/controller/item_impl.rs`)
Processes `#[controller]` on impl blocks. Distinguishes between:

**Proxied methods** (normal methods):
* Creates request/response channels for each method.
* Generates matching client-side methods that send requests and await responses.
* Adds arms to the controller's `run()` method select loop to handle requests.

**Signal methods** (marked with `#[controller(signal)]`):
* Methods have no body in the user's impl block.
* Generates method implementation that broadcasts to subscribers.
* Creates `<StructName><MethodName>` stream type and `<StructName><MethodName>Args` struct.
* Signal methods are NOT exposed in the client API (controller emits them directly).

The generated `run()` method contains a `select_biased!` loop that receives method calls from clients and dispatches them to the user's implementations.

### Constants (`src/controller/mod.rs`)
Channel capacities and subscriber limits are defined here:
* `ALL_CHANNEL_CAPACITY`: 8
* `SIGNAL_CHANNEL_CAPACITY`: 8
* `BROADCAST_MAX_PUBLISHERS`: 1
* `BROADCAST_MAX_SUBSCRIBERS`: 16

### Utilities (`src/util.rs`)
Case conversion functions (`pascal_to_snake_case`, `snake_to_pascal_case`) used for generating type and method names.

## Dependencies

User code must have these dependencies (per README):
* `futures` with `async-await` feature.
* `embassy-sync` for channels and synchronization.

Dev dependencies include `embassy-executor` and `embassy-time` for testing.

## Key Limitations

* Singleton operation: multiple controller instances interfere with each other.
* Methods must be async and cannot use reference parameters/return types.
* Maximum 16 subscribers per state/signal stream.
* Published fields must implement `Clone` and `Debug`.
* Streams must be continuously polled or notifications are missed.
