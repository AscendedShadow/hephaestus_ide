(interface_declaration name: (type_identifier) @title.interface)
(type_alias_declaration name: (type_identifier) @title.type)
(enum_declaration name: (identifier) @title.enum)
(internal_module name: (identifier) @title.namespace)

[(type_identifier) (predefined_type)] @type

[
  "abstract" "declare" "enum" "implements" "interface" "keyof" "namespace"
  "private" "protected" "public" "type" "readonly" "override" "satisfies"
] @keyword
