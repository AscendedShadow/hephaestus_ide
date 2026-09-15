(line_comment !doc) @comment
(block_comment !doc) @comment
(line_comment doc: (_)) @comment.doc
(block_comment doc: (_)) @comment.doc

(attribute_item) @attribute
(inner_attribute_item) @attribute
(attribute (identifier) @attribute.path)
(attribute (scoped_identifier (identifier) @attribute.path))
(attribute arguments: (token_tree (identifier) @attribute.argument))

(function_item name: (identifier) @title.fn)
(function_signature_item name: (identifier) @title.fn)
(macro_definition name: (identifier) @title.macro)
(struct_item name: (type_identifier) @title.struct)
(enum_item name: (type_identifier) @title.enum)
(enum_variant name: (identifier) @title.variant)
(union_item name: (type_identifier) @title.union)
(trait_item name: (type_identifier) @title.trait)
(type_item name: (type_identifier) @title.type)
(associated_type name: (type_identifier) @title.type)
(const_item name: (identifier) @title.const)
(static_item name: (identifier) @title.static)
(mod_item name: (identifier) @title.mod)

(call_expression function: (identifier) @function.call)
(call_expression function: (scoped_identifier name: (identifier) @function.path))
(call_expression function: (field_expression field: (field_identifier) @function.method))
(generic_function function: (identifier) @function.call)
(generic_function function: (scoped_identifier name: (identifier) @function.path))
(generic_function function: (field_expression field: (field_identifier) @function.method))

(macro_invocation macro: (identifier) @function.macro)
(macro_invocation macro: (scoped_identifier name: (identifier) @function.macro))
(macro_invocation "!" @function.bang)

(token_tree (token_tree "(" @punctuation.bracket))
(token_tree
  (identifier) @function.token
  .
  (token_tree "("))

(token_tree "!" @operator)
(token_tree
  (identifier) @function.token
  .
  "!" @function.bang)

(token_tree "::" @punctuation.delimiter)
(token_tree
  (identifier) @type.token
  .
  "::")

[
  (type_identifier)
  (primitive_type)
] @type

(scoped_identifier path: (identifier) @type.path)
(scoped_identifier path: (scoped_identifier name: (identifier) @type.path))
(scoped_type_identifier path: (identifier) @type.path)
(scoped_type_identifier path: (scoped_identifier name: (identifier) @type.path))
(scoped_use_list path: (identifier) @type.path)
(scoped_use_list path: (scoped_identifier name: (identifier) @type.path))

[
  (integer_literal)
  (float_literal)
  (boolean_literal)
] @constant

[
  (string_literal)
  (raw_string_literal)
  (char_literal)
] @string

[
  (identifier)
  (field_identifier)
  (shorthand_field_identifier)
  (metavariable)
] @variable

(lifetime "'" @keyword.quote)
(lifetime (identifier) @keyword.lifetime)
(label "'" @keyword.quote)
(label (identifier) @keyword.label)

[
  "as"
  "async"
  "await"
  "break"
  "const"
  "continue"
  "default"
  "dyn"
  "else"
  "enum"
  "extern"
  "fn"
  "for"
  "gen"
  "if"
  "impl"
  "in"
  "let"
  "loop"
  "macro_rules!"
  "match"
  "mod"
  "move"
  "pub"
  "raw"
  "ref"
  "return"
  "static"
  "struct"
  "trait"
  "try"
  "type"
  "union"
  "unsafe"
  "use"
  "where"
  "while"
  "yield"
  (crate)
  (mutable_specifier)
  (self)
  (super)
] @keyword

[
  "!"
  "!="
  "%"
  "%="
  "&"
  "&&"
  "&="
  "*"
  "*="
  "+"
  "+="
  "-"
  "-="
  "->"
  ".."
  "..."
  "..="
  "/"
  "/="
  "<"
  "<<"
  "<<="
  "<="
  "="
  "=="
  "=>"
  ">"
  ">="
  ">>"
  ">>="
  "?"
  "@"
  "^"
  "^="
  "|"
  "|="
  "||"
  "_"
] @operator

[
  "("
  ")"
  "["
  "]"
  "{"
  "}"
] @punctuation.bracket

[
  "#"
  "$"
  ","
  "."
  ":"
  "::"
  ";"
] @punctuation.delimiter
