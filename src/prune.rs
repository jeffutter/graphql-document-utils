use crate::util;
use graphql_parser::{
    query::{
        parse_query, Definition as QueryDef, FragmentDefinition, OperationDefinition, Selection,
        SelectionSet, TypeCondition,
    },
    schema::{
        parse_schema, Definition as SchemaDef, Document as SchemaDoc, Field, InputValue, Type,
        TypeDefinition,
    },
};
use std::path::PathBuf;
use std::{
    collections::{HashMap, HashSet},
    fs,
};

pub fn process(schema: PathBuf, query: PathBuf) {
    let schema_str = fs::read_to_string(schema).expect("Failed to read schema file");
    let query_str = fs::read_to_string(query).expect("Failed to read query file");

    let schema_doc = parse_schema::<String>(&schema_str).expect("Failed to parse schema");
    let query_doc = parse_query::<String>(&query_str).expect("Failed to parse query");

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
    let mut used_types: HashSet<String> = HashSet::new();

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
            used_types.insert(op_type.to_string());
            collect_used_fields(
                op_type,
                selection_set,
                &type_map,
                &mut used_fields,
                &mut used_types,
                &fragments,
            );
        }
    }

    let pruned_defs: Vec<_> = schema_doc
        .definitions
        .iter()
        .filter_map(|def| match def {
            SchemaDef::TypeDefinition(ref td) => match td {
                TypeDefinition::Object(obj) if used_types.contains(&obj.name) => {
                    let kept_fields = obj
                        .fields
                        .clone()
                        .into_iter()
                        .filter(|f| {
                            used_fields
                                .get(&obj.name)
                                .is_some_and(|set| set.contains(&f.name))
                        })
                        .collect();

                    Some(SchemaDef::TypeDefinition(TypeDefinition::Object(
                        graphql_parser::schema::ObjectType {
                            fields: kept_fields,
                            ..obj.clone()
                        },
                    )))
                }
                TypeDefinition::Interface(iface) if used_types.contains(&iface.name) => {
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
                _ if used_types.contains(util::schema_type_definition_name(td).unwrap()) => {
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

    println!("{}", pruned_doc);
}

fn collect_used_fields<'a>(
    parent_type: &str,
    selection_set: &SelectionSet<String>,
    type_map: &HashMap<String, &'a TypeDefinition<'a, String>>,
    used_fields: &mut HashMap<String, HashSet<String>>,
    used_types: &mut HashSet<String>,
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
                        used_fields
                            .entry(parent_type.to_string())
                            .or_default()
                            .insert(field.name.clone());

                        let nested_type = get_named_type(&schema_field.field_type);
                        if used_types.insert(nested_type.clone()) {
                            // track for input arg traversal later
                        }

                        for arg in &schema_field.arguments {
                            collect_input_types(arg, used_types, type_map);
                        }

                        collect_used_fields(
                            &nested_type,
                            &field.selection_set,
                            type_map,
                            used_fields,
                            used_types,
                            fragments,
                        );
                    }
                }
                Selection::FragmentSpread(spread) => {
                    if let Some(frag) = fragments.get(&spread.fragment_name) {
                        let TypeCondition::On(type_condition) = &frag.type_condition;
                        used_types.insert(type_condition.clone());
                        collect_used_fields(
                            type_condition,
                            &frag.selection_set,
                            type_map,
                            used_fields,
                            used_types,
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

                    used_types.insert(type_name.to_string());
                    collect_used_fields(
                        &type_name,
                        &frag.selection_set,
                        type_map,
                        used_fields,
                        used_types,
                        fragments,
                    );
                }
            }
        }
    }
}

fn collect_input_types<'a>(
    arg: &'a InputValue<'a, String>,
    used_types: &mut HashSet<String>,
    type_map: &HashMap<String, &'a TypeDefinition<'a, String>>,
) {
    let inner = get_named_type(&arg.value_type);
    if used_types.insert(inner.clone()) {
        if let Some(TypeDefinition::InputObject(input_obj)) = type_map.get(&inner) {
            for field in &input_obj.fields {
                collect_input_types(field, used_types, type_map);
            }
        }
    }
}

fn get_named_type(t: &Type<String>) -> String {
    match t {
        Type::NamedType(name) => name.clone(),
        Type::ListType(inner) | Type::NonNullType(inner) => get_named_type(inner),
    }
}

fn type_fields<'a>(typ: &'a TypeDefinition<'a, String>) -> Option<&'a Vec<Field<'a, String>>> {
    match typ {
        TypeDefinition::Object(obj) => Some(&obj.fields),
        TypeDefinition::Interface(iface) => Some(&iface.fields),
        _ => None,
    }
}

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
