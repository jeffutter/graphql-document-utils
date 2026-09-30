use crate::{
    error::Error,
    input::{Input, Kind},
    query_target, util,
};
use graphql_parser::schema::TypeDefinition;
use petgraph::graph::NodeIndex;
use petgraph::visit::Walker;
use std::collections::{HashMap, HashSet};

/// Reduces the schema `read_schema` returns to `types` and their descendants.
///
/// Roots are checked in two passes. The first needs no schema, so it runs
/// before `read_schema`, which may block on stdin: a schema file passed
/// positionally, the likeliest mistake, fails at once instead of waiting on a
/// terminal or an idle pipe. Whether a root names a type needs the schema, so
/// that check comes after the read, and is skipped for a blank schema, which
/// yields an empty document.
///
/// A name the schema defines more than once adds a warning to `warnings`, and
/// only its first definition is kept (see `Input::parse_schema`).
pub fn process(
    types: &[&str],
    read_schema: impl FnOnce() -> Result<Input, Error>,
    warnings: &mut Vec<String>,
) -> Result<String, Error> {
    // A path is never a type name, so it is caught first, before its `.`
    // reads as `Type.field`.
    for &name in types {
        if query_target::looks_like_path(name) {
            return Err(Error::PathAsTarget {
                target: name.to_string(),
                kind: Kind::Schema,
            });
        }
        if name.contains('.') {
            return Err(Error::FieldInSchemaFocus {
                target: name.to_string(),
            });
        }
    }

    // A blank schema passes through as empty, as it does through every schema
    // command. There is then nothing to look a root up in, so only its form,
    // checked above, is checked at all.
    let schema = read_schema()?;
    let Some(schema_ast) = schema.parse_schema(warnings)? else {
        return Ok(String::new());
    };

    let mut g: petgraph::Graph<&String, ()> = petgraph::Graph::new();
    let mut type_node_map: HashMap<&String, NodeIndex> = HashMap::new();

    // Extensions are merged in, so the fields, union members, and `implements`
    // clauses they add are walked like any other, and a type defined only by
    // an extension can be a root.
    let merged = util::merged_type_definitions(&schema_ast);

    // Every root must be a type the schema defines, so a typo fails instead of
    // quietly focusing on nothing. A built-in scalar is a type, but one with no
    // definition here to keep.
    for &name in types {
        if merged.contains_key(name) {
            continue;
        }
        if util::BUILT_IN_SCALARS.contains(&name) {
            return Err(Error::BuiltInScalar {
                name: name.to_string(),
                origin: schema.origin().clone(),
            });
        }
        return Err(query_target::unknown_type(
            name,
            schema.origin(),
            merged.keys().map(String::as_str),
        ));
    }

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

    Ok(format!("{focused}"))
}

#[cfg(test)]
mod tests {
    use crate::{focus, input::Input, util};
    use indoc::indoc;
    use pretty_assertions::assert_eq;

    /// Focuses and checks that the output names only types and directives it
    /// defines.
    fn focused(schema: &str, types: &[&str]) -> String {
        let result = focus::process(types, || Ok(Input::inline(schema)), &mut Vec::new()).unwrap();
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

    fn focus_error(schema: &str, types: &[&str]) -> String {
        focus::process(types, || Ok(Input::inline(schema)), &mut Vec::new())
            .expect_err("expected the focus to fail")
            .to_string()
    }

    const USER_SCHEMA: &str = indoc! {"
        type Query {
          user: User
        }

        type User {
          id: ID
          name: String
        }
    "};

    #[test]
    fn rejects_an_unknown_type_with_a_suggestion() {
        assert_eq!(
            focus_error(USER_SCHEMA, &["Usr"]),
            "unknown type `Usr` in (stdin); did you mean `User`?"
        );
        assert_eq!(
            focus_error(USER_SCHEMA, &["nonExistent"]),
            "unknown type `nonExistent` in (stdin)"
        );
    }

    #[test]
    fn suggests_the_right_case() {
        assert_eq!(
            focus_error(USER_SCHEMA, &["User", "user"]),
            "unknown type `user` in (stdin); did you mean `User`?"
        );
    }

    #[test]
    fn rejects_field_targets() {
        assert_eq!(
            focus_error(USER_SCHEMA, &["Query.user"]),
            "`schema focus` takes type names; for fields use `query focus` (got `Query.user`)"
        );
    }

    /// A second schema file passed positionally would otherwise read as
    /// `Type.field` and be told to use `query focus`.
    #[test]
    fn rejects_a_path_with_a_hint_to_pass_it_with_its_flag() {
        for target in ["other.graphql", "User.gql", "schemas/user"] {
            assert_eq!(
                focus_error(USER_SCHEMA, &["User", target]),
                format!("unknown target '{target}'; pass the schema file with -s: -s {target}")
            );
        }
    }

    /// Checking a root's form needs no schema, so it happens before the
    /// schema is read, since reading it may block on stdin.
    #[test]
    fn rejects_malformed_roots_without_reading_the_schema() {
        for (types, expected) in [
            (
                &["User", "schema.graphql"][..],
                "unknown target 'schema.graphql'; pass the schema file with -s: -s schema.graphql",
            ),
            (
                &["Query.user"][..],
                "`schema focus` takes type names; for fields use `query focus` (got `Query.user`)",
            ),
        ] {
            let error = focus::process(
                types,
                || panic!("read the schema before checking {types:?}"),
                &mut Vec::new(),
            )
            .expect_err("expected the focus to fail");
            assert_eq!(error.to_string(), expected);
        }
    }

    /// A blank schema passes through as empty. With nothing to look a root up
    /// in, a root is only checked for its form.
    #[test]
    fn passes_a_blank_schema_through() {
        for schema in ["", "\n", "# no definitions\n"] {
            let result =
                focus::process(&["Usr"], || Ok(Input::inline(schema)), &mut Vec::new()).unwrap();
            assert_eq!(result, "", "{schema:?}");
            assert_eq!(
                focus_error(schema, &["Query.user"]),
                "`schema focus` takes type names; for fields use `query focus` (got `Query.user`)"
            );
        }
    }

    #[test]
    fn rejects_a_built_in_scalar_the_schema_does_not_define() {
        assert_eq!(
            focus_error(USER_SCHEMA, &["String"]),
            "`String` is a built-in scalar, which (stdin) does not define; `schema focus` takes types the schema defines"
        );
    }

    #[test]
    fn accepts_a_type_defined_only_by_an_extension() {
        let schema = indoc! {"
            type Query {
              user: User
            }

            extend type User {
              id: ID
            }
        "};

        // The output extends a type it does not define, exactly as the input
        // does, so it is not checked for self-containment.
        let result =
            focus::process(&["User"], || Ok(Input::inline(schema)), &mut Vec::new()).unwrap();
        let expected_schema = indoc! {"
            extend type User {
              id: ID
            }
        "};
        assert_eq!(result.trim(), expected_schema.trim());
    }

    #[test]
    fn uses_only_the_first_definition_of_a_repeated_type() {
        // The second `User` is dropped, so `Avatar`, which only it names, is
        // not a descendant, and it is not printed again beside the first.
        // Extensions still fold into the first.
        let schema = indoc! {"
            type Query {
              user: User
            }

            type User {
              id: ID
            }

            type User {
              avatar: Avatar
            }

            type Avatar {
              url: String
            }

            extend type User {
              name: String
            }
        "};

        let mut warnings = Vec::new();
        let result =
            focus::process(&["Query"], || Ok(Input::inline(schema)), &mut warnings).unwrap();
        util::assert_self_contained(&result);

        assert_eq!(
            result,
            indoc! {"
                type Query {
                  user: User
                }

                type User {
                  id: ID
                }

                extend type User {
                  name: String
                }
            "}
        );
        assert_eq!(
            warnings,
            ["type `User` is defined more than once in (stdin) (at 5:1, 9:1); only the first is used"]
        );
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
        let result =
            focus::process(&["Query"], || Ok(Input::inline(schema)), &mut Vec::new()).unwrap();
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
