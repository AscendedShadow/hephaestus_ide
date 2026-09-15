[
  (statement_block)
  (function_declaration)
  (generator_function_declaration)
  (function_expression)
  (generator_function)
  (arrow_function)
  (method_definition)
  (method_signature)
  (abstract_method_signature)
  (function_signature)
  (call_signature)
  (construct_signature)
  (function_type)
  (class_declaration)
  (abstract_class_declaration)
  (class)
  (class_body)
  (interface_declaration)
  (type_alias_declaration)
  (for_statement)
  (for_in_statement)
  (catch_clause)
] @scope

(function_declaration name: (identifier) @item) @definition
(generator_function_declaration name: (identifier) @item) @definition
(function_signature name: (identifier) @item) @definition
(class_declaration name: (type_identifier) @item) @definition
(abstract_class_declaration name: (type_identifier) @item) @definition
(interface_declaration name: (type_identifier) @item) @definition
(type_alias_declaration name: (type_identifier) @item) @definition
(enum_declaration name: (identifier) @item) @definition
(enum_body name: (property_identifier) @item)
(enum_assignment name: (property_identifier) @item) @definition
(internal_module name: (identifier) @item) @definition
(method_definition name: (property_identifier) @item) @definition
(method_signature name: (property_identifier) @item) @definition
(abstract_method_signature name: (property_identifier) @item) @definition
(property_signature name: (property_identifier) @item) @definition
(public_field_definition name: (property_identifier) @item) @definition
(required_parameter (accessibility_modifier) pattern: (identifier) @item)
(program (lexical_declaration (variable_declarator name: (_) @item)) @definition)
(program (variable_declaration (variable_declarator name: (_) @item)) @definition)
(export_statement declaration: (lexical_declaration (variable_declarator name: (_) @item)) @definition)
(export_statement declaration: (variable_declaration (variable_declarator name: (_) @item)) @definition)

(type_parameter name: (type_identifier) @local)
(variable_declarator name: (_) @local) @definition
(function_expression name: (identifier) @local)
(class name: (type_identifier) @local)
(required_parameter pattern: (_) @local)
(optional_parameter pattern: (_) @local)
(arrow_function parameter: (identifier) @local)
(for_in_statement left: (_) @local)
(catch_clause parameter: (_) @local)

(import_clause (identifier) @import)
(import_specifier alias: (identifier) @import)
(import_specifier name: (identifier) @import !alias)
(namespace_import (identifier) @import)
