use crate::{error::Error, input::Input, util};
use graphql_parser::schema::{Definition, Document};

pub fn process(schema: &Input) -> Result<String, Error> {
    // A blank schema has nothing to sort, so it passes through as empty.
    // Sorting only lays definitions out, so a name defined twice keeps both.
    let Some(schema_ast) = schema.parse_schema_as_written()? else {
        return Ok(String::new());
    };

    // Create a vector of indices paired with sort keys
    let mut indices_with_keys: Vec<(usize, (u8, String))> = schema_ast
        .definitions
        .iter()
        .enumerate()
        .map(|(i, def)| {
            let category = match def {
                Definition::SchemaDefinition(_) => 0,
                Definition::DirectiveDefinition(_) => 1,
                Definition::TypeDefinition(_) => 2,
                Definition::TypeExtension(_) => 3,
            };

            let name = match def {
                Definition::SchemaDefinition(_) => String::new(),
                Definition::DirectiveDefinition(dir) => dir.name.clone(),
                Definition::TypeDefinition(td) => util::schema_type_definition_name(td)
                    .cloned()
                    .unwrap_or_default(),
                Definition::TypeExtension(te) => util::type_extension_name(te).clone(),
            };

            (i, (category, name))
        })
        .collect();

    // Sort by the keys. The sort is stable, so definitions sharing a key, such
    // as several extensions of one type, keep their source order.
    indices_with_keys.sort_by(|(_, a), (_, b)| a.cmp(b));

    // Create sorted definitions using the sorted indices
    let sorted_definitions: Vec<_> = indices_with_keys
        .into_iter()
        .map(|(i, _)| schema_ast.definitions[i].clone())
        .collect();

    // Create a new document with sorted definitions
    let sorted_doc = Document {
        definitions: sorted_definitions,
    };

    Ok(format!("{sorted_doc}"))
}

#[cfg(test)]
mod tests {
    use crate::{input::Input, sort, util};
    use indoc::indoc;
    use pretty_assertions::assert_eq;

    fn sorted(schema: &str) -> String {
        sort::process(&Input::inline(schema)).unwrap()
    }

    #[test]
    fn test_sort_basic_types() {
        let schema = indoc! {"
            type User {
              id: ID!
              name: String
            }

            type Query {
              user: User
            }

            type Company {
              id: ID!
              name: String
            }
        "};

        let result = sorted(schema);
        let expected_schema = indoc! {"
            type Company {
              id: ID!
              name: String
            }

            type Query {
              user: User
            }

            type User {
              id: ID!
              name: String
            }
        "};

        assert_eq!(result.trim(), expected_schema.trim());
    }

    #[test]
    fn test_sort_mixed_definitions() {
        let schema = indoc! {"
            type User {
              id: ID!
              name: String
            }

            enum Status {
              ACTIVE
              INACTIVE
            }

            interface Node {
              id: ID!
            }

            type Query {
              user: User
            }

            scalar DateTime

            union SearchResult = User | Company

            type Company {
              id: ID!
              name: String
            }
        "};

        let result = sorted(schema);
        let expected_schema = indoc! {"
            type Company {
              id: ID!
              name: String
            }

            scalar DateTime

            interface Node {
              id: ID!
            }

            type Query {
              user: User
            }

            union SearchResult = User | Company

            enum Status {
              ACTIVE
              INACTIVE
            }

            type User {
              id: ID!
              name: String
            }
        "};

        assert_eq!(result.trim(), expected_schema.trim());
    }

    #[test]
    fn test_sort_with_schema_definition() {
        let schema = indoc! {"
            type User {
              id: ID!
              name: String
            }

            schema {
              query: Query
              mutation: Mutation
            }

            type Query {
              user: User
            }

            type Mutation {
              createUser(name: String!): User
            }
        "};

        let result = sorted(schema);
        let expected_schema = indoc! {"
            schema {
              query: Query
              mutation: Mutation
            }

            type Mutation {
              createUser(name: String!): User
            }

            type Query {
              user: User
            }

            type User {
              id: ID!
              name: String
            }
        "};

        assert_eq!(result.trim(), expected_schema.trim());
    }

    #[test]
    fn test_sort_with_directives() {
        let schema = indoc! {"
            type User {
              id: ID!
              name: String
            }

            directive @deprecated(reason: String) on FIELD_DEFINITION

            directive @auth(role: String!) on FIELD_DEFINITION

            type Query {
              user: User
            }
        "};

        let result = sorted(schema);
        let expected_schema = indoc! {"
            directive @auth(role: String!) on FIELD_DEFINITION

            directive @deprecated(reason: String) on FIELD_DEFINITION

            type Query {
              user: User
            }

            type User {
              id: ID!
              name: String
            }
        "};

        assert_eq!(result.trim(), expected_schema.trim());
    }

    #[test]
    fn test_sort_input_types() {
        let schema = indoc! {"
            type User {
              id: ID!
              name: String
            }

            input UserInput {
              name: String!
            }

            input CreateUserInput {
              user: UserInput!
            }

            type Query {
              user: User
            }
        "};

        let result = sorted(schema);
        let expected_schema = indoc! {"
            input CreateUserInput {
              user: UserInput!
            }

            type Query {
              user: User
            }

            type User {
              id: ID!
              name: String
            }

            input UserInput {
              name: String!
            }
        "};

        assert_eq!(result.trim(), expected_schema.trim());
    }

    #[test]
    fn test_sort_extensions_by_extended_type_name() {
        let schema = indoc! {"
            extend type Query {
              b: B
            }

            type Query {
              a: A
            }

            extend type B {
              x: Int
            }

            type B {
              id: ID
            }

            extend type A {
              y: Int
            }

            type A {
              id: ID
            }
        "};

        let result = sorted(schema);
        let expected_schema = indoc! {"
            type A {
              id: ID
            }

            type B {
              id: ID
            }

            type Query {
              a: A
            }

            extend type A {
              y: Int
            }

            extend type B {
              x: Int
            }

            extend type Query {
              b: B
            }
        "};

        assert_eq!(result.trim(), expected_schema.trim());
        util::assert_self_contained(&result);
    }

    #[test]
    fn test_sort_extensions_of_one_type_keep_source_order() {
        let schema = indoc! {"
            type Query {
              id: ID
            }

            type User {
              id: ID
            }

            extend type User {
              third: Int
            }

            extend type Query {
              second: Int
            }

            extend type User {
              first: Int
            }

            extend type Query {
              first: Int
            }

            extend type User {
              second: Int
            }
        "};

        let result = sorted(schema);
        let expected_schema = indoc! {"
            type Query {
              id: ID
            }

            type User {
              id: ID
            }

            extend type Query {
              second: Int
            }

            extend type Query {
              first: Int
            }

            extend type User {
              third: Int
            }

            extend type User {
              first: Int
            }

            extend type User {
              second: Int
            }
        "};

        assert_eq!(result.trim(), expected_schema.trim());
        util::assert_self_contained(&result);
    }

    #[test]
    fn test_sort_mixed_extension_kinds() {
        let schema = indoc! {"
            extend union SearchResult = Company

            extend scalar DateTime @tag

            extend input UserInput {
              email: String
            }

            extend enum Status {
              PENDING
            }

            extend interface Node {
              createdAt: DateTime
            }

            extend type Query {
              search: [SearchResult]
            }

            type User implements Node {
              id: ID!
              createdAt: DateTime
            }

            type Company {
              id: ID!
            }

            input UserInput {
              name: String
            }

            union SearchResult = User

            enum Status {
              ACTIVE
            }

            interface Node {
              id: ID!
            }

            scalar DateTime

            type Query {
              user(input: UserInput, status: Status): User
            }

            directive @tag on SCALAR
        "};

        let result = sorted(schema);
        let expected_schema = indoc! {"
            directive @tag on SCALAR

            type Company {
              id: ID!
            }

            scalar DateTime

            interface Node {
              id: ID!
            }

            type Query {
              user(input: UserInput, status: Status): User
            }

            union SearchResult = User

            enum Status {
              ACTIVE
            }

            type User implements Node {
              id: ID!
              createdAt: DateTime
            }

            input UserInput {
              name: String
            }

            extend scalar DateTime @tag

            extend interface Node {
              createdAt: DateTime
            }

            extend type Query {
              search: [SearchResult]
            }

            extend union SearchResult = Company

            extend enum Status {
              PENDING
            }

            extend input UserInput {
              email: String
            }
        "};

        assert_eq!(result.trim(), expected_schema.trim());
        util::assert_self_contained(&result);
    }

    #[test]
    fn test_sort_empty_schema() {
        // A blank schema has nothing to sort, so it passes through as empty.
        assert_eq!(sorted(""), "");
        assert_eq!(sorted("\n# only a comment\n"), "");
    }

    #[test]
    fn test_sort_single_type() {
        let schema = indoc! {"
            type User {
              id: ID!
              name: String
            }
        "};

        let result = sorted(schema);
        let expected_schema = indoc! {"
            type User {
              id: ID!
              name: String
            }
        "};

        assert_eq!(result.trim(), expected_schema.trim());
    }

    /// The schema definition comes before directives, and names compare as
    /// written, so uppercase sorts before lowercase.
    #[test]
    fn test_sort_compares_names_as_written() {
        let schema = indoc! {"
            directive @b on FIELD_DEFINITION

            directive @A on FIELD_DEFINITION

            schema {
              query: b
            }

            type b {
              id: ID
            }

            type Z {
              id: ID
            }

            type a {
              id: ID
            }
        "};

        let expected_schema = indoc! {"
            schema {
              query: b
            }

            directive @A on FIELD_DEFINITION

            directive @b on FIELD_DEFINITION

            type Z {
              id: ID
            }

            type a {
              id: ID
            }

            type b {
              id: ID
            }
        "};

        assert_eq!(sorted(schema).trim(), expected_schema.trim());
    }

    /// Members keep their source order, a name defined twice keeps both
    /// definitions, and descriptions stay while comments go.
    #[test]
    fn test_sort_keeps_members_and_repeats_as_written() {
        let schema = indoc! {"
            # Dropped.
            \"Kept.\"
            type Z {
              f(b: Int, a: Int): Int
            }

            input I {
              b: Int
              a: Int
            }

            enum S {
              B
              A
            }

            scalar D

            scalar D
        "};

        let expected_schema = indoc! {"
            scalar D

            scalar D

            input I {
              b: Int
              a: Int
            }

            enum S {
              B
              A
            }

            \"Kept.\"
            type Z {
              f(b: Int, a: Int): Int
            }
        "};

        assert_eq!(sorted(schema).trim(), expected_schema.trim());
    }
}
