use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct CrystalConfig {
    /// Crystal module name (PascalCase). Defaults to the crate name in PascalCase.
    pub module_name: Option<String>,
    /// Cargo features to enable in the binding crate.
    #[serde(default)]
    pub features: Option<Vec<String>>,
    /// Functions to exclude from Crystal binding generation.
    #[serde(default)]
    pub exclude_functions: Vec<String>,
    /// Types to exclude from Crystal binding generation.
    #[serde(default)]
    pub exclude_types: Vec<String>,
    /// Fields to exclude from Crystal binding generation.
    /// Format: `"TypeName.field_name"`.
    #[serde(default)]
    pub exclude_fields: Vec<String>,
    /// Per-field name remapping. Key is `TypeName.field_name`, value is the
    /// desired Crystal field name (snake_case).
    #[serde(default)]
    pub rename_fields: HashMap<String, String>,
    /// Opaque handle types whose FFI handle is BORROWED (owned by the host /
    /// shared static) — these classes get NO freeing `finalize`, preventing
    /// double-free when the same handle is wrapped multiple times.
    #[serde(default)]
    pub borrowed_handles: Vec<String>,
}
