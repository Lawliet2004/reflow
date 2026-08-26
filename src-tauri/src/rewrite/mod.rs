pub mod client;
pub mod prompt;
pub mod runtime_install;
pub mod safety;
pub mod server;

pub use client::{polish_or_fallback, FlowClient, PolishOutcome, RewriteRequest};
pub use runtime_install::{
    auto_install_if_missing, install_runtime, pick_runtime_spec, LlamaRuntimeSpec, RuntimePhase,
};
pub use safety::accept_rewrite;
pub use server::{flow_gguf_path, flow_http_timeout, llama_server_bin, FlowRuntime, LlamaMode};
