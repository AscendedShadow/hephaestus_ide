[
  (compound_statement)
  (function_definition)
  (for_statement)
] @scope

(function_definition declarator: (_) @item) @definition
(translation_unit
  (declaration declarator: (init_declarator declarator: (_) @item)) @definition)
(translation_unit
  (declaration
    declarator: [
      (identifier)
      (pointer_declarator)
      (array_declarator)
      (function_declarator)
      (parenthesized_declarator)
    ] @item) @definition)
(type_definition declarator: (_) @item) @definition
(field_declaration declarator: (_) @item) @definition
(struct_specifier name: (type_identifier) @item body: (_)) @definition
(union_specifier name: (type_identifier) @item body: (_)) @definition
(enum_specifier name: (type_identifier) @item body: (_)) @definition
(enumerator name: (identifier) @item) @definition
(preproc_def name: (identifier) @item) @definition
(preproc_function_def name: (identifier) @item) @definition

(declaration declarator: (init_declarator declarator: (_) @local)) @definition
(declaration
  declarator: [
    (identifier)
    (pointer_declarator)
    (array_declarator)
    (function_declarator)
    (parenthesized_declarator)
  ] @local) @definition
(function_definition
  declarator: (function_declarator
    parameters: (parameter_list (parameter_declaration declarator: (_) @local))))
(function_definition
  declarator: (pointer_declarator
    declarator: (function_declarator
      parameters: (parameter_list (parameter_declaration declarator: (_) @local)))))
