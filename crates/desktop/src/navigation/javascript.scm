[
  (statement_block)
  (function_declaration)
  (generator_function_declaration)
  (function_expression)
  (generator_function)
  (arrow_function)
  (method_definition)
  (class_declaration)
  (class)
  (class_body)
  (for_statement)
  (for_in_statement)
  (catch_clause)
] @scope

(function_declaration name: (identifier) @item) @definition
(generator_function_declaration name: (identifier) @item) @definition
(class_declaration name: (identifier) @item) @definition
(method_definition name: (property_identifier) @item) @definition
(field_definition property: (property_identifier) @item) @definition
(program (lexical_declaration (variable_declarator name: (_) @item)) @definition)
(program (variable_declaration (variable_declarator name: (_) @item)) @definition)
(export_statement declaration: (lexical_declaration (variable_declarator name: (_) @item)) @definition)
(export_statement declaration: (variable_declaration (variable_declarator name: (_) @item)) @definition)

(variable_declarator name: (_) @local) @definition
(function_expression name: (identifier) @local)
(class name: (identifier) @local)
(formal_parameters (_) @local)
(arrow_function parameter: (identifier) @local)
(for_in_statement left: (_) @local)
(catch_clause parameter: (_) @local)

(import_clause (identifier) @import)
(import_specifier alias: (identifier) @import)
(import_specifier name: (identifier) @import !alias)
(namespace_import (identifier) @import)
