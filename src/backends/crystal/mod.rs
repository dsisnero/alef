//! Crystal binding generator backend for alef.
//!
//! Crystal binds to the generated C FFI layer via `lib`/`fun` declarations
//! (a C-ABI consumer, like the Go and Zig backends). Design goals:
//! - **Ruby-style naming** — snake_case methods, PascalCase types (see
//!   `crate::codegen::naming` `Language::Crystal` arms).
//! - **Rust-like generics** — typed wrappers use Crystal generics (`Array(T)`,
//!   `Hash(K, V)`, `class Foo(T)`).
//! - **Go/Crystal concurrency** — async and streaming adapters map onto
//!   Crystal fibers (`spawn`) and `Channel(T)`.

mod gen_bindings;
pub(crate) mod template_env;
pub mod trait_bridge;
pub mod type_map;

pub use gen_bindings::CrystalBackend;
