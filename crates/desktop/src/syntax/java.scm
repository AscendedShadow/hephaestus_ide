(method_declaration name: (identifier) @title.method)
(constructor_declaration name: (identifier) @title.constructor)
(class_declaration name: (identifier) @title.class)
(interface_declaration name: (identifier) @title.interface)
(enum_declaration name: (identifier) @title.enum)
(record_declaration name: (identifier) @title.record)
(enum_constant name: (identifier) @title.variant)

(method_invocation name: (identifier) @function.method)

[
  (type_identifier)
  (boolean_type)
  (integral_type)
  (floating_point_type)
  (void_type)
] @type

[
  (hex_integer_literal)
  (decimal_integer_literal)
  (octal_integer_literal)
  (decimal_floating_point_literal)
  (hex_floating_point_literal)
  (true)
  (false)
  (null_literal)
] @constant

[(character_literal) (string_literal)] @string
[(line_comment) (block_comment)] @comment
[(annotation) (marker_annotation)] @attribute
(identifier) @variable

[
  "abstract" "assert" "break" "case" "catch" "class" "continue" "default"
  "do" "else" "enum" "exports" "extends" "final" "finally" "for" "if"
  "implements" "import" "instanceof" "interface" "module" "native" "new"
  "non-sealed" "open" "opens" "package" "permits" "private" "protected"
  "provides" "public" "record" "requires" "return" "sealed" "static"
  "strictfp" (super) "switch" "synchronized" (this) "throw" "throws" "to"
  "transient" "transitive" "try" "uses" "volatile" "when" "while" "with" "yield"
] @keyword

[
  "=" ">" "<" "!" "~" "?" ":" "->" "==" ">=" "<=" "!=" "&&"
  "||" "++" "--" "+" "-" "*" "/" "&" "|" "^" "%" "<<" ">>"
  ">>>" "+=" "-=" "*=" "/=" "&=" "|=" "^=" "%=" "<<=" ">>=" ">>>="
] @operator

["(" ")" "[" "]" "{" "}"] @punctuation.bracket
["." "," ";" "@"] @punctuation.delimiter
