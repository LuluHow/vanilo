use std::path::Path;

use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::SourceType;
use oxc_transformer::{TransformOptions, Transformer};

/// Transpiles TypeScript source code to JavaScript.
/// Type annotations, interfaces, type aliases, enums, and other TS-only syntax are stripped.
pub fn strip_types(source: &str) -> Result<String, String> {
    let allocator = Allocator::default();
    let source_type = SourceType::from_path("file.ts")
        .map_err(|e| format!("source type: {e}"))?;

    let mut ret = Parser::new(&allocator, source, source_type).parse();
    if !ret.errors.is_empty() {
        let msgs: Vec<String> = ret.errors.iter().map(|e| e.to_string()).collect();
        return Err(msgs.join("\n"));
    }

    let sem_ret = SemanticBuilder::new()
        .build(&ret.program);

    let options = TransformOptions::default();

    let transformer_ret = Transformer::new(&allocator, Path::new("file.ts"), &options)
        .build_with_scoping(
            sem_ret.semantic.into_scoping(),
            &mut ret.program,
        );

    if !transformer_ret.errors.is_empty() {
        let msgs: Vec<String> = transformer_ret.errors.iter().map(|e| e.to_string()).collect();
        return Err(msgs.join("\n"));
    }

    let code = Codegen::new().build(&ret.program);
    Ok(code.code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_basic_types() {
        let ts = "function handler(req: Request): Response { return { status: 200 }; }";
        let js = strip_types(ts).unwrap();
        assert!(!js.contains(": Request"));
        assert!(!js.contains(": Response"));
        assert!(js.contains("status: 200"));
    }

    #[test]
    fn strip_interface() {
        let ts = "interface User { name: string; age: number; }\nconst x = 1;";
        let js = strip_types(ts).unwrap();
        assert!(!js.contains("interface"));
        assert!(js.contains("const x = 1"));
    }

    #[test]
    fn strip_type_alias() {
        let ts = "type ID = string;\nconst id = \"abc\";";
        let js = strip_types(ts).unwrap();
        assert!(!js.contains("type ID"));
        assert!(js.contains("const id"));
    }

    #[test]
    fn strip_generics() {
        let ts = "function identity<T>(x: T): T { return x; }";
        let js = strip_types(ts).unwrap();
        assert!(!js.contains("<T>"));
        assert!(!js.contains(": T"));
        assert!(js.contains("return x"));
    }

    #[test]
    fn strip_as_cast() {
        let ts = "const x = y as string;";
        let js = strip_types(ts).unwrap();
        assert!(!js.contains("as string"));
        assert!(js.contains("const x = y"));
    }

    #[test]
    fn strip_enum() {
        let ts = "enum Status { Active, Inactive }\nconst s = Status.Active;";
        let js = strip_types(ts).unwrap();
        assert!(js.contains("Status"));
        assert!(js.contains("Active"));
    }

    #[test]
    fn strip_optional() {
        let ts = "function f(x?: number) { return x; }";
        let js = strip_types(ts).unwrap();
        assert!(!js.contains("number"));
        assert!(js.contains("return x"));
    }

    #[test]
    fn preserve_runtime_code() {
        let ts = r#"
interface Req { method: string; path: string; }
type Status = number;

function handler(req: Req): { status: Status; body: string } {
    const items: string[] = ["a", "b"];
    return { status: 200, body: JSON.stringify(items) };
}
"#;
        let js = strip_types(ts).unwrap();
        assert!(js.contains("function handler(req)"));
        assert!(js.contains("JSON.stringify(items)"));
        assert!(js.contains("status: 200"));
        assert!(!js.contains("interface Req"));
        assert!(!js.contains("type Status"));
    }

    #[test]
    fn error_on_invalid_ts() {
        let ts = "function f(x: { return x; }";
        let result = strip_types(ts);
        assert!(result.is_err());
    }

    #[test]
    fn strip_readonly() {
        let ts = "class Foo { readonly bar: string = \"hi\"; }";
        let js = strip_types(ts).unwrap();
        assert!(!js.contains("readonly"));
        assert!(js.contains("bar"));
    }
}
