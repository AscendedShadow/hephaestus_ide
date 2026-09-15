[
  (block)
  (class_body)
  (interface_body)
  (enum_body)
  (class_declaration)
  (interface_declaration)
  (enum_declaration)
  (record_declaration)
  (method_declaration)
  (constructor_declaration)
  (lambda_expression)
  (for_statement)
  (enhanced_for_statement)
  (catch_clause)
  (try_with_resources_statement)
] @scope

(class_declaration name: (identifier) @item) @definition
(interface_declaration name: (identifier) @item) @definition
(enum_declaration name: (identifier) @item) @definition
(record_declaration name: (identifier) @item) @definition
(annotation_type_declaration name: (identifier) @item) @definition
(method_declaration name: (identifier) @item) @definition
(enum_constant name: (identifier) @item) @definition
(field_declaration declarator: (variable_declarator name: (identifier) @item)) @definition
(constant_declaration declarator: (variable_declarator name: (identifier) @item)) @definition
(record_declaration
  parameters: (formal_parameters (formal_parameter name: (identifier) @item)))

(type_parameter (type_identifier) @local)
(local_variable_declaration declarator: (variable_declarator name: (identifier) @local)) @definition
(formal_parameter name: (identifier) @local)
(spread_parameter (variable_declarator name: (identifier) @local))
(catch_formal_parameter name: (identifier) @local)
(resource name: (identifier) @local) @definition
(enhanced_for_statement name: (identifier) @local)
(inferred_parameters (identifier) @local)
(lambda_expression parameters: (identifier) @local)
(instanceof_expression name: (identifier) @local)

(import_declaration (scoped_identifier name: (identifier) @import))
