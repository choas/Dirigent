use std::sync::LazyLock;

use eframe::egui;
use egui_extras::syntax_highlighting::SyntectSettings;
use syntect::parsing::SyntaxDefinition;

/// Custom `SyntectSettings` that extends syntect's defaults with extra languages
/// (e.g. Kotlin) that are not shipped in the default Sublime Text syntax pack.
pub(crate) static SYNTAX_SETTINGS: LazyLock<SyntectSettings> = LazyLock::new(|| {
    let defaults = syntect::parsing::SyntaxSet::load_defaults_newlines();
    let mut builder = defaults.into_builder();

    // Kotlin
    if let Ok(def) = SyntaxDefinition::load_from_str(KOTLIN_SUBLIME_SYNTAX, true, None) {
        builder.add(def);
    }

    // Dart
    if let Ok(def) = SyntaxDefinition::load_from_str(DART_SUBLIME_SYNTAX, true, None) {
        builder.add(def);
    }

    SyntectSettings {
        ps: builder.build(),
        ts: syntect::highlighting::ThemeSet::load_defaults(),
    }
});

/// Convenience wrapper around `egui_extras::syntax_highlighting::highlight_with`
/// that uses our custom syntax settings (which include Kotlin, Dart, etc.).
///
/// Post-processes the LayoutJob to fix egui_extras bugs:
/// - Strips incorrect underlines (egui_extras checks ITALIC instead of UNDERLINE)
/// - Strips strikethrough for safety
/// - Clears per-section background colors and expansion to avoid colored rectangles
pub(crate) fn highlight(
    ctx: &egui::Context,
    style: &egui::Style,
    theme: &egui_extras::syntax_highlighting::CodeTheme,
    code: &str,
    language: &str,
) -> egui::text::LayoutJob {
    let mut job = egui_extras::syntax_highlighting::highlight_with(
        ctx,
        style,
        theme,
        code,
        language,
        &SYNTAX_SETTINGS,
    );
    for section in &mut job.sections {
        section.format.underline = egui::Stroke::NONE;
        section.format.strikethrough = egui::Stroke::NONE;
        section.format.background = egui::Color32::TRANSPARENT;
        section.format.expand_bg = 0.0;
    }
    job
}

// ---------------------------------------------------------------------------
// Embedded syntax definitions
// ---------------------------------------------------------------------------

const KOTLIN_SUBLIME_SYNTAX: &str = r#####"
%YAML 1.2
---
name: Kotlin
file_extensions: [kt, kts]
scope: source.kotlin

contexts:
  main:
    # Line comment
    - match: //
      scope: punctuation.definition.comment.kotlin
      push: line_comment

    # Block comment
    - match: /\*
      scope: punctuation.definition.comment.begin.kotlin
      push: block_comment

    # Annotations
    - match: '@\w+'
      scope: storage.type.annotation.kotlin

    # Raw string literals (triple-quoted)
    - match: '"""'
      scope: punctuation.definition.string.begin.kotlin
      push: raw_string

    # Regular string literals
    - match: '"'
      scope: punctuation.definition.string.begin.kotlin
      push: string

    # Character literals
    - match: "'.'"
      scope: string.quoted.single.kotlin
    - match: "'\\\\[tnrb'\"\\\\$]'"
      scope: string.quoted.single.kotlin

    # Numeric literals
    - match: '\b0[xX][0-9a-fA-F_]+[uUlL]*\b'
      scope: constant.numeric.hex.kotlin
    - match: '\b0[bB][01_]+[uUlL]*\b'
      scope: constant.numeric.binary.kotlin
    - match: '\b[0-9][0-9_]*\.[0-9][0-9_]*([eE][+-]?[0-9_]+)?[fF]?\b'
      scope: constant.numeric.float.kotlin
    - match: '\b[0-9][0-9_]*[eE][+-]?[0-9_]+[fF]?\b'
      scope: constant.numeric.float.kotlin
    - match: '\b[0-9][0-9_]*[fF]\b'
      scope: constant.numeric.float.kotlin
    - match: '\b[0-9][0-9_]*[uUlL]*\b'
      scope: constant.numeric.integer.kotlin

    # Null / booleans
    - match: '\b(null|true|false)\b'
      scope: constant.language.kotlin

    # Hard keywords
    - match: '\b(as|break|class|continue|do|else|for|fun|if|in|interface|is|object|package|return|super|this|throw|try|typealias|val|var|when|while)\b'
      scope: keyword.control.kotlin
    - match: '\b(typeof)\b'
      scope: keyword.control.kotlin

    # Soft keywords and modifiers
    - match: '\b(by|catch|constructor|delegate|dynamic|field|file|finally|get|import|init|param|property|receiver|set|setparam|where)\b'
      scope: keyword.other.kotlin
    - match: '\b(abstract|actual|annotation|companion|const|crossinline|data|enum|expect|external|final|infix|inline|inner|internal|lateinit|noinline|open|operator|out|override|private|protected|public|reified|sealed|suspend|tailrec|value|vararg)\b'
      scope: storage.modifier.kotlin

    # Built-in types
    - match: '\b(Any|Boolean|Byte|Char|Double|Float|Int|Long|Nothing|Number|Short|String|UByte|UInt|ULong|UShort|Unit)\b'
      scope: support.type.kotlin
    - match: '\b(Array|List|Map|MutableList|MutableMap|MutableSet|Pair|Set|Triple|Sequence)\b'
      scope: support.type.kotlin

    # Lambda arrow
    - match: '->'
      scope: keyword.operator.arrow.kotlin

    # Operators
    - match: '[!&|<>=+\-*/%^~?:]+'
      scope: keyword.operator.kotlin

    # Braces, brackets, parens
    - match: '[{}()\[\]]'
      scope: punctuation.section.kotlin

    # Semicolons and commas
    - match: '[;,]'
      scope: punctuation.separator.kotlin

  line_comment:
    - meta_scope: comment.line.double-slash.kotlin
    - match: $\n?
      pop: true

  block_comment:
    - meta_scope: comment.block.kotlin
    # Nested block comments
    - match: /\*
      push: block_comment
    - match: \*/
      scope: punctuation.definition.comment.end.kotlin
      pop: true

  string:
    - meta_scope: string.quoted.double.kotlin
    - match: '\\[tnrb''"\\$]'
      scope: constant.character.escape.kotlin
    - match: '\\u[0-9a-fA-F]{4}'
      scope: constant.character.escape.kotlin
    - match: '\$\{'
      scope: punctuation.section.interpolation.begin.kotlin
      push: string_interpolation_braced
    - match: '\$\w+'
      scope: variable.other.interpolation.kotlin
    - match: '"'
      scope: punctuation.definition.string.end.kotlin
      pop: true

  raw_string:
    - meta_scope: string.quoted.triple.kotlin
    - match: '\$\{'
      scope: punctuation.section.interpolation.begin.kotlin
      push: string_interpolation_braced
    - match: '\$\w+'
      scope: variable.other.interpolation.kotlin
    - match: '"""'
      scope: punctuation.definition.string.end.kotlin
      pop: true

  string_interpolation_braced:
    - meta_scope: source.kotlin.embedded
    - match: '\}'
      scope: punctuation.section.interpolation.end.kotlin
      pop: true
    - include: main
"#####;

const DART_SUBLIME_SYNTAX: &str = r#####"
%YAML 1.2
---
name: Dart
file_extensions: [dart]
scope: source.dart

contexts:
  main:
    # Line comment
    - match: //
      scope: punctuation.definition.comment.dart
      push: line_comment

    # Block comment
    - match: /\*
      scope: punctuation.definition.comment.begin.dart
      push: block_comment

    # Annotations / metadata
    - match: '@\w+'
      scope: storage.type.annotation.dart

    # Raw strings
    - match: "r'''"
      scope: punctuation.definition.string.begin.dart
      push: raw_triple_single_string
    - match: 'r"""'
      scope: punctuation.definition.string.begin.dart
      push: raw_triple_double_string
    - match: "r'"
      scope: punctuation.definition.string.begin.dart
      push: raw_single_string
    - match: 'r"'
      scope: punctuation.definition.string.begin.dart
      push: raw_double_string

    # Triple-quoted strings
    - match: "'''"
      scope: punctuation.definition.string.begin.dart
      push: triple_single_string
    - match: '"""'
      scope: punctuation.definition.string.begin.dart
      push: triple_double_string

    # Regular strings
    - match: "'"
      scope: punctuation.definition.string.begin.dart
      push: single_string
    - match: '"'
      scope: punctuation.definition.string.begin.dart
      push: double_string

    # Numeric literals
    - match: '\b0[xX][0-9a-fA-F]+\b'
      scope: constant.numeric.hex.dart
    - match: '\b[0-9]+\.[0-9]+([eE][+-]?[0-9]+)?\b'
      scope: constant.numeric.float.dart
    - match: '\b[0-9]+[eE][+-]?[0-9]+\b'
      scope: constant.numeric.float.dart
    - match: '\b[0-9]+\b'
      scope: constant.numeric.integer.dart

    # Built-in constants
    - match: '\b(null|true|false)\b'
      scope: constant.language.dart

    # Keywords
    - match: '\b(abstract|as|assert|async|await|base|break|case|catch|class|const|continue|covariant|default|deferred|do|dynamic|else|enum|export|extends|extension|external|factory|final|finally|for|Function|get|hide|if|implements|import|in|interface|is|late|library|mixin|new|of|on|operator|part|required|rethrow|return|sealed|set|show|static|super|switch|sync|this|throw|try|typedef|var|void|when|while|with|yield)\b'
      scope: keyword.control.dart

    # Built-in types
    - match: '\b(bool|double|dynamic|int|num|Object|String|void|Never|Null|Future|FutureOr|Iterable|Iterator|List|Map|Set|Stream|Type|Symbol|Record|Duration|DateTime|RegExp|Function)\b'
      scope: support.type.dart

    # Lambda arrow
    - match: '=>'
      scope: keyword.operator.arrow.dart

    # Operators
    - match: '[!&|<>=+\-*/%^~?:]+'
      scope: keyword.operator.dart

    # Cascade operator
    - match: '\.\.'
      scope: keyword.operator.cascade.dart

    # Braces, brackets, parens
    - match: '[{}()\[\]]'
      scope: punctuation.section.dart

    # Semicolons and commas
    - match: '[;,]'
      scope: punctuation.separator.dart

  line_comment:
    - meta_scope: comment.line.double-slash.dart
    - match: $\n?
      pop: true

  block_comment:
    - meta_scope: comment.block.dart
    - match: /\*
      push: block_comment
    - match: \*/
      scope: punctuation.definition.comment.end.dart
      pop: true

  # --- String contexts ---

  single_string:
    - meta_scope: string.quoted.single.dart
    - match: '\\.'
      scope: constant.character.escape.dart
    - match: '\$\{'
      scope: punctuation.section.interpolation.begin.dart
      push: string_interpolation
    - match: '\$\w+'
      scope: variable.other.interpolation.dart
    - match: "'"
      scope: punctuation.definition.string.end.dart
      pop: true

  double_string:
    - meta_scope: string.quoted.double.dart
    - match: '\\.'
      scope: constant.character.escape.dart
    - match: '\$\{'
      scope: punctuation.section.interpolation.begin.dart
      push: string_interpolation
    - match: '\$\w+'
      scope: variable.other.interpolation.dart
    - match: '"'
      scope: punctuation.definition.string.end.dart
      pop: true

  triple_single_string:
    - meta_scope: string.quoted.triple.dart
    - match: '\\.'
      scope: constant.character.escape.dart
    - match: '\$\{'
      scope: punctuation.section.interpolation.begin.dart
      push: string_interpolation
    - match: '\$\w+'
      scope: variable.other.interpolation.dart
    - match: "'''"
      scope: punctuation.definition.string.end.dart
      pop: true

  triple_double_string:
    - meta_scope: string.quoted.triple.dart
    - match: '\\.'
      scope: constant.character.escape.dart
    - match: '\$\{'
      scope: punctuation.section.interpolation.begin.dart
      push: string_interpolation
    - match: '\$\w+'
      scope: variable.other.interpolation.dart
    - match: '"""'
      scope: punctuation.definition.string.end.dart
      pop: true

  raw_single_string:
    - meta_scope: string.quoted.single.raw.dart
    - match: "'"
      scope: punctuation.definition.string.end.dart
      pop: true

  raw_double_string:
    - meta_scope: string.quoted.double.raw.dart
    - match: '"'
      scope: punctuation.definition.string.end.dart
      pop: true

  raw_triple_single_string:
    - meta_scope: string.quoted.triple.raw.dart
    - match: "'''"
      scope: punctuation.definition.string.end.dart
      pop: true

  raw_triple_double_string:
    - meta_scope: string.quoted.triple.raw.dart
    - match: '"""'
      scope: punctuation.definition.string.end.dart
      pop: true

  string_interpolation:
    - meta_scope: source.dart.embedded
    - match: '\}'
      scope: punctuation.section.interpolation.end.dart
      pop: true
    - include: main
"#####;
