[
  (block)
  (declaration_list)
  (class_declaration)
  (struct_declaration)
  (interface_declaration)
  (record_declaration)
  (delegate_declaration)
  (method_declaration)
  (constructor_declaration)
  (local_function_statement)
  (lambda_expression)
  (for_statement)
  (foreach_statement)
  (catch_clause)
  (using_statement)
  (switch_section)
] @scope

(class_declaration name: (identifier) @item) @definition
(struct_declaration name: (identifier) @item) @definition
(interface_declaration name: (identifier) @item) @definition
(record_declaration name: (identifier) @item) @definition
(enum_declaration name: (identifier) @item) @definition
(delegate_declaration name: (identifier) @item) @definition
(method_declaration name: (identifier) @item) @definition
(local_function_statement name: (identifier) @item) @definition
(property_declaration name: (identifier) @item) @definition
(enum_member_declaration name: (identifier) @item) @definition
(field_declaration (variable_declaration (variable_declarator name: (identifier) @item))) @definition
(event_field_declaration (variable_declaration (variable_declarator name: (identifier) @item))) @definition
(record_declaration (parameter_list (parameter name: (identifier) @item)))

(type_parameter name: (identifier) @local)
(local_declaration_statement (variable_declaration (variable_declarator name: (identifier) @local))) @definition
(for_statement initializer: (variable_declaration (variable_declarator name: (identifier) @local)))
(using_statement (variable_declaration (variable_declarator name: (identifier) @local)))
(parameter name: (identifier) @local)
(parameter_list name: (identifier) @local)
(lambda_expression parameters: (implicit_parameter) @local)
(foreach_statement left: (_) @local)
(catch_declaration name: (identifier) @local)
(declaration_pattern name: (identifier) @local)

(using_directive name: (identifier) @import)
