//! File extension to language mapping for the `languages` metric.

/// Returns the language name for a file name, or `None` when the file has no
/// recognizable extension at all.
pub fn detect(file_name: &str) -> Option<String> {
    if let Some(name) = well_known(file_name) {
        return Some(name.to_string());
    }
    let ext = extension_of(file_name)?;
    let lower = ext.to_ascii_lowercase();
    Some(
        by_extension(&lower)
            .map(|name| name.to_string())
            .unwrap_or_else(|| format!(".{lower}")),
    )
}

/// The extension of a file name without the leading dot. Dot files such as
/// `.gitignore` have no extension.
pub fn extension_of(file_name: &str) -> Option<&str> {
    let (stem, ext) = file_name.rsplit_once('.')?;
    if stem.is_empty() || ext.is_empty() {
        return None;
    }
    Some(ext)
}

/// The extension with its leading dot, used when matching `ignoring_extensions`.
pub fn dotted_extension(file_name: &str) -> Option<String> {
    extension_of(file_name).map(|ext| format!(".{ext}"))
}

fn well_known(file_name: &str) -> Option<&'static str> {
    Some(match file_name {
        "Makefile" | "makefile" | "GNUmakefile" => "Makefile",
        "Dockerfile" | "Containerfile" => "Dockerfile",
        "CMakeLists.txt" => "CMake",
        "Cargo.lock" => "TOML",
        "go.mod" | "go.sum" => "Go",
        _ => return None,
    })
}

fn by_extension(ext: &str) -> Option<&'static str> {
    Some(match ext {
        "rs" => "Rust",
        "c" | "h" => "C",
        "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" => "C++",
        "cs" => "C#",
        "java" => "Java",
        "kt" | "kts" => "Kotlin",
        "swift" => "Swift",
        "m" | "mm" => "Objective-C",
        "go" => "Go",
        "py" | "pyi" | "pyw" => "Python",
        "rb" => "Ruby",
        "php" => "PHP",
        "pl" | "pm" => "Perl",
        "lua" => "Lua",
        "dart" => "Dart",
        "scala" | "sc" => "Scala",
        "hs" => "Haskell",
        "ex" | "exs" => "Elixir",
        "erl" | "hrl" => "Erlang",
        "clj" | "cljs" | "cljc" => "Clojure",
        "zig" => "Zig",
        "nim" => "Nim",
        "jl" => "Julia",
        "r" => "R",
        "sql" => "SQL",
        "sh" | "bash" | "zsh" | "fish" => "Shell",
        "ps1" | "psm1" => "PowerShell",
        "bat" | "cmd" => "Batch",
        "js" | "mjs" | "cjs" => "JavaScript",
        "jsx" => "JavaScript (JSX)",
        "ts" | "mts" | "cts" => "TypeScript",
        "tsx" => "TypeScript (TSX)",
        "vue" => "Vue",
        "svelte" => "Svelte",
        "html" | "htm" => "HTML",
        "css" => "CSS",
        "scss" | "sass" => "Sass",
        "less" => "Less",
        "uss" => "USS",
        "uxml" => "UXML",
        "xml" => "XML",
        "toml" => "TOML",
        "ini" | "cfg" | "conf" => "INI",
        "gradle" => "Gradle",
        "shader" | "cginc" | "hlsl" | "compute" => "HLSL",
        "glsl" | "vert" | "frag" => "GLSL",
        "wgsl" => "WGSL",
        "metal" => "Metal",
        "asmdef" | "asmref" => "Unity Assembly Definition",
        "proto" => "Protocol Buffers",
        "graphql" | "gql" => "GraphQL",
        "tf" => "Terraform",
        "vim" => "Vim Script",
        "asm" | "s" => "Assembly",
        "f90" | "f95" | "f03" => "Fortran",
        "vb" => "Visual Basic",
        "pas" => "Pascal",
        "d" => "D",
        "cr" => "Crystal",
        "sol" => "Solidity",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_known_and_unknown_extensions() {
        assert_eq!(detect("main.rs").as_deref(), Some("Rust"));
        assert_eq!(detect("App.tsx").as_deref(), Some("TypeScript (TSX)"));
        assert_eq!(detect("thing.qqq").as_deref(), Some(".qqq"));
        assert_eq!(detect("Makefile").as_deref(), Some("Makefile"));
        assert_eq!(detect(".gitignore"), None);
    }

    #[test]
    fn extensions_ignore_leading_dots() {
        assert_eq!(extension_of(".gitignore"), None);
        assert_eq!(dotted_extension("notes.md").as_deref(), Some(".md"));
    }
}
