use std::borrow::Cow;

use crate::codegen::type_mapper::TypeMapper;
use crate::core::ir::{PrimitiveType, TypeRef};
use heck::ToPascalCase;

/// Convert an IR type name to a Crystal type identifier (PascalCase, Ruby-style).
pub fn crystal_type_name(name: &str) -> String {
    // A path-qualified Rust type (e.g. `bytes::Bytes`) maps to just the final
    // segment in Crystal; full-path pascal-casing would produce `BytesBytes`.
    let last = name.rsplit("::").next().unwrap_or(name);
    last.to_pascal_case()
}

/// TypeMapper for high-level Crystal wrapper types.
///
/// Maps Rust IR types to idiomatic Crystal types:
/// - Integers use Crystal's explicit-width types (`UInt8`, `Int32`, ...)
/// - `usize`/`isize` map to `UInt64`/`Int64`
/// - `Optional<T>` becomes `T?` (nilable union)
/// - `Vec<T>` becomes `Array(T)` (Rust-like generic instantiation)
/// - `Map<K,V>` becomes `Hash(K, V)`
/// - `Bytes` becomes `Bytes` (`Slice(UInt8)`)
/// - JSON becomes `JSON::Any`
/// - Unit becomes `Nil`
/// - Duration becomes `Int64` (milliseconds)
pub struct CrystalMapper;

impl TypeMapper for CrystalMapper {
    fn primitive(&self, prim: &PrimitiveType) -> Cow<'static, str> {
        Cow::Borrowed(match prim {
            PrimitiveType::Bool => "Bool",
            PrimitiveType::U8 => "UInt8",
            PrimitiveType::U16 => "UInt16",
            PrimitiveType::U32 => "UInt32",
            PrimitiveType::U64 => "UInt64",
            PrimitiveType::I8 => "Int8",
            PrimitiveType::I16 => "Int16",
            PrimitiveType::I32 => "Int32",
            PrimitiveType::I64 => "Int64",
            PrimitiveType::F32 => "Float32",
            PrimitiveType::F64 => "Float64",
            PrimitiveType::Usize => "UInt64",
            PrimitiveType::Isize => "Int64",
        })
    }

    fn string(&self) -> Cow<'static, str> {
        Cow::Borrowed("String")
    }

    fn bytes(&self) -> Cow<'static, str> {
        Cow::Borrowed("Bytes")
    }

    fn path(&self) -> Cow<'static, str> {
        Cow::Borrowed("String")
    }

    fn json(&self) -> Cow<'static, str> {
        Cow::Borrowed("JSON::Any")
    }

    fn unit(&self) -> Cow<'static, str> {
        Cow::Borrowed("Nil")
    }

    fn duration(&self) -> Cow<'static, str> {
        Cow::Borrowed("Int64")
    }

    fn optional(&self, inner: &str) -> String {
        format!("{inner}?")
    }

    fn vec(&self, inner: &str) -> String {
        format!("Array({inner})")
    }

    fn map(&self, key: &str, value: &str) -> String {
        format!("Hash({key}, {value})")
    }

    fn named<'a>(&self, name: &'a str) -> Cow<'a, str> {
        Cow::Owned(crystal_type_name(name))
    }

    fn error_wrapper(&self) -> &str {
        // Crystal surfaces failures via exceptions, not a Result wrapper type.
        ""
    }
}

/// Maps a TypeRef to its high-level Crystal type representation.
pub fn crystal_type(ty: &TypeRef) -> Cow<'static, str> {
    Cow::Owned(CrystalMapper.map_type(ty))
}

/// Maps a TypeRef to the Crystal `lib` (C-ABI) binding type.
///
/// The generated C FFI layer marshals every non-scalar value as a JSON string
/// (`*mut c_char`), scalars pass by value, and `Unit` returns nothing. This
/// mirrors the FFI backend's C signatures so the `fun` declarations line up
/// with the exported symbols.
pub fn crystal_c_type(ty: &TypeRef) -> Cow<'static, str> {
    match ty {
        TypeRef::Primitive(p) => Cow::Borrowed(match p {
            PrimitiveType::Bool => "Bool",
            PrimitiveType::U8 => "UInt8",
            PrimitiveType::U16 => "UInt16",
            PrimitiveType::U32 => "UInt32",
            PrimitiveType::U64 => "UInt64",
            PrimitiveType::I8 => "Int8",
            PrimitiveType::I16 => "Int16",
            PrimitiveType::I32 => "Int32",
            PrimitiveType::I64 => "Int64",
            PrimitiveType::F32 => "Float32",
            PrimitiveType::F64 => "Float64",
            PrimitiveType::Usize => "LibC::SizeT",
            PrimitiveType::Isize => "LibC::SSizeT",
        }),
        TypeRef::Unit => Cow::Borrowed("Void"),
        TypeRef::Duration => Cow::Borrowed("UInt64"),
        // Every non-scalar type crosses the C ABI as a NUL-terminated JSON string.
        TypeRef::String
        | TypeRef::Char
        | TypeRef::Path
        | TypeRef::Json
        | TypeRef::Bytes
        | TypeRef::Optional(_)
        | TypeRef::Vec(_)
        | TypeRef::Map(_, _)
        | TypeRef::Named(_) => Cow::Borrowed("LibC::Char*"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primitives_use_crystal_width_types() {
        let m = CrystalMapper;
        assert_eq!(m.primitive(&PrimitiveType::Bool), "Bool");
        assert_eq!(m.primitive(&PrimitiveType::U32), "UInt32");
        assert_eq!(m.primitive(&PrimitiveType::I64), "Int64");
        assert_eq!(m.primitive(&PrimitiveType::F64), "Float64");
        assert_eq!(m.primitive(&PrimitiveType::Usize), "UInt64");
    }

    #[test]
    fn containers_use_crystal_generics() {
        assert_eq!(
            CrystalMapper.map_type(&TypeRef::Vec(Box::new(TypeRef::String))),
            "Array(String)"
        );
        assert_eq!(
            CrystalMapper.map_type(&TypeRef::Map(
                Box::new(TypeRef::String),
                Box::new(TypeRef::Primitive(PrimitiveType::I32))
            )),
            "Hash(String, Int32)"
        );
    }

    #[test]
    fn optional_uses_nilable_union() {
        assert_eq!(
            CrystalMapper.map_type(&TypeRef::Optional(Box::new(TypeRef::String))),
            "String?"
        );
    }

    #[test]
    fn bytes_and_json_and_unit() {
        assert_eq!(CrystalMapper.map_type(&TypeRef::Bytes), "Bytes");
        assert_eq!(CrystalMapper.map_type(&TypeRef::Json), "JSON::Any");
        assert_eq!(CrystalMapper.map_type(&TypeRef::Unit), "Nil");
    }

    #[test]
    fn named_types_are_pascal_case() {
        assert_eq!(
            CrystalMapper.map_type(&TypeRef::Named("parse_output".to_string())),
            "ParseOutput"
        );
    }

    #[test]
    fn c_type_scalars_pass_by_value_complex_as_char_ptr() {
        assert_eq!(crystal_c_type(&TypeRef::Primitive(PrimitiveType::I32)), "Int32");
        assert_eq!(crystal_c_type(&TypeRef::Unit), "Void");
        assert_eq!(crystal_c_type(&TypeRef::String), "LibC::Char*");
        assert_eq!(crystal_c_type(&TypeRef::Named("Foo".to_string())), "LibC::Char*");
    }
}
