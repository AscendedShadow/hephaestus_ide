[
  (block)
  (function_declaration)
  (method_declaration)
  (method_elem)
  (func_literal)
  (function_type)
  (type_spec)
  (for_statement)
  (if_statement)
  (expression_switch_statement)
  (type_switch_statement)
  (select_statement)
  (expression_case)
  (type_case)
  (default_case)
  (communication_case)
] @scope

(function_declaration name: (identifier) @item) @definition
(method_declaration name: (field_identifier) @item) @definition
(method_elem name: (field_identifier) @item) @definition
(type_spec name: (type_identifier) @item) @definition
(type_alias name: (type_identifier) @item) @definition
(field_declaration name: (field_identifier) @item) @definition
(source_file (const_declaration (const_spec name: (identifier) @item)) @definition)
(source_file (var_declaration (var_spec name: (identifier) @item)) @definition)

(const_spec name: (identifier) @local) @definition
(var_spec name: (identifier) @local) @definition
(short_var_declaration left: (expression_list (identifier) @local)) @definition
(range_clause left: (expression_list (identifier) @local)) @definition
(type_switch_statement alias: (expression_list (identifier) @local))
(parameter_declaration name: (identifier) @local)
(variadic_parameter_declaration name: (identifier) @local)
(type_parameter_declaration name: (identifier) @local)

(import_spec name: (package_identifier) @import)
