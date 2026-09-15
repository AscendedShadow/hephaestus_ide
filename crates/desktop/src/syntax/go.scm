(function_declaration name: (identifier) @title.fn)
(method_declaration name: (field_identifier) @title.method)
(type_declaration (type_spec name: (type_identifier) @title.type))

(call_expression function: (identifier) @function.call)
(call_expression function: (selector_expression field: (field_identifier) @function.method))

(type_identifier) @type
[(int_literal) (float_literal) (imaginary_literal) (true) (false) (nil) (iota)] @constant
[(interpreted_string_literal) (raw_string_literal) (rune_literal)] @string
(comment) @comment
[(identifier) (field_identifier)] @variable

[
  "break" "case" "chan" "const" "continue" "default" "defer" "else"
  "fallthrough" "for" "func" "go" "goto" "if" "import" "interface"
  "map" "package" "range" "return" "select" "struct" "switch" "type" "var"
] @keyword

[
  "--" "-" "-=" ":=" "!" "!=" "..." "*" "*=" "/" "/=" "&"
  "&&" "&=" "%" "%=" "^" "^=" "+" "++" "+=" "<-" "<" "<<"
  "<<=" "<=" "=" "==" ">" ">=" ">>" ">>=" "|" "|=" "||" "~"
] @operator

["(" ")" "[" "]" "{" "}"] @punctuation.bracket
["." "," ":" ";"] @punctuation.delimiter
