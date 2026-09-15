(method_declaration name: (identifier) @title.method)
(local_function_statement name: (identifier) @title.fn)
(constructor_declaration name: (identifier) @title.constructor)
(destructor_declaration name: (identifier) @title.destructor)
(class_declaration name: (identifier) @title.class)
(interface_declaration name: (identifier) @title.interface)
(enum_declaration name: (identifier) @title.enum)
(struct_declaration (identifier) @title.struct)
(record_declaration (identifier) @title.record)

(invocation_expression (identifier) @function.call)
(invocation_expression (member_access_expression name: (identifier) @function.method))

[
  (predefined_type)
  (generic_name (identifier))
  (type_parameter (identifier))
] @type
(_ type: (identifier) @type)
(base_list (identifier) @type)

[(real_literal) (integer_literal) (boolean_literal) (null_literal)] @constant
[
  (character_literal)
  (string_literal)
  (raw_string_literal)
  (verbatim_string_literal)
  (interpolated_string_expression)
] @string

(comment) @comment
(attribute) @attribute
(identifier) @variable

[
  (modifier) (implicit_type) "this" "add" "alias" "as" "base" "break"
  "case" "catch" "checked" "class" "continue" "default" "delegate" "do"
  "else" "enum" "event" "explicit" "extern" "finally" "for" "foreach"
  "global" "goto" "if" "implicit" "interface" "is" "lock" "namespace"
  "notnull" "operator" "params" "return" "remove" "sizeof" "stackalloc"
  "static" "struct" "switch" "throw" "try" "typeof" "unchecked" "using"
  "while" "new" "await" "in" "yield" "get" "set" "when" "out" "ref"
  "from" "where" "select" "record" "init" "with" "let"
] @keyword

[
  "--" "-" "-=" "&" "&=" "&&" "+" "++" "+=" "<" "<=" "<<"
  "<<=" "=" "==" "!" "!=" "=>" ">" ">=" ">>" ">>=" ">>>"
  ">>>=" "|" "|=" "||" "?" "??" "??=" "^" "^=" "~" "*" "*="
  "/" "/=" "%" "%=" ":" ".."
] @operator

["(" ")" "[" "]" "{" "}" (interpolation_brace)] @punctuation.bracket
[";" "." ","] @punctuation.delimiter
