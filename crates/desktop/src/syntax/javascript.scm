(function_declaration name: (identifier) @title.fn)
(function_expression name: (identifier) @title.fn)
(method_definition name: (property_identifier) @title.method)
(class_declaration name: (_) @title.class)
(variable_declarator name: (identifier) @title.fn value: [(function_expression) (arrow_function)])

(call_expression function: (identifier) @function.call)
(call_expression function: (member_expression property: (property_identifier) @function.method))
(new_expression constructor: (identifier) @function.constructor)

[(number) (true) (false) (null) (undefined)] @constant
[(string) (template_string) (regex)] @string
(comment) @comment
[(identifier) (property_identifier) (shorthand_property_identifier) (shorthand_property_identifier_pattern)] @variable

[
  "as" "async" "await" "break" "case" "catch" "class" "const" "continue"
  "debugger" "default" "delete" "do" "else" "export" "extends" "finally"
  "for" "from" "function" "get" "if" "import" "in" "instanceof" "let"
  "new" "of" "return" "set" "static" (super) "switch" "target" (this)
  "throw" "try" "typeof" "var" "void" "while" "with" "yield"
] @keyword

[
  "-" "--" "-=" "+" "++" "+=" "*" "*=" "**" "**=" "/" "/="
  "%" "%=" "<" "<=" "<<" "<<=" "=" "==" "===" "!" "!=" "!=="
  "=>" ">" ">=" ">>" ">>=" ">>>" ">>>=" "~" "^" "&" "|" "^="
  "&=" "|=" "&&" "||" "??" "&&=" "||=" "??="
] @operator

["(" ")" "[" "]" "{" "}"] @punctuation.bracket
[";" (optional_chain) "." ","] @punctuation.delimiter
