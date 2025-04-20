use crate::util;
use graphql_parser::parse_schema;
use graphql_parser::schema::{Definition, Document, TypeDefinition};
use petgraph::graph::NodeIndex;
use petgraph::visit::Walker;
use std::collections::{HashMap, HashSet};

pub fn process(schema: &str, types: &[&str]) -> String {
    let schema_ast = parse_schema::<String>(schema).expect("Invalid schema");

    let mut g: petgraph::Graph<&String, ()> = petgraph::Graph::new();
    let mut type_node_map: HashMap<&String, NodeIndex> = HashMap::new();

    for definition in schema_ast.definitions.iter() {
        match definition {
            Definition::SchemaDefinition(_schema_definition) => (),
            Definition::TypeDefinition(type_definition) => match type_definition {
                TypeDefinition::Scalar(_scalar_type) => (),
                TypeDefinition::Object(object_type) => {
                    let idx = *type_node_map
                        .entry(&object_type.name)
                        .or_insert_with(|| g.add_node(&object_type.name));

                    for field in &object_type.fields {
                        let tn = util::named_type(&field.field_type).unwrap();

                        let tn_idx = type_node_map.entry(tn).or_insert_with(|| g.add_node(tn));

                        g.add_edge(idx, *tn_idx, ());
                    }

                    for i in &object_type.implements_interfaces {
                        let i_idx = type_node_map.entry(i).or_insert_with(|| g.add_node(i));

                        g.add_edge(*i_idx, idx, ());
                    }
                }
                TypeDefinition::Interface(interface_type) => {
                    let idx = *type_node_map
                        .entry(&interface_type.name)
                        .or_insert_with(|| g.add_node(&interface_type.name));

                    for field in &interface_type.fields {
                        let tn = util::named_type(&field.field_type).unwrap();

                        let tn_idx = type_node_map.entry(tn).or_insert_with(|| g.add_node(tn));

                        g.add_edge(idx, *tn_idx, ());
                    }

                    for i in &interface_type.implements_interfaces {
                        let i_idx = type_node_map.entry(i).or_insert_with(|| g.add_node(i));

                        g.add_edge(*i_idx, idx, ());
                    }
                }
                TypeDefinition::Union(union_type) => {
                    let idx = *type_node_map
                        .entry(&union_type.name)
                        .or_insert_with(|| g.add_node(&union_type.name));

                    for ty in union_type.types.iter() {
                        let ty_idx = type_node_map.entry(ty).or_insert_with(|| g.add_node(ty));
                        g.add_edge(idx, *ty_idx, ());
                    }
                }
                TypeDefinition::Enum(enum_type) => {
                    type_node_map
                        .entry(&enum_type.name)
                        .or_insert_with(|| g.add_node(&enum_type.name));
                }
                TypeDefinition::InputObject(input_object_type) => {
                    let idx = *type_node_map
                        .entry(&input_object_type.name)
                        .or_insert_with(|| g.add_node(&input_object_type.name));

                    for field in &input_object_type.fields {
                        let tn = util::named_type(&field.value_type).unwrap();

                        let tn_idx = type_node_map.entry(tn).or_insert_with(|| g.add_node(tn));
                        g.add_edge(idx, *tn_idx, ());
                    }
                }
            },
            Definition::TypeExtension(_type_extension) => (),
            Definition::DirectiveDefinition(_directive_definition) => (),
        }
    }

    let used: HashSet<&String> = types
        .iter()
        .flat_map(|t| {
            if let Some(root_idx) = type_node_map.get(&String::from(*t)) {
                let dfs = petgraph::visit::Dfs::new(&g, *root_idx);
                return dfs.iter(&g).map(|n| g[n]).collect();
            }
            Vec::new()
        })
        .collect();

    if used.is_empty() {
        return String::from("");
    }

    strip_unused_types(&schema_ast, used)
}

/// Removes unused types from the GraphQL schema.
/// It filters out definitions that are not in the set of used types and returns the modified schema as a string.
fn strip_unused_types<'a>(
    schema: &'a Document<'a, String>,
    used_types: HashSet<&String>,
) -> String {
    let retained: Vec<_> = schema
        .definitions
        .iter()
        .filter(|def| {
            util::schema_definition_name(def)
                .map_or_else(|| false, |name| used_types.contains(name))
        })
        .collect();

    let result_doc = Document {
        definitions: retained.into_iter().cloned().collect(),
    };

    format!("{}", result_doc)
}

#[cfg(test)]
mod tests {
    use crate::focus;
    use indoc::indoc;
    use pretty_assertions::assert_eq;

    #[test]
    fn test_focus_query_operation() {
        let schema = indoc! {"
            type Query {
              user: User
            }

            type User {
              id: ID
              name: String
            }
        "};

        let result = focus::process(schema, &["User"]);
        let expected_schema = indoc! {"
            type User {
              id: ID
              name: String
            }
        "};

        assert_eq!(result.trim(), expected_schema.trim());
    }

    #[test]
    fn test_focus_multiple_types() {
        let schema = indoc! {"
            type Query {
              user: User
              company: Company
            }

            type User {
              id: ID
              name: String
            }

            type Company {
              id: ID
              name: String
            }
        "};

        let result = focus::process(schema, &["User", "Company"]);
        let expected_schema = indoc! {"
            type User {
              id: ID
              name: String
            }

            type Company {
              id: ID
              name: String
            }
        "};

        assert_eq!(result.trim(), expected_schema.trim());
    }

    #[test]
    fn test_focus_interface() {
        let schema = indoc! {"
            type Query {
              user: User
            }

            interface Person {
              name: String
            }

            type User implements Person {
              id: ID
              name: String
              admin: Bool
            }

            type Guest implements Person {
              id: ID
              name: String
            }
        "};

        let result = focus::process(schema, &["Person"]);
        let expected_schema = indoc! {"
            interface Person {
              name: String
            }

            type User implements Person {
              id: ID
              name: String
              admin: Bool
            }

            type Guest implements Person {
              id: ID
              name: String
            }
        "};

        assert_eq!(result.trim(), expected_schema.trim());
    }

    #[test]
    fn test_nested_interface() {
        let schema = indoc! {"
            type Query {
                company: Company
            }

            type Company {
              employees: [Person]
            }

            interface Person {
              name: String
            }

            type User implements Person {
              id: ID
              name: String
              admin: Bool
            }

            type Guest implements Person {
              id: ID
              name: String
            }
        "};

        let result = focus::process(schema, &["Company"]);
        let expected_schema = indoc! {"
            type Company {
              employees: [Person]
            }

            interface Person {
              name: String
            }

            type User implements Person {
              id: ID
              name: String
              admin: Bool
            }

            type Guest implements Person {
              id: ID
              name: String
            }
        "};

        assert_eq!(result.trim(), expected_schema.trim());
    }

    #[test]
    fn test_non_null_nested_interface() {
        let schema = indoc! {"
            type Query {
                company: Company
            }

            type Company {
              employees: Person!
            }

            interface Person {
              name: String
            }

            type User implements Person {
              id: ID
              name: String
              admin: Bool
            }

            type Guest implements Person {
              id: ID
              name: String
            }
        "};

        let result = focus::process(schema, &["Company"]);
        let expected_schema = indoc! {"
            type Company {
              employees: Person!
            }

            interface Person {
              name: String
            }

            type User implements Person {
              id: ID
              name: String
              admin: Bool
            }

            type Guest implements Person {
              id: ID
              name: String
            }
        "};

        assert_eq!(result.trim(), expected_schema.trim());
    }

    #[test]
    fn test_focus_query_missing_operation() {
        let schema = indoc! {"
            type Query {
              user: User
            }

            type User {
              id: ID
              name: String
            }
        "};

        let result = focus::process(schema, &["nonExistent"]);
        assert_eq!(result.trim(), "");
    }

    #[test]
    fn test_focus_nested_types() {
        let schema = indoc! {"
            type Query {
              user: User
            }

            type User {
              id: ID
              profile: Profile
            }

            type Profile {
              email: String
            }
        "};

        let result = focus::process(schema, &["User"]);
        let expected_schema = indoc! {"
            type User {
              id: ID
              profile: Profile
            }

            type Profile {
              email: String
            }
        "};

        assert_eq!(result.trim(), expected_schema.trim());
    }

    #[test]
    fn test_focus_unused_types() {
        let schema = indoc! {"
            type Query {
              user: User
            }

            type User {
              id: ID
              profile: Profile
            }

            type Profile {
              email: String
            }

            type UnusedType {
              field: String
            }
        "};

        let result = focus::process(schema, &["User"]);
        let expected_schema = indoc! {"
            type User {
              id: ID
              profile: Profile
            }

            type Profile {
              email: String
            }
        "};

        assert_eq!(result.trim(), expected_schema.trim());
    }
}
