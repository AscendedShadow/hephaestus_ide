[
  (block)
  (function_declaration)
  (struct_declaration)
  (enum_declaration)
  (union_declaration)
  (opaque_declaration)
  (test_declaration)
  (for_statement)
  (while_statement)
  (if_statement)
  (else_clause)
  (switch_case)
] @scope

(function_declaration name: (identifier) @item) @definition
(source_file (variable_declaration ["const" "var"] . (identifier) @item) @definition)
(struct_declaration (variable_declaration ["const" "var"] . (identifier) @item) @definition)
(enum_declaration (variable_declaration ["const" "var"] . (identifier) @item) @definition)
(union_declaration (variable_declaration ["const" "var"] . (identifier) @item) @definition)
(opaque_declaration (variable_declaration ["const" "var"] . (identifier) @item) @definition)
(container_field name: (identifier) @item) @definition

(block (variable_declaration ["const" "var"] . (identifier) @local) @definition)
(parameter name: (identifier) @local)
(payload (identifier) @local)
