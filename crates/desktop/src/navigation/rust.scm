[
  (block)
  (declaration_list)
  (function_item)
  (function_signature_item)
  (closure_expression)
  (for_expression)
  (while_expression)
  (if_expression)
  (match_arm)
  (impl_item)
  (trait_item)
  (struct_item)
  (enum_item)
  (union_item)
  (type_item)
] @scope

(function_item name: (identifier) @item) @definition
(function_signature_item name: (identifier) @item) @definition
(struct_item name: (type_identifier) @item) @definition
(enum_item name: (type_identifier) @item) @definition
(union_item name: (type_identifier) @item) @definition
(trait_item name: (type_identifier) @item) @definition
(type_item name: (type_identifier) @item) @definition
(associated_type name: (type_identifier) @item) @definition
(enum_variant name: (identifier) @item) @definition
(field_declaration name: (field_identifier) @item) @definition
(const_item name: (identifier) @item) @definition
(static_item name: (identifier) @item) @definition
(mod_item name: (identifier) @item) @definition
(macro_definition name: (identifier) @item) @definition

(type_parameter name: (type_identifier) @local)
(const_parameter name: (identifier) @local)
(parameter pattern: (_) @local)
(closure_parameters (_) @local)
(let_declaration pattern: (_) @local) @definition
(let_condition pattern: (_) @local) @definition
(for_expression pattern: (_) @local)
(match_arm pattern: (_) @local)

(use_declaration argument: (identifier) @import)
(use_declaration argument: (scoped_identifier name: (identifier) @import))
(use_list (identifier) @import)
(use_list (scoped_identifier name: (identifier) @import))
(use_as_clause alias: (identifier) @import)
