use graphql_parser::query::{
    Directive, Mutation, OperationDefinition, Query, SelectionSet, Subscription, Text,
    TypeCondition, Value, VariableDefinition,
};
use graphql_parser::schema::{Definition, Document, Field, Type, TypeDefinition};
use std::collections::HashSet;

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

/// Retrieves the fields of an object or interface type. Other type definitions
/// have no selectable fields.
pub fn type_fields<'a, D: Text<'a>>(
    td: &'a TypeDefinition<'a, D>,
) -> Option<&'a Vec<Field<'a, D>>> {
    match td {
        TypeDefinition::Object(obj) => Some(&obj.fields),
        TypeDefinition::Interface(iface) => Some(&iface.fields),
        _ => None,
    }
}

/// The type names backing each operation root, as named by the schema
/// definition. `query` defaults to `Query` when the schema leaves it implicit;
/// `mutation` and `subscription` are absent unless the schema declares them.
pub struct RootTypes {
    pub query: String,
    pub mutation: Option<String>,
    pub subscription: Option<String>,
}

/// Detects root types (Query, Mutation, Subscription) from the schema.
pub fn detect_root_types(schema: &Document<String>) -> RootTypes {
    let mut root = RootTypes {
        query: "Query".to_string(),
        mutation: None,
        subscription: None,
    };

    for def in &schema.definitions {
        if let Definition::SchemaDefinition(schema_def) = def {
            if let Some(query) = &schema_def.query {
                root.query = query.clone();
            }
            if let Some(mutation) = &schema_def.mutation {
                root.mutation = Some(mutation.clone());
            }
            if let Some(subscription) = &schema_def.subscription {
                root.subscription = Some(subscription.clone());
            }
        }
    }

    root
}

/// The root type name and selection set an operation starts from. Anonymous
/// operations and bare selection sets root at the query type.
pub fn operation_root<'a, 'q>(
    root_types: &'a RootTypes,
    op: &'a OperationDefinition<'q, String>,
) -> (&'a str, &'a SelectionSet<'q, String>) {
    match op {
        OperationDefinition::SelectionSet(set) => (&root_types.query, set),
        OperationDefinition::Query(query) => (&root_types.query, &query.selection_set),
        OperationDefinition::Mutation(mutation) => (
            root_types.mutation.as_deref().unwrap_or("Mutation"),
            &mutation.selection_set,
        ),
        OperationDefinition::Subscription(subscription) => (
            root_types.subscription.as_deref().unwrap_or("Subscription"),
            &subscription.selection_set,
        ),
    }
}

pub fn operation_directives<'a, 'q>(
    op: &'a OperationDefinition<'q, String>,
) -> &'a [Directive<'q, String>] {
    match op {
        OperationDefinition::SelectionSet(_) => &[],
        OperationDefinition::Query(query) => &query.directives,
        OperationDefinition::Mutation(mutation) => &mutation.directives,
        OperationDefinition::Subscription(subscription) => &subscription.directives,
    }
}

/// Rebuilds an operation around a rewritten selection set, keeping only the
/// variable definitions named in `used_variables`. A bare selection set has
/// nowhere to put directives, so `directives` is ignored for that form.
pub fn rebuild_operation<'q>(
    op: &OperationDefinition<'q, String>,
    selection_set: SelectionSet<'q, String>,
    directives: Vec<Directive<'q, String>>,
    used_variables: &HashSet<String>,
) -> OperationDefinition<'q, String> {
    match op {
        OperationDefinition::SelectionSet(_) => OperationDefinition::SelectionSet(selection_set),
        OperationDefinition::Query(query) => OperationDefinition::Query(Query {
            variable_definitions: retain_variables(&query.variable_definitions, used_variables),
            directives,
            selection_set,
            ..query.clone()
        }),
        OperationDefinition::Mutation(mutation) => OperationDefinition::Mutation(Mutation {
            variable_definitions: retain_variables(&mutation.variable_definitions, used_variables),
            directives,
            selection_set,
            ..mutation.clone()
        }),
        OperationDefinition::Subscription(subscription) => {
            OperationDefinition::Subscription(Subscription {
                variable_definitions: retain_variables(
                    &subscription.variable_definitions,
                    used_variables,
                ),
                directives,
                selection_set,
                ..subscription.clone()
            })
        }
    }
}

/// The variable definitions an operation declares, in source order.
pub fn operation_variable_definitions<'a, 'q>(
    op: &'a OperationDefinition<'q, String>,
) -> &'a [VariableDefinition<'q, String>] {
    match op {
        OperationDefinition::SelectionSet(_) => &[],
        OperationDefinition::Query(query) => &query.variable_definitions,
        OperationDefinition::Mutation(mutation) => &mutation.variable_definitions,
        OperationDefinition::Subscription(subscription) => &subscription.variable_definitions,
    }
}

fn retain_variables<'q>(
    variable_definitions: &[VariableDefinition<'q, String>],
    used: &HashSet<String>,
) -> Vec<VariableDefinition<'q, String>> {
    variable_definitions
        .iter()
        .filter(|def| used.contains(&def.name))
        .cloned()
        .collect()
}

pub fn type_condition<'a>(condition: Option<&'a TypeCondition<'_, String>>) -> Option<&'a str> {
    condition.map(|TypeCondition::On(name)| name.as_str())
}

pub fn collect_directive_variables(
    directives: &[Directive<'_, String>],
    used: &mut HashSet<String>,
) {
    for directive in directives {
        for (_name, value) in &directive.arguments {
            collect_value_variables(value, used);
        }
    }
}

pub fn collect_value_variables(value: &Value<'_, String>, used: &mut HashSet<String>) {
    match value {
        Value::Variable(name) => {
            used.insert(name.clone());
        }
        Value::List(values) => {
            for value in values {
                collect_value_variables(value, used);
            }
        }
        Value::Object(fields) => {
            for value in fields.values() {
                collect_value_variables(value, used);
            }
        }
        Value::Int(_)
        | Value::Float(_)
        | Value::String(_)
        | Value::Boolean(_)
        | Value::Null
        | Value::Enum(_) => (),
    }
}
