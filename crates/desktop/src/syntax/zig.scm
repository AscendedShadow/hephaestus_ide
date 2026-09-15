(function_declaration name: (identifier) @title.fn)
(variable_declaration (identifier) @title.type "=" [(struct_declaration) (enum_declaration) (union_declaration) (opaque_declaration)])

(call_expression function: (identifier) @function.call)
(call_expression function: (field_expression member: (identifier) @function.method))
(builtin_identifier) @function.builtin

[(builtin_type) "anyframe"] @type
[(integer) (float) (boolean) "null" "unreachable" "undefined"] @constant
[(character) (string) (multiline_string)] @string
(comment) @comment
[(identifier) (builtin_identifier)] @variable

[
  "asm" "defer" "errdefer" "test" "error" "const" "var" "struct" "union"
  "enum" "opaque" "async" "await" "suspend" "nosuspend" "resume" "fn"
  "and" "or" "orelse" "return" "if" "else" "switch" "for" "while" "break"
  "continue" "usingnamespace" "export" "try" "catch" "volatile" "allowzero"
  "noalias" "addrspace" "align" "callconv" "linksection" "pub" "inline"
  "noinline" "extern" "comptime" "packed" "threadlocal"
] @keyword

[
  "=" "*=" "*%=" "*|=" "/=" "%=" "+=" "+%=" "+|=" "-=" "-%="
  "-|=" "<<=" "<<|=" ">>=" "&=" "^=" "|=" "!" "~" "-" "-%" "&"
  "==" "!=" ">" ">=" "<=" "<" "^" "|" "<<" ">>" "<<|" "+" "++"
  "+%" "+|" "-|" "*" "/" "%" "**" "*%" "*|" "||" ".*" ".?" "?" ".."
] @operator

["[" "]" "(" ")" "{" "}"] @punctuation.bracket
[";" "." "," ":" "=>" "->"] @punctuation.delimiter
