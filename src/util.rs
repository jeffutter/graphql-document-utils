use graphql_parser::query::Text;
use graphql_parser::schema::{Definition, Type, TypeDefinition, TypeExtension};

pub fn schema_definition_name<'a, V, D: Text<'a, Value = V>>(
    d: &'a Definition<'a, D>,
) -> Option<&'a V> {
    match d {
        Definition::SchemaDefinition(_) => None,
        Definition::TypeDefinition(type_definition) => schema_type_definition_name(type_definition),
        Definition::TypeExtension(_) => None,
        Definition::DirectiveDefinition(directive_definition) => Some(&directive_definition.name),
    }
}

pub fn schema_type_definition_name<'a, V, D: Text<'a, Value = V>>(
    td: &'a TypeDefinition<'a, D>,
) -> Option<&'a V> {
    match td {
        TypeDefinition::Scalar(scalar_type) => Some(&scalar_type.name),
        TypeDefinition::Object(object_type) => Some(&object_type.name),
        TypeDefinition::Interface(interface_type) => Some(&interface_type.name),
        TypeDefinition::Union(union_type) => Some(&union_type.name),
        TypeDefinition::Enum(enum_type) => Some(&enum_type.name),
        TypeDefinition::InputObject(input_object_type) => Some(&input_object_type.name),
    }
}

pub fn named_type<'a, V, D: Text<'a, Value = V>>(ty: &'a Type<'a, D>) -> Option<&'a V> {
    match ty {
        Type::NamedType(n) => Some(n),
        Type::ListType(inner) | Type::NonNullType(inner) => named_type(inner),
    }
}
