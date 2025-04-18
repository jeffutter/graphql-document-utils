use crate::util;
use graphql_parser::{
    query::{
        parse_query, Definition as QueryDef, FragmentDefinition, OperationDefinition, Selection,
        SelectionSet, TypeCondition,
    },
    schema::{
        parse_schema, Definition as SchemaDef, Document as SchemaDoc, Field, InputValue,
        TypeDefinition,
    },
};
use std::collections::{HashMap, HashSet};

/// Processes the schema and query files to prune unused types and fields.
pub fn process(schema: &str, query: &str) -> String {
    let schema_doc = parse_schema::<String>(schema).expect("Failed to parse schema");
    let query_doc = parse_query::<String>(query).expect("Failed to parse query");

    let schema_doc_copy = schema_doc.clone();

    let type_map: HashMap<_, _> = schema_doc_copy
        .definitions
        .iter()
        .filter_map(|def| {
            if let SchemaDef::TypeDefinition(td) = def {
                Some((
                    util::schema_type_definition_name(td).unwrap().to_string(),
                    td,
                ))
            } else {
                None
            }
        })
        .collect();

    let fragments: HashMap<_, _> = query_doc
        .definitions
        .iter()
        .filter_map(|def| {
            if let QueryDef::Fragment(f) = def {
                Some((f.name.clone(), f))
            } else {
                None
            }
        })
        .collect();

    let root_types = detect_root_types(&schema_doc);

    let mut used_fields: HashMap<String, HashSet<String>> = HashMap::new();

    for def in &query_doc.definitions {
        if let QueryDef::Operation(op) = def {
            let (op_type, selection_set) = match op {
                OperationDefinition::Query(q) => (root_types.query.as_str(), &q.selection_set),
                OperationDefinition::Mutation(m) => (
                    root_types.mutation.as_deref().unwrap_or("Mutation"),
                    &m.selection_set,
                ),
                OperationDefinition::Subscription(s) => (
                    root_types.subscription.as_deref().unwrap_or("Subscription"),
                    &s.selection_set,
                ),
                OperationDefinition::SelectionSet(ss) => (root_types.query.as_str(), ss),
            };
            used_fields.insert(op_type.to_string(), HashSet::new());
            collect_used_fields(
                op_type,
                selection_set,
                &type_map,
                &mut used_fields,
                &fragments,
            );
        }
    }

    let pruned_defs: Vec<_> = schema_doc
        .definitions
        .iter()
        .filter_map(|def| match def {
            SchemaDef::TypeDefinition(ref td) => match td {
                TypeDefinition::Object(obj) => {
                    let include_object = used_fields.contains_key(&obj.name)
                        || obj
                            .implements_interfaces
                            .iter()
                            .any(|i| used_fields.contains_key(i));

                    if include_object {
                        let kept_fields = obj
                            .fields
                            .clone()
                            .into_iter()
                            .filter(|f| {
                                used_fields
                                    .get(&obj.name)
                                    .is_some_and(|set| set.contains(&f.name))
                                    || obj.implements_interfaces.iter().any(|i| {
                                        used_fields.get(i).is_some_and(|set| set.contains(&f.name))
                                    })
                            })
                            .collect();

                        return Some(SchemaDef::TypeDefinition(TypeDefinition::Object(
                            graphql_parser::schema::ObjectType {
                                fields: kept_fields,
                                ..obj.clone()
                            },
                        )));
                    }

                    None
                }
                TypeDefinition::Interface(iface) if used_fields.contains_key(&iface.name) => {
                    let kept_fields = iface
                        .fields
                        .clone()
                        .into_iter()
                        .filter(|f| {
                            used_fields
                                .get(&iface.name)
                                .is_some_and(|set| set.contains(&f.name))
                        })
                        .collect();

                    Some(SchemaDef::TypeDefinition(TypeDefinition::Interface(
                        graphql_parser::schema::InterfaceType {
                            fields: kept_fields,
                            ..iface.clone()
                        },
                    )))
                }
                _ if used_fields.contains_key(util::schema_type_definition_name(td).unwrap()) => {
                    Some(SchemaDef::TypeDefinition(td.clone()))
                }
                _ => None,
            },
            SchemaDef::SchemaDefinition(_)
            | SchemaDef::DirectiveDefinition(_)
            | SchemaDef::TypeExtension(_) => Some(def.clone()),
        })
        .collect();

    let pruned_doc = SchemaDoc {
        definitions: pruned_defs,
    };

    format!("{}", pruned_doc)
}

/// Collects used fields from the selection set.
fn collect_used_fields<'a>(
    parent_type: &str,
    selection_set: &SelectionSet<String>,
    type_map: &HashMap<String, &'a TypeDefinition<'a, String>>,
    used_fields: &mut HashMap<String, HashSet<String>>,
    fragments: &HashMap<String, &'a FragmentDefinition<'a, String>>,
) {
    if let Some(parent_def) = type_map.get(parent_type) {
        let fields = type_fields(parent_def);

        for selection in &selection_set.items {
            match selection {
                Selection::Field(field) => {
                    if let Some(schema_field) =
                        fields.and_then(|fields| fields.iter().find(|f| f.name == field.name))
                    {
                        let used_types = used_fields.entry(parent_type.to_string()).or_default();
                        used_types.insert(field.name.clone());

                        let nested_type = util::named_type(&schema_field.field_type).unwrap();
                        // if used_fields
                        //     .insert(nested_type.clone(), HashSet::new())
                        //     .is_none()
                        // {
                        //     // track for input arg traversal later
                        // }

                        for arg in &schema_field.arguments {
                            collect_input_types(arg, used_types, type_map);
                        }

                        collect_used_fields(
                            nested_type,
                            &field.selection_set,
                            type_map,
                            used_fields,
                            fragments,
                        );
                    }
                }
                Selection::FragmentSpread(spread) => {
                    if let Some(frag) = fragments.get(&spread.fragment_name) {
                        let TypeCondition::On(type_condition) = &frag.type_condition;
                        used_fields.insert(type_condition.clone(), HashSet::new());
                        collect_used_fields(
                            type_condition,
                            &frag.selection_set,
                            type_map,
                            used_fields,
                            fragments,
                        );
                    }
                }
                Selection::InlineFragment(frag) => {
                    let type_name = frag
                        .type_condition
                        .clone()
                        .map(|tc| match tc {
                            TypeCondition::On(name) => name,
                        })
                        .unwrap_or(parent_type.to_string());

                    used_fields.insert(type_name.to_string(), HashSet::new());
                    collect_used_fields(
                        &type_name,
                        &frag.selection_set,
                        type_map,
                        used_fields,
                        fragments,
                    );
                }
            }
        }
    }
}

/// Collects input types from the argument.
fn collect_input_types<'a>(
    arg: &'a InputValue<'a, String>,
    used_types: &mut HashSet<String>,
    type_map: &HashMap<String, &'a TypeDefinition<'a, String>>,
) {
    let inner = util::named_type(&arg.value_type).unwrap();
    if used_types.insert(inner.clone()) {
        if let Some(TypeDefinition::InputObject(input_obj)) = type_map.get(inner) {
            for field in &input_obj.fields {
                collect_input_types(field, used_types, type_map);
            }
        }
    }
}

/// Retrieves fields for an object or interface type.
fn type_fields<'a>(typ: &'a TypeDefinition<'a, String>) -> Option<&'a Vec<Field<'a, String>>> {
    match typ {
        TypeDefinition::Object(obj) => Some(&obj.fields),
        TypeDefinition::Interface(iface) => Some(&iface.fields),
        _ => None,
    }
}

/// Detects root types (Query, Mutation, Subscription) from the schema.
fn detect_root_types(schema: &SchemaDoc<String>) -> RootTypes {
    let mut root = RootTypes {
        query: "Query".to_string(),
        mutation: None,
        subscription: None,
    };

    for def in &schema.definitions {
        if let SchemaDef::SchemaDefinition(schema_def) = def {
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

struct RootTypes {
    query: String,
    mutation: Option<String>,
    subscription: Option<String>,
}

#[cfg(test)]
mod tests {
    use crate::prune;
    use indoc::indoc;
    use pretty_assertions::assert_eq;

    #[test]
    fn prunes_fields() {
        let schema = indoc! {"
            type Query {
              user: User
            }

            type User {
              id: ID!
              name: String
              first_name: String
              last_name: String
            }
        "};

        let query = indoc! {"
            query User {
              user {
                id
                name
              }
            }
        "};

        let result = prune::process(schema, query);

        assert_eq!(
            result,
            indoc! {"
                type Query {
                  user: User
                }

                type User {
                  id: ID!
                  name: String
                }
            "}
        );
    }

    #[test]
    fn prunes_interface_fields() {
        let schema = indoc! {"
            type Query {
              person: Person
            }

            interface Person {
              id: ID!
              name: String
              first_name: String
              last_name: String
            }

            type User implements Person {
              id: ID!
              name: String
              first_name: String
              last_name: String
              user_name: String
              login: String
            }

            type Customer implements Person {
              id: ID!
              name: String
              first_name: String
              last_name: String
              last_visited: String
            }

            type Guest implements Person {
              id: ID!
              name: String
              first_name: String
              last_name: String
            }
        "};

        let query = indoc! {"
            query Person {
              person {
                id
                name
                ... on User {
                  user_name
                }
              }
            }
        "};

        let result = prune::process(schema, query);

        assert_eq!(
            result,
            indoc! {"
                type Query {
                  person: Person
                }

                interface Person {
                  id: ID!
                  name: String
                }

                type User implements Person {
                  id: ID!
                  name: String
                  user_name: String
                }

                type Customer implements Person {
                  id: ID!
                  name: String
                }

                type Guest implements Person {
                  id: ID!
                  name: String
                }
            "}
        );
    }
}
