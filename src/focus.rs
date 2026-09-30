use crate::util;
use graphql_parser::parse_schema;
use graphql_parser::schema::TypeDefinition;
use petgraph::graph::NodeIndex;
use petgraph::visit::Walker;
use std::collections::{HashMap, HashSet};

pub fn process(schema: &str, types: &[&str]) -> String {
    let schema_ast = parse_schema::<String>(schema).expect("Invalid schema");

    let mut g: petgraph::Graph<&String, ()> = petgraph::Graph::new();
    let mut type_node_map: HashMap<&String, NodeIndex> = HashMap::new();

    // Extensions are merged in, so the fields, union members, and `implements`
    // clauses they add are walked like any other, and a type defined only by
    // an extension can be a root.
    let merged = util::merged_type_definitions(&schema_ast);

    // Every type the schema defines is a node, whether or not anything names
    // it or it names anything, so any of them can be a root. Names it only
    // references, such as built-in scalars, get a node when an edge reaches
    // them, and `retain_with_dependencies` skips them.
    for (name, type_definition) in &merged {
        let idx = *type_node_map
            .entry(name)
            .or_insert_with(|| g.add_node(name));

        match type_definition {
            TypeDefinition::Scalar(_) | TypeDefinition::Enum(_) => (),
            TypeDefinition::Object(object_type) => {
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
                for ty in union_type.types.iter() {
                    let ty_idx = type_node_map.entry(ty).or_insert_with(|| g.add_node(ty));
                    g.add_edge(idx, *ty_idx, ());
                }
            }
            TypeDefinition::InputObject(input_object_type) => {
                for field in &input_object_type.fields {
                    let tn = util::named_type(&field.value_type).unwrap();

                    let tn_idx = type_node_map.entry(tn).or_insert_with(|| g.add_node(tn));
                    g.add_edge(idx, *tn_idx, ());
                }
            }
        }
    }

    // The walk collects descendants: field types, union members, and an
    // interface's implementors. The closure then adds what those need to stand
    // on their own (argument and input types, implemented interfaces, directive
    // definitions, the trimmed `schema {}`), without treating them as roots, so
    // an interface kept only because a type implements it does not bring its
    // other implementors.
    let descendants: HashSet<&str> = types
        .iter()
        .filter_map(|t| type_node_map.get(&String::from(*t)))
        .flat_map(|root_idx| petgraph::visit::Dfs::new(&g, *root_idx).iter(&g))
        .map(|n| g[n].as_str())
        .collect();
    let descendants: Vec<&str> = descendants.into_iter().collect();

    let focused = util::retain_with_dependencies(&schema_ast, &descendants, &[], |td| td.clone());

    format!("{focused}")
}

#[cfg(test)]
mod tests {
    use crate::{focus, util};
    use indoc::indoc;
    use pretty_assertions::assert_eq;

    /// Focuses and checks that the output names only types and directives it
    /// defines.
    fn focused(schema: &str, types: &[&str]) -> String {
        let result = focus::process(schema, types);
        util::assert_self_contained(&result);
        result
    }

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

        let result = focused(schema, &["User"]);
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

        let result = focused(schema, &["User", "Company"]);
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
              admin: Boolean
            }

            type Guest implements Person {
              id: ID
              name: String
            }
        "};

        let result = focused(schema, &["Person"]);
        let expected_schema = indoc! {"
            interface Person {
              name: String
            }

            type User implements Person {
              id: ID
              name: String
              admin: Boolean
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
              admin: Boolean
            }

            type Guest implements Person {
              id: ID
              name: String
            }
        "};

        let result = focused(schema, &["Company"]);
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
              admin: Boolean
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
              admin: Boolean
            }

            type Guest implements Person {
              id: ID
              name: String
            }
        "};

        let result = focused(schema, &["Company"]);
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
              admin: Boolean
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

        // An empty document is not a parseable schema, so it cannot be checked
        // for self-containment.
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

        let result = focused(schema, &["User"]);
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

        let result = focused(schema, &["User"]);
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
    fn test_focus_keeps_argument_input_types() {
        let schema = indoc! {"
            type Query {
              search(filter: SearchFilter): [User]
            }

            input SearchFilter {
              name: String
              range: DateRange
            }

            input DateRange {
              from: DateTime
              order: SortOrder
            }

            scalar DateTime

            enum SortOrder {
              ASC
              DESC
            }

            input UnusedInput {
              field: String
            }

            type User {
              id: ID
            }
        "};

        let result = focused(schema, &["Query"]);
        let expected_schema = indoc! {"
            type Query {
              search(filter: SearchFilter): [User]
            }

            input SearchFilter {
              name: String
              range: DateRange
            }

            input DateRange {
              from: DateTime
              order: SortOrder
            }

            scalar DateTime

            enum SortOrder {
              ASC
              DESC
            }

            type User {
              id: ID
            }
        "};

        assert_eq!(result.trim(), expected_schema.trim());
    }

    #[test]
    fn test_focus_keeps_implemented_interfaces_without_their_implementors() {
        let schema = indoc! {"
            interface Node {
              id: ID!
            }

            interface Resource implements Node {
              id: ID!
              url: String
            }

            type User implements Resource & Node {
              id: ID!
              url: String
            }

            type Guest implements Node {
              id: ID!
            }
        "};

        // Node and Resource are kept because User implements them, not as
        // roots, so Guest is not dragged in.
        let result = focused(schema, &["User"]);
        let expected_schema = indoc! {"
            interface Node {
              id: ID!
            }

            interface Resource implements Node {
              id: ID!
              url: String
            }

            type User implements Resource & Node {
              id: ID!
              url: String
            }
        "};

        assert_eq!(result.trim(), expected_schema.trim());
    }

    #[test]
    fn test_focus_keeps_used_directive_definitions() {
        let schema = indoc! {"
            directive @auth(requires: Role = ADMIN) on OBJECT | FIELD_DEFINITION

            directive @cache(seconds: Int) on FIELD_DEFINITION

            enum Role {
              ADMIN
              USER
            }

            type Query {
              user: User @auth(requires: USER)
            }

            type User {
              id: ID
            }
        "};

        let result = focused(schema, &["Query"]);
        let expected_schema = indoc! {"
            directive @auth(requires: Role = ADMIN) on OBJECT | FIELD_DEFINITION

            enum Role {
              ADMIN
              USER
            }

            type Query {
              user: User @auth(requires: USER)
            }

            type User {
              id: ID
            }
        "};

        assert_eq!(result.trim(), expected_schema.trim());
    }

    #[test]
    fn test_focus_trims_schema_definition_to_surviving_roots() {
        let schema = indoc! {"
            schema {
              query: Query
              mutation: Mutation
            }

            type Query {
              user: User
            }

            type Mutation {
              rename(name: String): User
            }

            type User {
              id: ID
            }
        "};

        let result = focused(schema, &["Query"]);
        let expected_schema = indoc! {"
            schema {
              query: Query
            }

            type Query {
              user: User
            }

            type User {
              id: ID
            }
        "};
        assert_eq!(result.trim(), expected_schema.trim());

        // No root survives, so the schema definition goes too.
        let result = focused(schema, &["User"]);
        let expected_schema = indoc! {"
            type User {
              id: ID
            }
        "};
        assert_eq!(result.trim(), expected_schema.trim());
    }

    #[test]
    fn test_focus_keeps_extensions_of_kept_types_whole() {
        let schema = indoc! {"
            type User {
              id: ID
            }

            extend type User {
              avatar: Image
            }

            type Image {
              url: String
            }

            type Company {
              id: ID
            }

            extend type Company {
              logo: Image
            }
        "};

        let result = focused(schema, &["User"]);
        let expected_schema = indoc! {"
            type User {
              id: ID
            }

            extend type User {
              avatar: Image
            }

            type Image {
              url: String
            }
        "};

        assert_eq!(result.trim(), expected_schema.trim());
    }

    #[test]
    fn test_focus_walks_what_extensions_add() {
        let schema = indoc! {"
            type Company {
              id: ID
            }

            extend type Company {
              staff: [Person]
            }

            interface Person {
              name: String
            }

            type Employee implements Person {
              name: String
            }

            interface Node {
              id: ID
            }

            type Robot {
              id: ID
            }

            extend type Robot implements Node

            type Unrelated {
              id: ID
            }
        "};

        // `staff` is only in the extension, and the walk from it reaches
        // Employee, which nothing but the interface's implementor edge names.
        let result = focused(schema, &["Company"]);
        let expected_schema = indoc! {"
            type Company {
              id: ID
            }

            extend type Company {
              staff: [Person]
            }

            interface Person {
              name: String
            }

            type Employee implements Person {
              name: String
            }
        "};
        assert_eq!(result.trim(), expected_schema.trim());

        // Robot implements Node only through its extension.
        let result = focused(schema, &["Node"]);
        let expected_schema = indoc! {"
            interface Node {
              id: ID
            }

            type Robot {
              id: ID
            }

            extend type Robot implements Node
        "};
        assert_eq!(result.trim(), expected_schema.trim());
    }

    #[test]
    fn test_focus_on_an_unreferenced_scalar() {
        let schema = indoc! {"
            scalar DateTime

            scalar Unrelated

            type Query {
              id: ID
            }
        "};

        let result = focused(schema, &["DateTime"]);
        let expected_schema = indoc! {"
            scalar DateTime
        "};

        assert_eq!(result.trim(), expected_schema.trim());
    }

    #[test]
    fn test_focus_on_an_unreferenced_enum() {
        let schema = indoc! {"
            enum Color {
              RED
              GREEN
            }

            type Query {
              id: ID
            }
        "};

        let result = focused(schema, &["Color"]);
        let expected_schema = indoc! {"
            enum Color {
              RED
              GREEN
            }
        "};

        assert_eq!(result.trim(), expected_schema.trim());
    }

    #[test]
    fn test_focus_on_an_unreferenced_input() {
        let schema = indoc! {"
            input Filter {
              at: DateTime
              color: Color
            }

            scalar DateTime

            enum Color {
              RED
            }

            input Unrelated {
              id: ID
            }

            type Query {
              id: ID
            }
        "};

        let result = focused(schema, &["Filter"]);
        let expected_schema = indoc! {"
            input Filter {
              at: DateTime
              color: Color
            }

            scalar DateTime

            enum Color {
              RED
            }
        "};

        assert_eq!(result.trim(), expected_schema.trim());
    }

    #[test]
    fn test_focus_on_a_scalar_keeps_its_directive_definitions() {
        let schema = indoc! {"
            directive @format(style: FormatStyle) on SCALAR

            directive @unused on SCALAR

            enum FormatStyle {
              ISO
            }

            scalar DateTime @format(style: ISO)

            type Query {
              id: ID
            }
        "};

        // The directive's definition and its argument type are dependencies,
        // not roots, but the output needs them to be self-contained.
        let result = focused(schema, &["DateTime"]);
        let expected_schema = indoc! {"
            directive @format(style: FormatStyle) on SCALAR

            enum FormatStyle {
              ISO
            }

            scalar DateTime @format(style: ISO)
        "};

        assert_eq!(result.trim(), expected_schema.trim());
    }

    #[test]
    fn test_focus_on_a_type_defined_only_by_extensions() {
        let schema = indoc! {"
            extend type Query {
              me: User
            }

            type User {
              id: ID
            }

            type Unrelated {
              id: ID
            }
        "};

        // The base `Query` lives in another document, as in a federation
        // subgraph, so the output extends a type it does not define, exactly as
        // the input does. That is why this is not checked for self-containment.
        let result = focus::process(schema, &["Query"]);
        let expected_schema = indoc! {"
            extend type Query {
              me: User
            }

            type User {
              id: ID
            }
        "};
        assert_eq!(result.trim(), expected_schema.trim());
    }
}
