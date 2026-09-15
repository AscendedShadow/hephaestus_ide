(function_definition declarator: (function_declarator declarator: (identifier) @title.fn))
(declaration declarator: (function_declarator declarator: (identifier) @title.fn))
(type_definition declarator: (type_identifier) @title.type)

(call_expression function: (identifier) @function.call)
(call_expression function: (field_expression field: (field_identifier) @function.method))

[
  (type_identifier)
  (primitive_type)
  (sized_type_specifier)
] @type

[
  (number_literal)
  (char_literal)
  (null)
] @constant

[
  (string_literal)
  (system_lib_string)
] @string

(comment) @comment

[
  (identifier)
  (field_identifier)
  (statement_identifier)
] @variable

[
  "break" "case" "const" "continue" "default" "do" "else" "enum"
  "extern" "for" "if" "inline" "return" "sizeof" "static" "struct"
  "switch" "typedef" "union" "volatile" "while"
  "#define" "#elif" "#else" "#endif" "#if" "#ifdef" "#ifndef" "#include"
  (preproc_directive)
] @keyword

[
  "--" "-" "-=" "->" "=" "!=" "*" "*=" "/" "/=" "%" "%="
  "&" "&&" "&=" "+" "++" "+=" "<" "<<" "<<=" "<=" "==" ">"
  ">=" ">>" ">>=" "|" "||" "|=" "^" "^=" "!" "~" "?"
] @operator

["(" ")" "[" "]" "{" "}"] @punctuation.bracket
["." "," ":" ";"] @punctuation.delimiter
