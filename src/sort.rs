use crate::util;
use graphql_parser::parse_schema;
use graphql_parser::schema::{Definition, Document};

pub fn process(schema: &str) -> String {
    let schema_ast = parse_schema::<String>(schema).expect("Invalid schema");

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
                Definition::TypeExtension(_) => String::new(),
            };

            (i, (category, name))
        })
        .collect();

    // Sort by the keys
    indices_with_keys.sort_by_key(|(_, key)| key.clone());

    // Create sorted definitions using the sorted indices
    let sorted_definitions: Vec<_> = indices_with_keys
        .into_iter()
        .map(|(i, _)| schema_ast.definitions[i].clone())
        .collect();

    // Create a new document with sorted definitions
    let sorted_doc = Document {
        definitions: sorted_definitions,
    };

    format!("{sorted_doc}")
}

#[cfg(test)]
mod tests {
    use crate::sort;
    use indoc::indoc;
    use pretty_assertions::assert_eq;

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

        let result = sort::process(schema);
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

        let result = sort::process(schema);
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

        let result = sort::process(schema);
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

        let result = sort::process(schema);
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

        let result = sort::process(schema);
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
    fn test_sort_empty_schema() {
        // GraphQL parser doesn't accept completely empty schemas
        // Use a minimal valid schema instead
        let schema = "type Query { id: ID }";
        let result = sort::process(schema);
        let expected = "type Query {\n  id: ID\n}";
        assert_eq!(result.trim(), expected.trim());
    }

    #[test]
    fn test_sort_single_type() {
        let schema = indoc! {"
            type User {
              id: ID!
              name: String
            }
        "};

        let result = sort::process(schema);
        let expected_schema = indoc! {"
            type User {
              id: ID!
              name: String
            }
        "};

        assert_eq!(result.trim(), expected_schema.trim());
    }
}
